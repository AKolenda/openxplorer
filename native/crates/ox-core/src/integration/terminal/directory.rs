// SPDX-License-Identifier: AGPL-3.0-only
//! The folder a terminal opens in: checked against fresh metadata, the
//! read-only locations and the real file system.
//!
//! Ports `checked_directory` and `prepare_directory` in
//! `desktop/terminal_integration.py` (OPEN-017, OPEN-020). A location is
//! the only input; no command, program, argument or environment is ever
//! taken from the caller. A file opens its folder, and a folder on a share
//! opens through its local mount: a local shell, never an SSH session.

use std::fmt;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::Access;

use super::TerminalError;
use crate::entry::{Entry, EntryError, EntryKind};
use crate::location::{file_uri, is_smb_server, normalise, split_location, LocationContext};
use crate::transfer::Cancellation;

/// The longest folder path accepted, in characters.
const MAX_DIRECTORY_CHARS: usize = 16_384;

/// What preparing a terminal folder needs from the rest of the app.
pub trait DirectoryChecks {
    /// Why the write guard refused a location; shown as it is.
    type Refusal: fmt::Display;

    /// Fresh metadata of `uri`: `entry::inspect` in the app.
    ///
    /// # Errors
    ///
    /// The [`EntryError`] of the query.
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<Entry, EntryError>;

    /// The local path of the folder `uri`, for a share through its CIFS or
    /// `GVfs` FUSE mount: the network service's lookup in the app.
    fn local_path(&self, uri: &str) -> Option<PathBuf>;

    /// Refuses a location inside a configured snapshot or backup: the
    /// previous-versions write guard in the app.
    ///
    /// # Errors
    ///
    /// The guard's refusal.
    fn check_writable(&self, uri: &str) -> Result<(), Self::Refusal>;
}

/// A folder a terminal may open in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDirectory {
    /// The folder's canonical URI.
    pub uri: String,
    /// The folder's real local path, with every symbolic link resolved.
    pub path: PathBuf,
    /// The folder is on an SMB share, reached through its local mount.
    pub is_network: bool,
}

/// Checks `path` as a terminal's starting folder and returns its real
/// path.
///
/// Safety rule "resolve aliases before checking" (`checked_directory` in
/// `terminal_integration.py`): the path must be absolute, without control
/// characters, and is resolved before it is checked to be an enterable
/// folder, so a link cannot swap in something else.
///
/// # Errors
///
/// [`TerminalError::InvalidDirectory`], [`TerminalError::NotADirectory`],
/// [`TerminalError::NoPermission`], or [`TerminalError::Io`] when the path
/// cannot be resolved, for example because it does not exist.
pub fn checked_directory(path: &Path) -> Result<PathBuf, TerminalError> {
    let bytes = path.as_os_str().as_bytes();
    let is_absolute = bytes.first() == Some(&b'/');
    let has_control_character = bytes.iter().any(|byte| *byte < 0x20 || *byte == 0x7f);
    let is_short_enough = path.to_string_lossy().chars().count() <= MAX_DIRECTORY_CHARS;
    if !is_absolute || has_control_character || !is_short_enough {
        return Err(TerminalError::InvalidDirectory);
    }
    let io_error = |error| TerminalError::Io {
        path: path.to_owned(),
        error,
    };
    let resolved = fs::canonicalize(path).map_err(io_error)?;
    if !fs::metadata(&resolved).map_err(io_error)?.is_dir() {
        return Err(TerminalError::NotADirectory);
    }
    if rustix::fs::access(&resolved, Access::EXEC_OK).is_err() {
        return Err(TerminalError::NoPermission);
    }
    Ok(resolved)
}

/// Decides the folder a terminal for `uri` opens in (OPEN-017): the
/// folder itself, or a file's parent folder. Runs GIO synchronously
/// through `checks`; call it on a worker thread.
///
/// Safety rule "query reality, not a cached row" (`prepare_directory` in
/// `terminal_integration.py`): the item is inspected again, links and
/// special files are refused, and both the location and the resolved local
/// folder must be live locations, not snapshots, so a link into a snapshot
/// is refused too.
///
/// # Errors
///
/// A [`TerminalError`] for every refusal; [`TerminalError::Cancelled`]
/// once `cancel` is cancelled.
pub fn prepare_directory<C: DirectoryChecks>(
    uri: &str,
    checks: &C,
    cancel: &Cancellation,
) -> Result<PreparedDirectory, TerminalError> {
    let uri = normalise(uri)?;
    if is_smb_server(&uri) {
        return Err(TerminalError::ServerListing);
    }
    check_writable(checks, &uri)?;
    stop_if_cancelled(cancel)?;
    let entry = checks.inspect(&uri, cancel)?;
    let directory_uri = directory_of(&uri, &entry)?;
    if is_smb_server(&directory_uri) {
        return Err(TerminalError::ShareNeeded);
    }
    check_live_location(checks, &directory_uri)?;
    let local_path = checks
        .local_path(&directory_uri)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(TerminalError::NeedsLocalMount)?;
    let path = checked_directory(&local_path)?;
    // Also check the resolved folder, which may be a link into a snapshot.
    check_live_location(checks, &file_uri(&path))?;
    stop_if_cancelled(cancel)?;
    Ok(PreparedDirectory {
        is_network: directory_uri.starts_with("smb:"),
        uri: directory_uri,
        path,
    })
}

/// The folder a terminal for the item `entry` at `uri` opens in.
fn directory_of(uri: &str, entry: &Entry) -> Result<String, TerminalError> {
    let is_link_or_special = matches!(
        entry.kind,
        EntryKind::Symlink | EntryKind::Special | EntryKind::Unknown
    );
    if entry.is_symlink || is_link_or_special {
        return Err(TerminalError::LinkOrSpecialFile);
    }
    if entry.is_dir {
        return Ok(normalise(entry.target_uri.as_deref().unwrap_or(uri))?);
    }
    if entry.kind == EntryKind::File {
        return parent_of(uri);
    }
    Err(TerminalError::NotFileOrFolder)
}

/// The folder that holds the file at `uri`.
fn parent_of(uri: &str) -> Result<String, TerminalError> {
    let parts = split_location(uri)?;
    let path = parts.path.trim_end_matches('/');
    let parent = match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent,
        _ => "/",
    };
    Ok(normalise(&format!(
        "{}://{}{parent}",
        parts.scheme, parts.authority
    ))?)
}

/// Refuses a snapshot location, by the app's guard and by its name.
fn check_live_location<C: DirectoryChecks>(checks: &C, uri: &str) -> Result<(), TerminalError> {
    check_writable(checks, uri)?;
    // The conventional snapshot folders (`.snapshot`, `@GMT-…`,
    // `.zfs/snapshot`, ...) are refused even where no source is configured.
    if LocationContext::default().is_snapshot_location(uri) {
        return Err(TerminalError::PreviousVersion);
    }
    Ok(())
}

/// The app's write guard, as a [`TerminalError`].
fn check_writable<C: DirectoryChecks>(checks: &C, uri: &str) -> Result<(), TerminalError> {
    checks
        .check_writable(uri)
        .map_err(|refusal| TerminalError::Protected(refusal.to_string()))
}

/// Stops once the user cancelled.
fn stop_if_cancelled(cancel: &Cancellation) -> Result<(), TerminalError> {
    if cancel.is_cancelled() {
        return Err(TerminalError::Cancelled);
    }
    Ok(())
}
