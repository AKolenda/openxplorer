// SPDX-License-Identifier: AGPL-3.0-only
//! Asking an SMB server for the shadow copies of an item, as Windows'
//! Previous Versions tab does.
//!
//! GIO, `GVfs` and libsmbclient cannot send `FSCTL_SRV_ENUMERATE_SNAPSHOTS`,
//! so [`SmbShadowCopies`] opens a connection of its own with the `smb`
//! client crate, only to send that request: negotiate, sign in, connect
//! the share, open the item for its attributes, ask, close. The versions
//! themselves are read through GIO.
//!
//! It signs in with the account the user already saved for the server:
//! `OpenXplorer`'s own, else the password `GVfs` remembered under GNOME's
//! `org.gnome.keyring.NetworkPassword` schema when it mounted the share.
//! Both are read from unlocked keyring items only, so opening the tab never
//! shows the keyring's unlock prompt, and it never asks for a password.
//! Safety rule (NET-014): an account is only sent to the host name and port
//! it was saved for, and a password the server refused is not sent again
//! this session, so looking again cannot lock the account out.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek};
use std::net::{IpAddr, ToSocketAddrs};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use smb::{
    Client, ClientConfig, ConnectionConfig, FileAccessMask, FileCreateArgs, FsctlCodes, FsctlRequest,
    FsctlResponseContent, IoctlReqData, SrvEnumerateSnapshotsRequest, UncPath,
};

use super::credential::{Credential, CredentialScope, Password};
use super::credential_store::{credential_attributes, locked, CredentialGeneration, CredentialStore};
use super::keyring::{KeyringError, SecretAttributes, SCHEMA_ATTRIBUTE};
use super::server::{ServerKey, DEFAULT_SMB_PORT};
use crate::location::{split_location, unquote_without_controls};
use crate::transfer::Cancellation;
use crate::versions::{ShadowCopies, ShadowCopyError, ShadowCopyLister};
use crate::LOG_DOMAIN;

/// How long the server may take to answer each request, the first reply
/// of the negotiation included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a lookup waits for the whole exchange. The `smb` client can
/// also wait without a timeout (for request credits, and when it logs off
/// as it is dropped), so it runs on a thread of its own that the lookup
/// stops waiting for.
const LOOKUP_DEADLINE: Duration = Duration::from_secs(30);

/// How often a waiting lookup checks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// The first reply size asked for: room for about 1,200 shadow copies,
/// which one credit covers.
const FIRST_REPLY_SIZE: u32 = 64 * 1024;

/// The largest reply asked for once a server said the first was too small.
const LARGE_REPLY_SIZE: u32 = 1024 * 1024;

/// The bytes one shadow-copy name takes in a reply: 24 UTF-16 units and
/// their terminator.
const TOKEN_BYTES: u32 = 50;

/// The size of the reply's counts: `NumberOfSnapShots`,
/// `NumberOfSnapShotsReturned` and `SnapShotArraySize`.
const REPLY_HEADER_SIZE: usize = 12;

/// The `protocol` attribute of the passwords `GVfs` remembers for SMB.
const GVFS_SMB_PROTOCOL: &str = "smb";

/// The schema of `OpenXplorer`'s own credentials, which are not `GVfs`'s.
const OWN_SCHEMA: &str = super::credential_store::SMB_CREDENTIAL_SCHEMA;

/// NT status codes the server answers with.
mod status {
    pub(super) const BUFFER_OVERFLOW: u32 = 0x8000_0005;
    pub(super) const BUFFER_TOO_SMALL: u32 = 0xC000_0023;
    pub(super) const INVALID_DEVICE_REQUEST: u32 = 0xC000_0010;
    pub(super) const NOT_SUPPORTED: u32 = 0xC000_00BB;
    pub(super) const INVALID_DEVICE_STATE: u32 = 0xC000_0184;
    pub(super) const WRONG_PASSWORD: u32 = 0xC000_006A;
    pub(super) const LOGON_FAILURE: u32 = 0xC000_006D;
    pub(super) const ACCOUNT_RESTRICTION: u32 = 0xC000_006E;
    pub(super) const PASSWORD_EXPIRED: u32 = 0xC000_0071;
    pub(super) const ACCOUNT_DISABLED: u32 = 0xC000_0072;
    pub(super) const PASSWORD_MUST_CHANGE: u32 = 0xC000_0224;
    pub(super) const ACCOUNT_LOCKED_OUT: u32 = 0xC000_0234;
}

/// The [`ShadowCopyLister`] of the app: the `smb` client, signed in with
/// the account saved for the server.
pub struct SmbShadowCopies {
    store: Arc<CredentialStore>,
    /// Accounts a server refused this session; they are not tried again.
    refused: Arc<Mutex<HashSet<AccountKey>>>,
    /// One sign-in at a time per account, so lookups in two windows
    /// cannot both send a password the server is about to refuse.
    attempts: Mutex<HashMap<AccountKey, Arc<Mutex<()>>>>,
}

/// An account on a server: the server, the user and a hash of the
/// password, kept in memory only.
type AccountKey = (ServerKey, String, u64);

impl std::fmt::Debug for SmbShadowCopies {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SmbShadowCopies").finish_non_exhaustive()
    }
}

impl SmbShadowCopies {
    /// A lister signing in with the accounts saved in `store`'s keyring.
    pub fn new(store: Arc<CredentialStore>) -> Self {
        Self {
            store,
            refused: Arc::default(),
            attempts: Mutex::default(),
        }
    }

    /// What identifies `account` on `key` among the refused ones.
    fn refusal_key(key: &ServerKey, account: &Account) -> AccountKey {
        let mut hasher = DefaultHasher::new();
        account.password.as_str().hash(&mut hasher);
        (key.clone(), account.user.clone(), hasher.finish())
    }
}

impl ShadowCopyLister for SmbShadowCopies {
    fn list(&self, uri: &str, cancel: &Cancellation) -> Result<ShadowCopies, ShadowCopyError> {
        if cancel.is_cancelled() {
            return Err(ShadowCopyError::Cancelled);
        }
        let target = SmbTarget::of(uri).ok_or_else(|| failed(format!("not a share location: {uri}")))?;
        let (account, generation) = saved_account(&self.store, &target)
            .map_err(ShadowCopyError::Keyring)?
            .ok_or(ShadowCopyError::NoCredential)?;
        let refusal = Self::refusal_key(&target.key, &account);
        if locked(&self.refused).contains(&refusal) {
            return Err(ShadowCopyError::SignInRefused);
        }
        check_cancelled(cancel)?;
        let deadline = Instant::now() + LOOKUP_DEADLINE;
        let gate = Arc::clone(locked(&self.attempts).entry(refusal.clone()).or_default());
        let refused = Arc::clone(&self.refused);
        let store = Arc::clone(&self.store);
        let (sender, receiver) = mpsc::channel();
        let exchange_cancel = cancel.clone();
        let exchange = move || {
            // The name is looked up before the account's turn is taken, so
            // a resolver that never answers holds up no other lookup.
            let server = match resolve(target.key.host(), target.key.port()) {
                Ok(server) => server,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            let Some(guard) = take_turn(&gate, deadline) else {
                let _ = sender.send(Err(failed("another lookup is still signing in".to_owned())));
                return;
            };
            let mut turn = SignInTurn {
                guard: Some(guard),
                refused,
                refusal,
                store,
                key: target.key.clone(),
                generation,
                deadline,
            };
            // The answer is sent before the client is dropped: dropping it
            // logs off over the network and can wait without a timeout. The
            // SMB client is young; a panic in it must not take the window
            // down.
            let answer = sender.clone();
            let report = move |outcome: &Result<ShadowCopies, ShadowCopyError>| {
                let _ = answer.send(outcome.clone());
            };
            let finished = catch_unwind(AssertUnwindSafe(|| {
                enumerate(&target, &server, &account, &mut turn, &exchange_cancel, report);
            }));
            if finished.is_err() {
                let _ = sender.send(Err(failed("the SMB client panicked".to_owned())));
            }
        };
        std::thread::Builder::new()
            .name("ox-shadow-copies".to_owned())
            .spawn(exchange)
            .map_err(|error| failed(format!("no thread for the SMB client: {error}")))?;
        wait_for(&receiver, cancel, deadline)
    }
}

/// How long a lookup waiting for the account's turn sleeps between tries.
const TURN_POLL: Duration = Duration::from_millis(50);

/// The account's turn to sign in, once `gate` is free, or `None` when
/// `deadline` passes first, so lookups never pile up behind a stalled one.
fn take_turn(gate: &Mutex<()>, deadline: Instant) -> Option<MutexGuard<'_, ()>> {
    loop {
        match gate.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() >= deadline => return None,
            Err(TryLockError::WouldBlock) => std::thread::sleep(TURN_POLL),
        }
    }
}

/// An account's turn to sign in: held only until the server accepted or
/// refused the password, and checked just before the password is sent.
struct SignInTurn<'gate> {
    guard: Option<MutexGuard<'gate, ()>>,
    refused: Arc<Mutex<HashSet<AccountKey>>>,
    refusal: AccountKey,
    store: Arc<CredentialStore>,
    key: ServerKey,
    /// The server's sign-out generation when the password was read.
    generation: CredentialGeneration,
    /// When the lookup stops waiting for this exchange.
    deadline: Instant,
}

impl SignInTurn<'_> {
    /// Refuses to send the password once the lookup gave up on this
    /// exchange, the server was signed out since the password was read
    /// (NET-021), or the server refused it meanwhile.
    fn check(&self) -> Result<(), ShadowCopyError> {
        if Instant::now() >= self.deadline {
            return Err(failed("the lookup stopped waiting before sign-in".to_owned()));
        }
        if self.store.generation_of(&self.key) != self.generation {
            return Err(ShadowCopyError::NoCredential);
        }
        if locked(&self.refused).contains(&self.refusal) {
            return Err(ShadowCopyError::SignInRefused);
        }
        Ok(())
    }

    /// Records how the sign-in went and gives the turn back.
    fn finish(&mut self, outcome: &Result<(), ShadowCopyError>) {
        if matches!(outcome, Err(ShadowCopyError::SignInRefused)) {
            locked(&self.refused).insert(self.refusal.clone());
        }
        self.guard = None;
    }
}

/// The answer of the exchange on `receiver`, unless `cancel` is cancelled
/// or [`LOOKUP_DEADLINE`] passes first; the exchange is then left to end
/// on its own thread.
fn wait_for(
    receiver: &mpsc::Receiver<Result<ShadowCopies, ShadowCopyError>>,
    cancel: &Cancellation,
    deadline: Instant,
) -> Result<ShadowCopies, ShadowCopyError> {
    loop {
        check_cancelled(cancel)?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(failed("the server did not answer in time".to_owned()));
        }
        match receiver.recv_timeout(left.min(CANCEL_POLL)) {
            Ok(outcome) => return outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(failed("the SMB client stopped without an answer".to_owned()));
            }
        }
    }
}

/// Where an item is on its server.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SmbTarget {
    key: ServerKey,
    share: String,
    /// The item's path inside the share, decoded, with `\` separators;
    /// empty for the share itself.
    path: String,
}

impl SmbTarget {
    /// The server, share and path of the `smb://` location `uri`; `None`
    /// for another location, a whole server or an undecodable path.
    fn of(uri: &str) -> Option<Self> {
        let key = ServerKey::for_location(uri)?;
        // Safety rule (SAFE-010): a location never holds a user name, so
        // the account comes from the keyring only.
        let parts = split_location(uri).ok()?;
        let components = parts
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(unquote_without_controls)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        // A decoded separator would name another item than the location.
        if components.iter().any(|part| part.contains(['/', '\\'])) {
            return None;
        }
        let (share, path) = components.split_first()?;
        Some(Self {
            key,
            share: share.clone(),
            path: path.join("\\"),
        })
    }
}

/// An account to sign in with.
#[derive(Debug)]
struct Account {
    /// `DOMAIN\user`, or `user` for the server's default domain.
    user: String,
    password: Password,
}

/// The account saved for the server of `target` in unlocked keyring items:
/// `OpenXplorer`'s, else the one `GVfs` remembered; `None` when neither
/// saved one, or the server was signed out meanwhile (NET-021).
/// The server's sign-out generation when it was read comes with it.
fn saved_account(
    store: &CredentialStore,
    target: &SmbTarget,
) -> Result<Option<(Account, CredentialGeneration)>, KeyringError> {
    let Some(keyring) = store.keyring() else {
        return Ok(None);
    };
    let key = &target.key;
    let generation = store.generation_of(key);
    for scope in [CredentialScope::Session, CredentialScope::Permanent] {
        for (_, text) in keyring.find_unlocked(&credential_attributes(key, Some(scope)), &|_| true)? {
            if let Some(credential) = Credential::from_keyring_text(&text, scope) {
                let account = Account {
                    user: qualified_user(&credential.domain, &credential.username),
                    password: credential.password,
                };
                return Ok((store.generation_of(key) == generation).then_some((account, generation)));
            }
        }
    }
    // GVfs saves the host as the user typed it, so it is compared without
    // regard to case here, as `ServerKey` compares hosts; the secret of an
    // item for another server or port is never read.
    let query = SecretAttributes::any_schema().with("protocol", GVFS_SMB_PROTOCOL);
    let items = keyring.find_unlocked(&query, &|attributes| gvfs_item_fits(attributes, target))?;
    let account = gvfs_account(&items, target);
    Ok(account
        .filter(|_| store.generation_of(key) == generation)
        .map(|account| (account, generation)))
}

/// True when the `GVfs` password with `attributes` was saved for the host,
/// port and share (or whole server) of `target`, with a user.
fn gvfs_item_fits(attributes: &SecretAttributes, target: &SmbTarget) -> bool {
    // GVfs leaves `port` out for the default port.
    let same_port = match attributes.get("port") {
        Some(saved) => saved == target.key.port().to_string(),
        None => target.key.port() == DEFAULT_SMB_PORT,
    };
    let same_host = attributes
        .get("server")
        .is_some_and(|server| server.eq_ignore_ascii_case(target.key.host()));
    // A password saved for another share of the server is not this one's.
    let same_share = attributes
        .get("object")
        .is_none_or(|object| object.is_empty() || object.eq_ignore_ascii_case(&target.share));
    same_host
        && same_port
        && same_share
        && attributes.get(SCHEMA_ATTRIBUTE) != Some(OWN_SCHEMA)
        && attributes.get("user").is_some_and(|user| !user.is_empty())
}

/// The `GVfs` password among `items` that fits `target`
/// ([`gvfs_item_fits`]), for its share or else for the whole server.
fn gvfs_account(items: &[(SecretAttributes, String)], target: &SmbTarget) -> Option<Account> {
    let for_share = |attributes: &SecretAttributes| {
        attributes
            .get("object")
            .is_some_and(|object| object.eq_ignore_ascii_case(&target.share))
    };
    let candidates = items
        .iter()
        .filter(|(attributes, _)| gvfs_item_fits(attributes, target));
    let (attributes, password) = candidates
        .clone()
        .find(|(attributes, _)| for_share(attributes))
        .or_else(|| {
            candidates
                .clone()
                .find(|(attributes, _)| attributes.get("object").is_none_or(str::is_empty))
        })?;
    Some(Account {
        user: qualified_user(
            attributes.get("domain").unwrap_or_default(),
            attributes.get("user").unwrap_or_default(),
        ),
        password: Password::from(password.as_str()),
    })
}

/// `domain\user`, or `user` without a domain.
fn qualified_user(domain: &str, user: &str) -> String {
    if domain.is_empty() {
        user.to_owned()
    } else {
        format!("{domain}\\{user}")
    }
}

/// Signs in to the share of `target` at the address `server` and asks for
/// the shadow copies of its item, giving the answer to `report` before the
/// client is dropped. After a failure the connection is stopped first, so
/// the client's log-off on drop does not wait for the server again; it
/// can still wait for request credits, which is why this runs on a thread
/// of its own.
fn enumerate(
    target: &SmbTarget,
    server: &str,
    account: &Account,
    turn: &mut SignInTurn<'_>,
    cancel: &Cancellation,
    report: impl FnOnce(&Result<ShadowCopies, ShadowCopyError>),
) {
    let config = ClientConfig {
        // A DFS referral would send the account to another server (NET-014).
        dfs: false,
        connection: ConnectionConfig {
            port: Some(target.key.port()),
            timeout: Some(REQUEST_TIMEOUT),
            // Only an SMB2 negotiation sets the timeout before the
            // server's first reply is read.
            smb2_only_negotiate: true,
            ..ConnectionConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = Client::new(config);
    let outcome = enumerate_with(&client, server, target, account, turn, cancel);
    // A turn not given back at sign-in (a failure before it) is given back
    // now, before anything that could wait.
    turn.guard = None;
    if outcome.is_err() {
        if let Ok(connection) = client.get_connection(server) {
            // Stopping only drops the socket; it cannot change the answer.
            let _ = connection.close();
        }
    }
    report(&outcome);
    if outcome.is_ok() {
        // Closing only ends the session; it cannot change the answer.
        let _ = client.close();
    }
}

fn enumerate_with(
    client: &Client,
    server: &str,
    target: &SmbTarget,
    account: &Account,
    turn: &mut SignInTurn<'_>,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    check_cancelled(cancel)?;
    let share = UncPath::new(server)
        .and_then(|server| server.with_share(&target.share))
        .map_err(classify)?;
    turn.check()?;
    let signed_in = client
        .share_connect(&share, &account.user, account.password.as_str().to_owned())
        .map_err(classify_sign_in);
    turn.finish(&signed_in);
    signed_in?;
    check_cancelled(cancel)?;
    let item = share.with_path(&target.path);
    let access = FileAccessMask::new()
        .with_file_read_attributes(true)
        .with_synchronize(true);
    let resource = client
        .create_file(&item, &FileCreateArgs::make_open_existing(access))
        .map_err(classify)?;
    let handle: &smb::resource::ResourceHandle = match &resource {
        smb::Resource::File(file) => file,
        smb::Resource::Directory(directory) => directory,
        smb::Resource::Pipe(pipe) => pipe,
    };
    check_cancelled(cancel)?;
    // The item is not closed here: a server slow to answer that would
    // delay an answer already read. Logging off after the answer is sent
    // closes it.
    ask_for_snapshots(handle, cancel)
}

/// The address of `host`; an IP address is returned as it is. A resolver
/// that does not answer leaves only the exchange's own thread waiting: the
/// lookup stops waiting after [`LOOKUP_DEADLINE`].
fn resolve(host: &str, port: u16) -> Result<String, ShadowCopyError> {
    if host.parse::<IpAddr>().is_ok() {
        return Ok(host.to_owned());
    }
    match (host, port)
        .to_socket_addrs()
        .map(|mut addresses| addresses.next())
    {
        Ok(Some(address)) => Ok(address.ip().to_string()),
        Ok(None) => Err(failed(format!("{host} has no address"))),
        Err(error) => Err(failed(format!("{host}: {error}"))),
    }
}

/// Sends `FSCTL_SRV_ENUMERATE_SNAPSHOTS` on the open item and reads the
/// names, asking once more, for as much as the list needs, when the first
/// reply was too small for all of them.
fn ask_for_snapshots(
    handle: &smb::resource::ResourceHandle,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    let first = match handle.fsctl_with_options(EnumerateSnapshots, FIRST_REPLY_SIZE) {
        Ok(reply) => SnapshotReply::read(&reply.0)?,
        // Too small, and the server says no more: only the largest reply
        // can hold the list.
        Err(error)
            if matches!(
                status_of(&error),
                Some(status::BUFFER_OVERFLOW | status::BUFFER_TOO_SMALL)
            ) =>
        {
            check_cancelled(cancel)?;
            let reply = handle
                .fsctl_with_options(EnumerateSnapshots, LARGE_REPLY_SIZE)
                .map_err(classify)?;
            return Ok(SnapshotReply::read(&reply.0)?.into_copies());
        }
        Err(error) => return Err(classify(error)),
    };
    if !first.is_cut_short() {
        return Ok(first.into_copies());
    }
    // A reply too small for the whole list names some or none of the
    // shadow copies; ask once more for as much as the list needs (each name
    // takes 50 bytes), which costs fewer credits than the largest reply.
    check_cancelled(cancel)?;
    let needed = u32::try_from(REPLY_HEADER_SIZE)
        .unwrap_or(u32::MAX)
        .saturating_add(first.total.saturating_mul(TOKEN_BYTES))
        .saturating_add(2);
    let size = needed.clamp(FIRST_REPLY_SIZE, LARGE_REPLY_SIZE);
    match handle.fsctl_with_options(EnumerateSnapshots, size) {
        Ok(reply) => Ok(SnapshotReply::read(&reply.0)?.into_copies()),
        // The names already read are kept, marked as not all of them.
        Err(error) => {
            glib::g_debug!(LOG_DOMAIN, "Asking again for shadow copies failed: {error}");
            Ok(first.into_copies())
        }
    }
}

/// `FSCTL_SRV_ENUMERATE_SNAPSHOTS` with a reply read as raw bytes: the
/// `smb` crate's own reply type cannot read an empty list, and unwraps
/// what it cannot read.
#[derive(Debug, Clone, Copy)]
struct EnumerateSnapshots;

impl binrw::BinWrite for EnumerateSnapshots {
    type Args<'a> = ();

    fn write_options<W: std::io::Write + Seek>(
        &self,
        _: &mut W,
        _: binrw::Endian,
        (): Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        Ok(())
    }
}

impl From<EnumerateSnapshots> for IoctlReqData {
    fn from(_: EnumerateSnapshots) -> Self {
        SrvEnumerateSnapshotsRequest(()).into()
    }
}

impl FsctlRequest for EnumerateSnapshots {
    type Response = RawReply;
    const FSCTL_CODE: FsctlCodes = FsctlCodes::SrvEnumerateSnapshots;
}

/// Every byte of an FSCTL reply; reading it never fails.
#[derive(Debug)]
struct RawReply(Vec<u8>);

impl binrw::BinRead for RawReply {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        _: binrw::Endian,
        (): Self::Args<'_>,
    ) -> binrw::BinResult<Self> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(Self(bytes))
    }
}

impl FsctlResponseContent for RawReply {
    const FSCTL_CODES: &'static [FsctlCodes] = &[FsctlCodes::SrvEnumerateSnapshots];
}

/// A `SRV_SNAPSHOT_ARRAY` (MS-SMB2 2.2.32.2).
#[derive(Debug, PartialEq, Eq)]
struct SnapshotReply {
    total: u32,
    returned: u32,
    names: Vec<String>,
}

impl SnapshotReply {
    /// [`parse`](Self::parse), or why the reply cannot be read.
    fn read(bytes: &[u8]) -> Result<Self, ShadowCopyError> {
        Self::parse(bytes).ok_or_else(|| failed("a malformed snapshot list".to_owned()))
    }

    /// The names, newest first, and whether the server keeps more.
    fn into_copies(self) -> ShadowCopies {
        ShadowCopies::from_names(self.names.iter().map(String::as_str), self.is_cut_short())
    }

    /// The reply in `bytes`, or `None` when it is shorter than its counts
    /// or holds text that is not UTF-16.
    fn parse(bytes: &[u8]) -> Option<Self> {
        let count = |index: usize| {
            let start = index * 4;
            bytes
                .get(start..start + 4)
                .and_then(|slice| slice.try_into().ok())
                .map(u32::from_le_bytes)
        };
        let (total, returned, array_size) = (count(0)?, count(1)?, count(2)?);
        let array = bytes.get(REPLY_HEADER_SIZE..).unwrap_or_default();
        let array = &array[..array.len().min(usize::try_from(array_size).ok()?)];
        let units: Vec<u16> = array
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let mut names = Vec::new();
        for name in units.split(|unit| *unit == 0) {
            if name.is_empty() {
                break;
            }
            names.push(String::from_utf16(name).ok()?);
        }
        Some(Self {
            total,
            returned,
            names,
        })
    }

    /// True when the server keeps more shadow copies than it named.
    fn is_cut_short(&self) -> bool {
        self.returned < self.total
            || usize::try_from(self.returned).is_ok_and(|returned| returned > self.names.len())
    }
}

/// [`ShadowCopyError::Cancelled`] once `cancel` is cancelled.
fn check_cancelled(cancel: &Cancellation) -> Result<(), ShadowCopyError> {
    if cancel.is_cancelled() {
        Err(ShadowCopyError::Cancelled)
    } else {
        Ok(())
    }
}

/// The NT status the server answered with, if the error is one.
fn status_of(error: &smb::Error) -> Option<u32> {
    match error {
        smb::Error::ReceivedErrorMessage(status, _) | smb::Error::UnexpectedMessageStatus(status) => {
            Some(*status)
        }
        _ => None,
    }
}

/// The error in the app's words.
#[allow(clippy::needless_pass_by_value)]
fn classify(error: smb::Error) -> ShadowCopyError {
    match status_of(&error) {
        Some(
            status::WRONG_PASSWORD
            | status::LOGON_FAILURE
            | status::ACCOUNT_RESTRICTION
            | status::PASSWORD_EXPIRED
            | status::ACCOUNT_DISABLED
            | status::PASSWORD_MUST_CHANGE
            | status::ACCOUNT_LOCKED_OUT,
        ) => ShadowCopyError::SignInRefused,
        Some(status::INVALID_DEVICE_REQUEST | status::NOT_SUPPORTED | status::INVALID_DEVICE_STATE) => {
            ShadowCopyError::Unsupported
        }
        _ => failed(error.to_string()),
    }
}

/// An error while signing in, in the app's words: a server refusing the
/// sign-in method is not one without previous versions.
fn classify_sign_in(error: smb::Error) -> ShadowCopyError {
    match classify(error) {
        ShadowCopyError::Unsupported => failed("the server refused the sign-in method".to_owned()),
        other => other,
    }
}

/// An error the app has no words of its own for. Its detail is logged for
/// developers; it holds no password (SAFE-011).
fn failed(detail: String) -> ShadowCopyError {
    glib::g_debug!(LOG_DOMAIN, "Listing shadow copies failed: {detail}");
    ShadowCopyError::Failed { detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::test_support::MemoryKeyring;
    use crate::network::{Keyring, KeyringCollection, NewSecret, SessionCredentials};

    fn gvfs_password(keyring: &MemoryKeyring, attributes: &[(&str, &str)], password: &str) {
        let mut stored = SecretAttributes::for_schema("org.gnome.keyring.NetworkPassword");
        for (name, value) in attributes {
            stored = stored.with(name, value);
        }
        keyring
            .store(&NewSecret {
                collection: KeyringCollection::Default,
                label: "GVfs",
                attributes: &stored,
                text: password,
            })
            .unwrap();
    }

    fn account_for(store: &CredentialStore, uri: &str) -> Option<(String, String)> {
        let target = SmbTarget::of(uri).unwrap();
        saved_account(store, &target)
            .unwrap()
            .map(|(account, _)| (account.user, account.password.as_str().to_owned()))
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn reply(total: u32, returned: u32, array: &[u8]) -> Vec<u8> {
        let size = u32::try_from(array.len()).unwrap();
        [total, returned, size]
            .iter()
            .flat_map(|count| count.to_le_bytes())
            .chain(array.iter().copied())
            .collect()
    }

    #[test]
    fn a_location_names_its_share_and_path_on_the_server() {
        let target = SmbTarget::of("smb://nas:1445/team%20files/work/a%20b.txt").unwrap();

        assert_eq!(target.key.host(), "nas");
        assert_eq!(target.key.port(), 1445);
        assert_eq!(target.share, "team files");
        assert_eq!(target.path, "work\\a b.txt");
        assert_eq!(SmbTarget::of("smb://nas/share").unwrap().path, "");
        assert_eq!(SmbTarget::of("smb://nas/"), None);
        assert_eq!(SmbTarget::of("smb://nas/share/a%2Fb"), None);
        assert_eq!(SmbTarget::of("smb://nas/share/a%5Cb"), None);
        assert_eq!(SmbTarget::of("file:///srv/share"), None);
    }

    #[test]
    fn the_servers_answers_are_told_apart() {
        let refused = [
            status::WRONG_PASSWORD,
            status::LOGON_FAILURE,
            status::PASSWORD_MUST_CHANGE,
            status::ACCOUNT_LOCKED_OUT,
        ];
        for code in refused {
            let error = classify(smb::Error::UnexpectedMessageStatus(code));
            assert!(
                matches!(error, ShadowCopyError::SignInRefused),
                "{code:#x}: {error:?}"
            );
        }
        let unsupported = smb::Error::UnexpectedMessageStatus(status::NOT_SUPPORTED);
        assert!(matches!(classify(unsupported), ShadowCopyError::Unsupported));
        let at_sign_in = smb::Error::UnexpectedMessageStatus(status::NOT_SUPPORTED);
        assert!(matches!(
            classify_sign_in(at_sign_in),
            ShadowCopyError::Failed { .. }
        ));
    }

    /// parity: PROP-032
    #[test]
    fn a_snapshot_list_is_read_whatever_its_size() {
        let names = utf16("@GMT-2026.11.21-00.30.00\0@GMT-2026.11.20-08.00.00\0\0");
        let full = SnapshotReply::parse(&reply(2, 2, &names)).unwrap();
        let empty = SnapshotReply::parse(&reply(0, 0, &[])).unwrap();
        let cut_short = SnapshotReply::parse(&[2, 0, 0, 0, 0, 0, 0, 0, 104, 0, 0, 0]).unwrap();

        assert_eq!(
            full.names,
            ["@GMT-2026.11.21-00.30.00", "@GMT-2026.11.20-08.00.00"]
        );
        assert!(!full.is_cut_short());
        assert!(empty.names.is_empty() && !empty.is_cut_short());
        assert!(cut_short.names.is_empty() && cut_short.is_cut_short());
        assert_eq!(SnapshotReply::parse(&[0, 0, 0]), None);
    }

    /// parity: PROP-032, NET-014
    #[test]
    fn the_password_gvfs_saved_for_the_server_and_port_signs_in_when_openxplorer_has_none() {
        let keyring = Arc::new(MemoryKeyring::default());
        let store = CredentialStore::new(keyring.clone());
        gvfs_password(
            &keyring,
            &[
                ("server", "nas"),
                ("protocol", "smb"),
                ("user", "dan"),
                ("port", "1445"),
            ],
            "other-port-secret",
        );
        gvfs_password(
            &keyring,
            &[
                ("server", "nas"),
                ("protocol", "smb"),
                ("user", "eve"),
                ("object", "photos"),
            ],
            "other-share-secret",
        );
        gvfs_password(
            &keyring,
            &[
                ("server", "NAS"),
                ("protocol", "smb"),
                ("user", "ann"),
                ("domain", "WORKGROUP"),
            ],
            "gvfs-secret",
        );
        gvfs_password(
            &keyring,
            &[("server", "other"), ("protocol", "smb"), ("user", "bob")],
            "other-secret",
        );

        assert_eq!(
            account_for(&store, "smb://nas/share/a.txt"),
            Some(("WORKGROUP\\ann".to_owned(), "gvfs-secret".to_owned()))
        );
        assert_eq!(
            account_for(&store, "smb://nas/photos/a.jpg"),
            Some(("eve".to_owned(), "other-share-secret".to_owned()))
        );
        assert_eq!(
            account_for(&store, "smb://nas:1445/share/a.txt"),
            Some(("dan".to_owned(), "other-port-secret".to_owned()))
        );
        assert_eq!(account_for(&store, "smb://nas:2445/share/a.txt"), None);
        assert_eq!(account_for(&store, "smb://elsewhere/share"), None);
    }

    /// parity: PROP-032
    #[test]
    fn openxplorers_own_account_comes_before_the_gvfs_password() {
        let keyring = Arc::new(MemoryKeyring::default());
        let store = Arc::new(CredentialStore::new(keyring.clone()));
        gvfs_password(
            &keyring,
            &[("server", "nas"), ("protocol", "smb"), ("user", "ann")],
            "gvfs-secret",
        );
        let credentials = SessionCredentials::new(Arc::clone(&store));
        let generation = credentials.generation("smb://nas/share");
        credentials
            .persist(
                "smb://nas/share",
                &Credential {
                    username: "carol".to_owned(),
                    domain: String::new(),
                    password: Password::from("own-secret"),
                    scope: CredentialScope::Permanent,
                },
                generation,
            )
            .unwrap();

        assert_eq!(
            account_for(&store, "smb://nas/share/a.txt"),
            Some(("carol".to_owned(), "own-secret".to_owned()))
        );
    }

    /// A password the server refused is not sent again this session.
    ///
    /// parity: PROP-032
    #[test]
    fn a_refused_password_is_not_tried_again() {
        let keyring = Arc::new(MemoryKeyring::default());
        let lister = SmbShadowCopies::new(Arc::new(CredentialStore::new(keyring.clone())));
        gvfs_password(
            &keyring,
            &[("server", "nas"), ("protocol", "smb"), ("user", "ann")],
            "stale-secret",
        );
        let target = SmbTarget::of("smb://nas/share/a.txt").unwrap();
        let (account, _) = saved_account(&lister.store, &target).unwrap().unwrap();
        locked(&lister.refused).insert(SmbShadowCopies::refusal_key(&target.key, &account));

        let outcome = lister.list("smb://nas/share/a.txt", &Cancellation::new());

        assert!(
            matches!(outcome, Err(ShadowCopyError::SignInRefused)),
            "{outcome:?}"
        );
    }
}
