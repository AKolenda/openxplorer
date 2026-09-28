// SPDX-License-Identifier: AGPL-3.0-only
//! Sign out of an SMB server: disconnect its mounts and forget its
//! credentials.
//!
//! Ports `sign_out` in `desktop/winspace.py`. [`sign_out`] does the
//! server's part: it checks the preconditions, marks the server in the
//! [`SignOutRegistry`], forgets the in-memory credential, unmounts every
//! mount of the server and deletes the saved credentials. The window does
//! the rest of the Python method around it: before, it tells every window
//! (`serverSigningOut`), forgets the server's [`VisitedNetwork`] roots,
//! cancels the loads and folder monitors on the server and pauses its
//! indexing; after, it clears the server's search cache if asked and
//! refreshes every window.
//!
//! [`VisitedNetwork`]: super::VisitedNetwork

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gio::prelude::*;

use super::error::NetworkError;
use super::keyring::{KeyringError, SecretAttributes};
use super::mounting::{PromptedOperation, WriteActivity};
use super::prompts::MountPrompts;
use super::session_credentials::{ForgetScope, SessionCredentials};
use crate::location::{normalise, split_location};

/// The `protocol` attribute of the SMB passwords `GVfs` remembers under
/// GNOME's `org.gnome.keyring.NetworkPassword` schema. Those items are
/// matched by their attributes only, whatever their schema, as libsecret
/// does for that schema (`SECRET_SCHEMA_DONT_MATCH_NAME` in `winspace.py`).
const GVFS_SMB_PROTOCOL: &str = "smb";

/// How long deleting the saved credentials may take before Sign out gives
/// up (`.result(timeout=25)` in `winspace.py`).
const KEYRING_DEADLINE: Duration = Duration::from_secs(25);

/// The servers being signed out, shared by every window.
///
/// While a server is signed out, listing it fails with
/// [`NetworkError::ServerSigningOut`] and connecting to it with
/// [`NetworkError::ReconnectAfterSignOut`] (NET-023), so no window mounts
/// it again halfway through.
#[derive(Debug, Default)]
pub struct SignOutRegistry {
    /// Lower-case host names.
    hosts: Mutex<HashSet<String>>,
}

impl SignOutRegistry {
    /// True when `uri` is an SMB location on a server being signed out.
    pub fn is_signing_out(&self, uri: &str) -> bool {
        smb_host(uri).is_some_and(|host| self.hosts().contains(&host))
    }

    /// Refuses to list `uri` while its server is being signed out.
    ///
    /// # Errors
    ///
    /// [`NetworkError::ServerSigningOut`] for a location on such a server.
    pub fn check_listing(&self, uri: &str) -> Result<(), NetworkError> {
        if self.is_signing_out(uri) {
            return Err(NetworkError::ServerSigningOut);
        }
        Ok(())
    }

    /// Marks `host` as being signed out until the returned guard is
    /// dropped.
    fn begin(&self, host: &str) -> Result<SigningOut<'_>, NetworkError> {
        let is_new = self.hosts().insert(host.to_owned());
        if !is_new {
            return Err(NetworkError::SignOutAlreadyRunning);
        }
        Ok(SigningOut {
            registry: self,
            host: host.to_owned(),
        })
    }

    fn hosts(&self) -> MutexGuard<'_, HashSet<String>> {
        // The set is valid after every statement, so a panic elsewhere
        // cannot leave it half-changed.
        self.hosts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A server marked in the registry; dropping it ends the sign-out, also
/// when the sign-out fails or its future is dropped.
struct SigningOut<'a> {
    registry: &'a SignOutRegistry,
    host: String,
}

impl Drop for SigningOut<'_> {
    fn drop(&mut self) {
        self.registry.hosts().remove(&self.host);
    }
}

/// What the user chose in the "Sign out of <host>?" dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignOutRequest<'a> {
    /// A location on the server.
    pub uri: &'a str,
    /// "Forget saved credentials for this server": checked is
    /// [`ForgetScope::AllScopes`], the default.
    pub forget: ForgetScope,
    /// Whether any window is writing files. Sign out is refused then.
    pub writes: WriteActivity,
}

/// What Sign out did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignOutReport {
    /// The server, lower-case.
    pub host: String,
    /// How many mounts of the server were disconnected.
    pub disconnected: usize,
    /// Whether saved credentials were deleted from the keyring:
    /// `OpenXplorer`'s own or the passwords `GVfs` remembered.
    pub credentials_removed: bool,
    /// Which saved credentials were asked to be forgotten.
    pub forget: ForgetScope,
}

/// Signs out of the server of `request.uri`: disconnects every one of its
/// mounts among `mounts` (the volume monitor's), in every application of
/// the session, then deletes its saved credentials in the chosen scope.
///
/// Unmounting may show the programs that keep a mount busy through
/// `prompts`; it never kills a program or forces an unmount.
///
/// # Errors
///
/// [`NetworkError::SignOutDuringWrites`], [`NetworkError::NotAnSmbLocation`]
/// or [`NetworkError::SignOutAlreadyRunning`] before anything is changed;
/// [`NetworkError::MountCannotBeDisconnected`] or GIO's error when a mount
/// stays connected; [`NetworkError::CredentialsNotRemoved`] when the
/// server was disconnected but the keyring could not delete its entries.
pub async fn sign_out(
    prompts: &MountPrompts,
    registry: &SignOutRegistry,
    mounts: &[gio::Mount],
    request: SignOutRequest<'_>,
) -> Result<SignOutReport, NetworkError> {
    // Safety rule (NET-023): unmounting under a running copy or move
    // could leave half-written files.
    if request.writes == WriteActivity::Writing {
        return Err(NetworkError::SignOutDuringWrites);
    }
    let uri = normalise(request.uri)?;
    let host = smb_host(&uri).ok_or(NetworkError::NotAnSmbLocation)?;
    let _signing_out = registry.begin(&host)?;
    let credentials = prompts.credentials();
    // Safety rule (SAFE-012): a keyring save still in flight for this
    // server is discarded from here on.
    credentials.forget_memory(&uri);
    let server_mounts = mounts_of_host(mounts, &host);
    for mount in server_mounts.iter().rev() {
        disconnect(prompts, mount).await?;
    }
    let credentials_removed = forget_saved_credentials(credentials, &uri, &host, request.forget).await?;
    Ok(SignOutReport {
        host,
        disconnected: server_mounts.len(),
        credentials_removed,
        forget: request.forget,
    })
}

/// The lower-case host of SMB location `uri`, or `None` for anything else.
fn smb_host(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    if !parts.is_smb() {
        return None;
    }
    parts.hostname()
}

/// The SMB mounts whose root is on `host`.
fn mounts_of_host(mounts: &[gio::Mount], host: &str) -> Vec<gio::Mount> {
    let is_on_host = |mount: &&gio::Mount| {
        let root = mount.root().uri();
        root.starts_with("smb:") && smb_host(&root).as_deref() == Some(host)
    };
    mounts.iter().filter(is_on_host).cloned().collect()
}

/// Unmounts `mount`, answering its questions through `prompts`.
async fn disconnect(prompts: &MountPrompts, mount: &gio::Mount) -> Result<(), NetworkError> {
    if !mount.can_unmount() {
        return Err(NetworkError::MountCannotBeDisconnected);
    }
    let root = mount.root().uri();
    // Unmounting saves no credentials, so the outcome stays Failed.
    let prompted = PromptedOperation::new(prompts, &root)?;
    mount
        .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&prompted.operation))
        .await?;
    Ok(())
}

/// Deletes the saved credentials of the server of `uri` off the main
/// thread; returns whether the keyring deleted anything.
async fn forget_saved_credentials(
    credentials: &Arc<SessionCredentials>,
    uri: &str,
    host: &str,
    forget: ForgetScope,
) -> Result<bool, NetworkError> {
    let credentials = Arc::clone(credentials);
    let uri = uri.to_owned();
    let host = host.to_owned();
    // The keyring may show an unlock prompt: never block the main thread.
    let worker = gio::spawn_blocking(move || delete_saved_credentials(&credentials, &uri, &host, forget));
    let Ok(finished) = glib::future_with_timeout(KEYRING_DEADLINE, worker).await else {
        return Err(NetworkError::CredentialsNotRemoved(KeyringError::TimedOut));
    };
    // A panic in the worker is a bug; report it where it happened.
    let deleted = finished.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    deleted.map_err(NetworkError::CredentialsNotRemoved)
}

/// Deletes `OpenXplorer`'s entries for the server of `uri` in `forget`'s
/// scope and, when forgetting everything, the passwords `GVfs` remembered
/// for `host`.
fn delete_saved_credentials(
    credentials: &SessionCredentials,
    uri: &str,
    host: &str,
    forget: ForgetScope,
) -> Result<bool, KeyringError> {
    let own_removed = credentials.forget(uri, forget)?;
    if forget == ForgetScope::SessionOnly {
        return Ok(own_removed);
    }
    // Safety rule (NET-021): GVfs keeps its own copy of a remembered
    // password; forgetting the server deletes that copy too.
    let keyring = credentials.keyring().ok_or(KeyringError::Unavailable)?;
    let gnome_passwords = SecretAttributes::any_schema()
        .with("server", host)
        .with("protocol", GVFS_SMB_PROTOCOL);
    let gvfs_removed = keyring.clear(&gnome_passwords)?;
    Ok(own_removed || gvfs_removed)
}

#[cfg(test)]
mod tests;
