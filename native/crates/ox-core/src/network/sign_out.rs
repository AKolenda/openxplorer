// SPDX-License-Identifier: AGPL-3.0-only
//! Sign out of an SMB server: disconnect its mounts and forget its
//! credentials.
//!
//! Ports `sign_out` in `desktop/winspace.py`, which checks every
//! precondition before it changes anything. The server's part comes in two
//! steps, and the window does the rest of the Python method between and
//! after them:
//!
//! 1. [`begin_sign_out`] checks the preconditions, marks the server in the
//!    [`SignOutRegistry`] and forgets its in-memory credential in every
//!    window. It changes nothing when it refuses.
//! 2. The window tells every window (`serverSigningOut`), forgets the
//!    server's [`VisitedNetwork`] roots, cancels the loads and folder
//!    monitors on the server and pauses its indexing.
//! 3. [`finish_sign_out`] unmounts every mount of the server and deletes
//!    the saved credentials.
//! 4. The window clears the server's search cache if asked, then drops the
//!    [`SigningOut`], which ends the sign-out, and refreshes every window.
//!
//! [`VisitedNetwork`]: super::VisitedNetwork

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gio::prelude::*;

use super::credential_store::ForgetScope;
use super::error::NetworkError;
use super::keyring::{KeyringError, SecretAttributes};
use super::mounting::{PromptedOperation, WriteActivity};
use super::prompts::MountPrompts;
use super::server::smb_host;
use super::session_credentials::SessionCredentials;
use crate::location::normalise;

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

    /// Marks `host` as being signed out until the returned entry is
    /// dropped.
    fn begin(&self, host: &str) -> Result<RegistryEntry<'_>, NetworkError> {
        let is_new = self.hosts().insert(host.to_owned());
        if !is_new {
            return Err(NetworkError::SignOutAlreadyRunning);
        }
        Ok(RegistryEntry {
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

/// A server marked in the registry; dropping it removes the mark, also
/// when the sign-out fails or its future is dropped.
#[derive(Debug)]
struct RegistryEntry<'a> {
    registry: &'a SignOutRegistry,
    host: String,
}

impl Drop for RegistryEntry<'_> {
    fn drop(&mut self) {
        self.registry.hosts().remove(&self.host);
    }
}

/// What the user chose in the "Sign out of `<host>`?" dialog.
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

/// A sign-out that [`begin_sign_out`] started. The server stays marked in
/// the [`SignOutRegistry`] until this is dropped.
#[derive(Debug)]
pub struct SigningOut<'a> {
    entry: RegistryEntry<'a>,
    prompts: &'a MountPrompts,
    /// The canonical location the user chose.
    uri: String,
    forget: ForgetScope,
}

impl SigningOut<'_> {
    /// The server being signed out, lower-case.
    pub fn host(&self) -> &str {
        &self.entry.host
    }
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

/// Starts signing out of the server of `request.uri`: checks every
/// precondition, marks the server in `registry` and forgets its in-memory
/// credential in every window. Then do the window's part and call
/// [`finish_sign_out`].
///
/// # Errors
///
/// [`NetworkError::SignOutDuringWrites`], a
/// [`LocationError`](crate::location::LocationError),
/// [`NetworkError::NotAnSmbLocation`] or
/// [`NetworkError::SignOutAlreadyRunning`]; nothing is changed then.
pub fn begin_sign_out<'a>(
    prompts: &'a MountPrompts,
    registry: &'a SignOutRegistry,
    request: SignOutRequest<'_>,
) -> Result<SigningOut<'a>, NetworkError> {
    // Safety rule (NET-023): unmounting under a running copy or move
    // could leave half-written files.
    if request.writes == WriteActivity::Writing {
        return Err(NetworkError::SignOutDuringWrites);
    }
    let uri = normalise(request.uri)?;
    let host = smb_host(&uri).ok_or(NetworkError::NotAnSmbLocation)?;
    let entry = registry.begin(&host)?;
    // Safety rule (NET-021, SAFE-012): every window's in-memory copy and
    // every keyring save still in flight for this server are discarded
    // from here on.
    prompts.credentials().forget_memory(&uri);
    Ok(SigningOut {
        entry,
        prompts,
        uri,
        forget: request.forget,
    })
}

/// Finishes `signing_out`: disconnects every mount of its server among
/// `mounts` (the volume monitor's), in every application of the session,
/// then deletes the saved credentials in the chosen scope.
///
/// Unmounting may show the programs that keep a mount busy through the
/// prompts; it never kills a program or forces an unmount.
///
/// # Errors
///
/// [`NetworkError::MountCannotBeDisconnected`] or GIO's error when a mount
/// stays connected; [`NetworkError::CredentialsNotRemoved`] when the
/// server was disconnected but the keyring could not delete its entries.
pub async fn finish_sign_out(
    signing_out: &SigningOut<'_>,
    mounts: &[gio::Mount],
) -> Result<SignOutReport, NetworkError> {
    finish_within(signing_out, mounts, KEYRING_DEADLINE).await
}

/// [`finish_sign_out`], giving up on the keyring after `keyring_deadline`.
async fn finish_within(
    signing_out: &SigningOut<'_>,
    mounts: &[gio::Mount],
    keyring_deadline: Duration,
) -> Result<SignOutReport, NetworkError> {
    let host = signing_out.host();
    let server_mounts = mounts_of_host(mounts, host);
    // The newest mount first, as `mounts.pop()` in winspace.py.
    for mount in server_mounts.iter().rev() {
        disconnect(signing_out.prompts, mount).await?;
    }
    let credentials_removed = forget_saved_credentials(signing_out, keyring_deadline).await?;
    Ok(SignOutReport {
        host: host.to_owned(),
        disconnected: server_mounts.len(),
        credentials_removed,
        forget: signing_out.forget,
    })
}

/// The mounts among `mounts` whose root is on `host`.
fn mounts_of_host(mounts: &[gio::Mount], host: &str) -> Vec<gio::Mount> {
    let is_on_this_host = |mount: &&gio::Mount| is_on_host(&mount.root().uri(), host);
    mounts.iter().filter(is_on_this_host).cloned().collect()
}

/// True when the mount root `root_uri` is an SMB location on `host`, on
/// any port. Sign out disconnects exactly these mounts, including those of
/// other applications.
fn is_on_host(root_uri: &str, host: &str) -> bool {
    root_uri.starts_with("smb:") && smb_host(root_uri).as_deref() == Some(host)
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

/// Deletes the saved credentials of the server off the main thread;
/// returns whether the keyring deleted anything.
async fn forget_saved_credentials(
    signing_out: &SigningOut<'_>,
    deadline: Duration,
) -> Result<bool, NetworkError> {
    let credentials = Arc::clone(signing_out.prompts.credentials());
    let uri = signing_out.uri.clone();
    let host = signing_out.host().to_owned();
    let forget = signing_out.forget;
    // The keyring may show an unlock prompt: never block the main thread.
    let worker = gio::spawn_blocking(move || delete_saved_credentials(&credentials, &uri, &host, forget));
    let Ok(finished) = glib::future_with_timeout(deadline, worker).await else {
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
