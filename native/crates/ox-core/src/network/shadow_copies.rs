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
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smb::{
    Client, ClientConfig, ConnectionConfig, FileAccessMask, FileCreateArgs, FsctlCodes, FsctlRequest,
    FsctlResponseContent, IoctlReqData, SrvEnumerateSnapshotsRequest, UncPath,
};

use super::credential::{Credential, CredentialScope, Password};
use super::credential_store::{credential_attributes, locked, CredentialStore};
use super::keyring::{KeyringError, SecretAttributes, SCHEMA_ATTRIBUTE};
use super::server::{ServerKey, DEFAULT_SMB_PORT};
use crate::location::{split_location, unquote_without_controls};
use crate::transfer::Cancellation;
use crate::versions::{ShadowCopies, ShadowCopyError, ShadowCopyLister};
use crate::LOG_DOMAIN;

/// How long the server may take to answer each request, the first reply
/// of the negotiation included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The first reply size asked for: room for about 1,200 shadow copies,
/// which one credit covers.
const FIRST_REPLY_SIZE: u32 = 64 * 1024;

/// The largest reply asked for once a server said the first was too small.
const LARGE_REPLY_SIZE: u32 = 1024 * 1024;

/// The size of the reply's counts: `NumberOfSnapShots`,
/// `NumberOfSnapShotsReturned` and `SnapShotArraySize`.
const REPLY_HEADER_SIZE: usize = 12;

/// [`REPLY_HEADER_SIZE`] as a reply size.
const REPLY_HEADER_BYTES: u32 = 12;

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
}

/// The [`ShadowCopyLister`] of the app: the `smb` client, signed in with
/// the account saved for the server.
pub struct SmbShadowCopies {
    store: Arc<CredentialStore>,
    /// Accounts a server refused this session, by server, user and a hash
    /// of the password; they are not tried again.
    refused: Mutex<HashSet<(ServerKey, String, u64)>>,
}

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
            refused: Mutex::default(),
        }
    }

    /// What identifies `account` on `key` among the refused ones.
    fn refusal_key(key: &ServerKey, account: &Account) -> (ServerKey, String, u64) {
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
        let account = saved_account(&self.store, &target)
            .map_err(|error| failed(format!("keyring: {error}")))?
            .ok_or(ShadowCopyError::NoCredential)?;
        let refusal = Self::refusal_key(&target.key, &account);
        if locked(&self.refused).contains(&refusal) {
            return Err(ShadowCopyError::SignInRefused);
        }
        if cancel.is_cancelled() {
            return Err(ShadowCopyError::Cancelled);
        }
        // The SMB client is young; a panic in it must not take the window
        // down with the lookup's worker thread.
        let outcome = catch_unwind(AssertUnwindSafe(|| enumerate(&target, &account, cancel)))
            .unwrap_or_else(|_| Err(failed("the SMB client panicked".to_owned())));
        if matches!(outcome, Err(ShadowCopyError::SignInRefused)) {
            locked(&self.refused).insert(refusal);
        }
        outcome
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
fn saved_account(store: &CredentialStore, target: &SmbTarget) -> Result<Option<Account>, KeyringError> {
    let Some(keyring) = store.keyring() else {
        return Ok(None);
    };
    let key = &target.key;
    let generation = store.generation_of(key);
    for scope in [CredentialScope::Session, CredentialScope::Permanent] {
        for (_, text) in keyring.find_unlocked(&credential_attributes(key, Some(scope)))? {
            if let Some(credential) = Credential::from_keyring_text(&text, scope) {
                return Ok((store.generation_of(key) == generation).then(|| Account {
                    user: qualified_user(&credential.domain, &credential.username),
                    password: credential.password,
                }));
            }
        }
    }
    let query = SecretAttributes::any_schema()
        .with("server", key.host())
        .with("protocol", GVFS_SMB_PROTOCOL);
    let items = keyring.find_unlocked(&query)?;
    let account = gvfs_account(&items, target);
    Ok(account.filter(|_| store.generation_of(key) == generation))
}

/// The `GVfs` password among `items` that fits `target`: saved for its
/// port, with a user, and for its share or for the whole server, in that
/// order of preference.
fn gvfs_account(items: &[(SecretAttributes, String)], target: &SmbTarget) -> Option<Account> {
    let port = target.key.port().to_string();
    let fits = |attributes: &SecretAttributes| {
        // GVfs leaves `port` out for the default port.
        let same_port = match attributes.get("port") {
            Some(saved) => saved == port,
            None => target.key.port() == DEFAULT_SMB_PORT,
        };
        same_port
            && attributes.get(SCHEMA_ATTRIBUTE) != Some(OWN_SCHEMA)
            && attributes.get("user").is_some_and(|user| !user.is_empty())
    };
    let for_share = |attributes: &SecretAttributes| {
        attributes
            .get("object")
            .is_some_and(|object| object.eq_ignore_ascii_case(&target.share))
    };
    let candidates = items.iter().filter(|(attributes, _)| fits(attributes));
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

/// Signs in to the share of `target` and asks for the shadow copies of its
/// item. A connection that failed or was cancelled is dropped without the
/// polite close, which could wait for an unresponsive server again.
fn enumerate(
    target: &SmbTarget,
    account: &Account,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
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
    let outcome = enumerate_with(&client, target, account, cancel);
    if outcome.is_ok() {
        // Closing only ends the session; it cannot change the answer.
        let _ = client.close();
    }
    outcome
}

fn enumerate_with(
    client: &Client,
    target: &SmbTarget,
    account: &Account,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    let share = UncPath::new(target.key.host())
        .and_then(|server| server.with_share(&target.share))
        .map_err(classify)?;
    client
        .share_connect(&share, &account.user, account.password.as_str().to_owned())
        .map_err(classify)?;
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
    let copies = ask_for_snapshots(handle, cancel)?;
    // Closing the item only releases it; it cannot change the answer.
    let _ = handle.close();
    Ok(copies)
}

/// Sends `FSCTL_SRV_ENUMERATE_SNAPSHOTS` on the open item and reads the
/// names, asking once more with a reply as large as the server says it
/// needs.
fn ask_for_snapshots(
    handle: &smb::resource::ResourceHandle,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    let mut size = FIRST_REPLY_SIZE;
    loop {
        let reply = match handle.fsctl_with_options(EnumerateSnapshots, size) {
            Ok(reply) => {
                SnapshotReply::parse(&reply.0).ok_or_else(|| failed("a malformed snapshot list".into()))?
            }
            Err(error)
                if size < LARGE_REPLY_SIZE
                    && matches!(
                        status_of(&error),
                        Some(status::BUFFER_OVERFLOW | status::BUFFER_TOO_SMALL)
                    ) =>
            {
                size = LARGE_REPLY_SIZE;
                check_cancelled(cancel)?;
                continue;
            }
            Err(error) => return Err(classify(error)),
        };
        // A reply too small for the list says how large it must be, and
        // names none.
        let needed = reply.needed_size();
        if reply.is_cut_short() && size < LARGE_REPLY_SIZE && needed > size {
            size = needed.min(LARGE_REPLY_SIZE);
            check_cancelled(cancel)?;
            continue;
        }
        return Ok(ShadowCopies::from_names(
            reply.names.iter().map(String::as_str),
            reply.is_cut_short(),
        ));
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
    array_size: u32,
    names: Vec<String>,
}

impl SnapshotReply {
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
            array_size,
            names,
        })
    }

    /// True when the server keeps more shadow copies than it named.
    fn is_cut_short(&self) -> bool {
        self.returned < self.total
            || usize::try_from(self.returned).is_ok_and(|returned| returned > self.names.len())
    }

    /// The reply size the whole list needs.
    fn needed_size(&self) -> u32 {
        self.array_size.saturating_add(REPLY_HEADER_BYTES)
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
            | status::ACCOUNT_DISABLED,
        ) => ShadowCopyError::SignInRefused,
        Some(status::INVALID_DEVICE_REQUEST | status::NOT_SUPPORTED | status::INVALID_DEVICE_STATE) => {
            ShadowCopyError::Unsupported
        }
        _ => failed(error.to_string()),
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
            .map(|account| (account.user, account.password.as_str().to_owned()))
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
        assert_eq!(cut_short.needed_size(), 116);
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
                ("server", "nas"),
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
        let account = saved_account(&lister.store, &target).unwrap().unwrap();
        locked(&lister.refused).insert(SmbShadowCopies::refusal_key(&target.key, &account));

        let outcome = lister.list("smb://nas/share/a.txt", &Cancellation::new());

        assert!(
            matches!(outcome, Err(ShadowCopyError::SignInRefused)),
            "{outcome:?}"
        );
    }
}
