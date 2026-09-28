// SPDX-License-Identifier: AGPL-3.0-only
//! Mounting network locations: shares on demand and Map network location.
//!
//! Ports `mount`, `after_connect`, `retry_list` and the `connect` operation
//! of `desktop/winspace.py`, and `verify_folder` of
//! `desktop/gio_backend.py`. Every function is asynchronous on the main
//! loop; dropping its future cancels the GIO call and aborts any sign-in
//! dialog it opened.
//!
//! The window does the rest of the Python methods:
//!
//! - Every successful mount of an SMB location, also the one of
//!   [`read_mounting_once`], reaches the window's
//!   [`MountPrompts::connect_server_mounted`] handlers, where it resumes
//!   indexing that server, which a Sign out paused (NET-022).
//! - After [`connect_share`] it saves the share in the sidebar if the user
//!   asked and remembers it for the Network list
//!   ([`VisitedNetwork`](super::VisitedNetwork)).

use std::future::Future;

use gio::prelude::*;

use super::error::NetworkError;
use super::prompts::{MountOutcome, MountPrompts};
use super::sign_out::SignOutRegistry;
use crate::entry::EntryError;
use crate::location::{normalise, require_share, safe_label, split_location, unquote_lossy};

/// The label of a mapped share whose address names no folder.
const FALLBACK_SHARE_LABEL: &str = "Network folder";

/// A share connected with Map network location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedShare {
    /// The canonical location of the share, verified to be a folder.
    pub uri: String,
    /// The sidebar label: the one typed, else the folder's name.
    pub label: String,
}

/// Whether files are being written, which unmounting must not interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteActivity {
    /// No copy, move, delete or other write is running.
    Idle,
    /// A write is running.
    Writing,
}

/// A read failure that mounting the location may cure.
pub trait NeedsMount {
    /// True when the location is on a volume or share that is not mounted.
    fn needs_mount(&self) -> bool;
}

impl NeedsMount for EntryError {
    fn needs_mount(&self) -> bool {
        EntryError::needs_mount(self)
    }
}

/// Why a read with one mount attempt failed.
#[derive(Debug, thiserror::Error)]
pub enum MountedReadError<E> {
    /// The read failed; after a mount, the retry failed.
    #[error(transparent)]
    Read(E),
    /// Mounting the location failed, for example a rejected password.
    #[error(transparent)]
    Mount(NetworkError),
}

/// Mounts the volume that holds `uri`, answering `GVfs`'s sign-in through
/// `prompts`. A location that is already mounted succeeds. On success,
/// `prompts` keep the account it signed in with and report an SMB server
/// to their [`MountPrompts::connect_server_mounted`] handlers.
///
/// # Errors
///
/// The [`LocationError`](crate::location::LocationError) of an invalid
/// address, or GIO's error, for example a rejected password.
pub async fn mount_location(prompts: &MountPrompts, uri: &str) -> Result<(), NetworkError> {
    let uri = normalise(uri)?;
    let mut prompted = PromptedOperation::new(prompts, &uri)?;
    let file = gio::File::for_uri(&uri);
    let mounted = file
        .mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&prompted.operation))
        .await;
    if let Err(error) = mounted {
        if !error.matches(gio::IOErrorEnum::AlreadyMounted) {
            return Err(error.into());
        }
    }
    prompted.outcome = MountOutcome::Mounted;
    Ok(())
}

/// Runs `read` on `uri`; when it fails because the location is not
/// mounted, mounts it once with [`mount_location`] (asking for credentials
/// if needed) and runs `read` again (NET-004).
///
/// Only for reads, such as listing a folder, reading properties or opening
/// a file. Safety rule (NET-004): a write (paste, create, rename, extract)
/// is never replayed after a mount or sign-in, so it cannot run twice.
///
/// # Errors
///
/// [`MountedReadError::Read`] with the read's error, including a second
/// "not mounted"; [`MountedReadError::Mount`] when mounting failed.
pub async fn read_mounting_once<T, E, Read>(
    prompts: &MountPrompts,
    uri: &str,
    read: impl FnMut() -> Read,
) -> Result<T, MountedReadError<E>>
where
    E: NeedsMount,
    Read: Future<Output = Result<T, E>>,
{
    retry_after_mount(|| mount_location(prompts, uri), read).await
}

/// [`read_mounting_once`] with the mount step passed in.
async fn retry_after_mount<T, E, Read, Mount>(
    mount: impl FnOnce() -> Mount,
    mut read: impl FnMut() -> Read,
) -> Result<T, MountedReadError<E>>
where
    E: NeedsMount,
    Read: Future<Output = Result<T, E>>,
    Mount: Future<Output = Result<(), NetworkError>>,
{
    match read().await {
        Err(error) if error.needs_mount() => {}
        finished => return finished.map_err(MountedReadError::Read),
    }
    mount().await.map_err(MountedReadError::Mount)?;
    read().await.map_err(MountedReadError::Read)
}

/// Map network location: mounts the share at `address` (`\\nas\Projects`
/// or `smb://nas/Projects`) and checks that it is a folder. Saving it in
/// the sidebar is the caller's choice.
///
/// # Errors
///
/// A [`NetworkError`] for an address that is not a share, a label that is
/// too long, a server being signed out, a failed mount, or a location that
/// is not a folder.
pub async fn connect_share(
    prompts: &MountPrompts,
    signing_out: &SignOutRegistry,
    address: &str,
    label: &str,
) -> Result<ConnectedShare, NetworkError> {
    let uri = require_share(address)?;
    // NET-023: a server being signed out is not mounted again halfway.
    if signing_out.is_signing_out(&uri) {
        return Err(NetworkError::ReconnectAfterSignOut);
    }
    let label = safe_label(label, &share_name(&uri))?;
    mount_location(prompts, &uri).await?;
    verify_folder(&uri).await?;
    Ok(ConnectedShare { uri, label })
}

/// The folder's own name, for a mapped share without a label: the last
/// segment of its path, as the settings name a saved share
/// (`Settings.bookmark` in `desktop/core.py`).
fn share_name(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_SHARE_LABEL.to_owned();
    };
    let path = unquote_lossy(&parts.path);
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    if name.is_empty() {
        return FALLBACK_SHARE_LABEL.to_owned();
    }
    name.to_owned()
}

/// Checks that `uri` is a folder (`verify_folder` in `gio_backend.py`).
async fn verify_folder(uri: &str) -> Result<(), NetworkError> {
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    if info.file_type() != gio::FileType::Directory {
        return Err(NetworkError::NotAFolder);
    }
    Ok(())
}

/// A mount operation from the prompts, reported back to them when the
/// mount ends, including when its future is dropped.
pub(super) struct PromptedOperation<'a> {
    prompts: &'a MountPrompts,
    pub(super) operation: gio::MountOperation,
    /// Reported on drop; stays [`MountOutcome::Failed`] unless the mount
    /// succeeded.
    pub(super) outcome: MountOutcome,
}

impl<'a> PromptedOperation<'a> {
    pub(super) fn new(prompts: &'a MountPrompts, uri: &str) -> Result<Self, NetworkError> {
        Ok(Self {
            operation: prompts.create(uri)?,
            prompts,
            outcome: MountOutcome::Failed,
        })
    }
}

impl Drop for PromptedOperation<'_> {
    fn drop(&mut self) {
        self.prompts.finish(&self.operation, self.outcome);
    }
}

#[cfg(test)]
mod tests;
