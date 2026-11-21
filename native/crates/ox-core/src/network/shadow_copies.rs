// SPDX-License-Identifier: AGPL-3.0-only
//! Asking an SMB server for the shadow copies of an item, as Windows'
//! Previous Versions tab does.
//!
//! GIO, `GVfs` and libsmbclient cannot send `FSCTL_SRV_ENUMERATE_SNAPSHOTS`,
//! so [`SmbShadowCopies`] opens a connection of its own with the pure-Rust
//! `smb` client, only to send that request: negotiate, sign in, connect
//! the share, open the item for its attributes, ask, close. The versions
//! themselves are read through GIO.
//!
//! It signs in with the account the user already saved for the server:
//! `OpenXplorer`'s own ([`SessionCredentials`]), else the password `GVfs`
//! remembered under GNOME's `org.gnome.keyring.NetworkPassword` schema when
//! it mounted the share. It never asks for a password. Safety rule
//! (NET-014): an account is only sent to the host name and port it was
//! saved for.

use std::sync::Arc;
use std::time::Duration;

use smb::{
    Client, ClientConfig, ConnectionConfig, FileAccessMask, FileCreateArgs, SrvEnumerateSnapshotsRequest,
    UncPath,
};

use super::credential_store::CredentialStore;
use super::keyring::{KeyringError, SecretAttributes};
use super::server::{ServerKey, DEFAULT_SMB_PORT};
use super::session_credentials::SessionCredentials;
use crate::location::{split_location, unquote_without_controls};
use crate::transfer::Cancellation;
use crate::versions::{ShadowCopies, ShadowCopyError, ShadowCopyLister};

/// How long the server may take to answer each request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The first reply size asked for: room for about 1,200 shadow copies,
/// which one credit covers.
const FIRST_REPLY_SIZE: u32 = 64 * 1024;

/// The reply size asked for once a server said the first was too small.
const LARGE_REPLY_SIZE: u32 = 1024 * 1024;

/// The `protocol` attribute of the passwords `GVfs` remembers for SMB.
const GVFS_SMB_PROTOCOL: &str = "smb";

/// NT status codes the server answers with.
mod status {
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
}

impl std::fmt::Debug for SmbShadowCopies {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SmbShadowCopies").finish_non_exhaustive()
    }
}

impl SmbShadowCopies {
    /// A lister signing in with the accounts saved in `store`'s keyring.
    pub fn new(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }
}

impl ShadowCopyLister for SmbShadowCopies {
    fn list(&self, uri: &str, cancel: &Cancellation) -> Result<ShadowCopies, ShadowCopyError> {
        let target = SmbTarget::of(uri).ok_or_else(|| ShadowCopyError::Failed {
            detail: format!("not an SMB share location: {uri}"),
        })?;
        let account = saved_account(&self.store, uri, &target).map_err(|error| ShadowCopyError::Failed {
            detail: error.to_string(),
        })?;
        let account = account.ok_or(ShadowCopyError::NoCredential)?;
        if cancel.is_cancelled() {
            return Err(ShadowCopyError::Cancelled);
        }
        let names = enumerate(&target, account, cancel)?;
        Ok(ShadowCopies::from_names(
            names.tokens.iter().map(String::as_str),
            names.is_truncated,
        ))
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
        let mut components = parts.path.split('/').filter(|part| !part.is_empty());
        let share = unquote_without_controls(components.next()?).ok()?;
        let path = components
            .map(unquote_without_controls)
            .collect::<Result<Vec<_>, _>>()
            .ok()?
            .join("\\");
        // A decoded separator would name another item than the location.
        if share.is_empty() || share.contains(['/', '\\']) || path.contains('/') {
            return None;
        }
        Some(Self { key, share, path })
    }
}

/// An account to sign in with.
struct Account {
    /// `DOMAIN\user`, or `user` for the server's default domain.
    user: String,
    password: String,
}

/// The account saved for the server of `target`: `OpenXplorer`'s, else the
/// one `GVfs` remembered; `None` when neither saved one.
fn saved_account(
    store: &Arc<CredentialStore>,
    uri: &str,
    target: &SmbTarget,
) -> Result<Option<Account>, KeyringError> {
    if let Some(credential) = SessionCredentials::new(Arc::clone(store)).load(uri)? {
        return Ok(Some(Account {
            user: qualified_user(&credential.domain, &credential.username),
            password: credential.password.as_str().to_owned(),
        }));
    }
    let Some(keyring) = store.keyring() else {
        return Ok(None);
    };
    let mut query = SecretAttributes::any_schema()
        .with("server", target.key.host())
        .with("protocol", GVFS_SMB_PROTOCOL);
    if target.key.port() != DEFAULT_SMB_PORT {
        query = query.with("port", &target.key.port().to_string());
    }
    let Some((attributes, password)) = keyring.lookup_item(&query)? else {
        return Ok(None);
    };
    // GVfs leaves `port` out for the default port; a password saved for
    // another port of the host is not this server's.
    if target.key.port() == DEFAULT_SMB_PORT && attributes.get("port").is_some_and(|port| port != "445") {
        return Ok(None);
    }
    let Some(user) = attributes.get("user").filter(|user| !user.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(Account {
        user: qualified_user(attributes.get("domain").unwrap_or_default(), user),
        password,
    }))
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
/// item. The connection is closed again whatever happens.
fn enumerate(
    target: &SmbTarget,
    account: Account,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    let config = ClientConfig {
        // A DFS referral would send the account to another server (NET-014).
        dfs: false,
        connection: ConnectionConfig {
            port: Some(target.key.port()),
            timeout: Some(REQUEST_TIMEOUT),
            ..ConnectionConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = Client::new(config);
    let outcome = enumerate_with(&client, target, account, cancel);
    // Closing only ends the session; it cannot change the answer.
    let _ = client.close();
    outcome
}

fn enumerate_with(
    client: &Client,
    target: &SmbTarget,
    account: Account,
    cancel: &Cancellation,
) -> Result<ShadowCopies, ShadowCopyError> {
    let share = UncPath::new(target.key.host())
        .and_then(|server| server.with_share(&target.share))
        .map_err(failed)?;
    client
        .share_connect(&share, &account.user, account.password)
        .map_err(classify)?;
    if cancel.is_cancelled() {
        return Err(ShadowCopyError::Cancelled);
    }
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
    let reply = match handle.fsctl_with_options(SrvEnumerateSnapshotsRequest(()), FIRST_REPLY_SIZE) {
        Err(error) if status_of(&error) == Some(status::BUFFER_TOO_SMALL) => {
            handle.fsctl_with_options(SrvEnumerateSnapshotsRequest(()), LARGE_REPLY_SIZE)
        }
        other => other,
    };
    // Closing the item only releases it; it cannot change the answer.
    let _ = handle.close();
    let reply = reply.map_err(classify)?;
    let names: Vec<String> = reply.snap_shots.iter().map(ToString::to_string).collect();
    let is_truncated = reply.number_of_snap_shots_returned < reply.number_of_snap_shots
        || usize::try_from(reply.number_of_snap_shots_returned).is_ok_and(|returned| returned > names.len());
    Ok(ShadowCopies {
        tokens: names,
        is_truncated,
    })
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
        _ => failed(error),
    }
}

/// An error the app has no words of its own for.
#[allow(clippy::needless_pass_by_value)]
fn failed(error: smb::Error) -> ShadowCopyError {
    ShadowCopyError::Failed {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::test_support::MemoryKeyring;
    use crate::network::{Credential, CredentialScope, Keyring, KeyringCollection, NewSecret, Password};

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

    fn account_for(store: &Arc<CredentialStore>, uri: &str) -> Option<(String, String)> {
        let target = SmbTarget::of(uri).unwrap();
        saved_account(store, uri, &target)
            .unwrap()
            .map(|account| (account.user, account.password))
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
        assert_eq!(SmbTarget::of("file:///srv/share"), None);
    }

    /// parity: PROP-032, NET-014
    #[test]
    fn the_password_gvfs_saved_for_the_server_signs_in_when_openxplorer_has_none() {
        let keyring = Arc::new(MemoryKeyring::default());
        let store = Arc::new(CredentialStore::new(keyring.clone()));
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
        assert_eq!(account_for(&store, "smb://nas:1445/share/a.txt"), None);
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
}
