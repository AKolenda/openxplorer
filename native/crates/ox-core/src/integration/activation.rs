// SPDX-License-Identifier: AGPL-3.0-only
//! What activating an item does, and which application opens a file.
//!
//! Ports `desktop/activation.py` (OPEN-001, OPEN-005). The decision uses
//! freshly queried metadata, never a cached row, so a folder named like a
//! file (`Archive.mp4`) still opens as a folder and a file whose cached
//! row claimed a folder opens as a file.

use super::applications::ApplicationInfo;
use super::mime_type::MimeType;
use crate::entry::{Entry, EntryError, EntryKind};
use crate::location::LocationError;

/// The app's own desktop IDs: the current one and the one packages
/// used before, both registered for `x-scheme-handler/smb`.
const OWN_DESKTOP_IDS: [&str; 2] = [
    "io.winspace.Development.desktop",
    "io.winspace.OpenXplorer.desktop",
];

/// Why an item could not be opened. `Display` is the message the window
/// shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    /// The item is a special file, a dangling link or of unknown type.
    #[error("This item is not a regular file or a readable folder.")]
    NotRegularOrFolder,
    /// No other application handles the content type.
    #[error("No application is installed for this file type. Use Open with… to choose one.")]
    NoApplication,
    /// The item turned out to be a folder, which is navigated instead.
    #[error("This item is a folder.")]
    IsFolder,
    /// The item is not a regular file.
    #[error("Cannot open a special filesystem object.")]
    SpecialObject,
    /// The item is on a share without a local path, and the application
    /// only accepts local paths.
    #[error(
        "This application needs a local path. Install gvfs-fuse or mount the share with CIFS, then \
         reopen it; or choose a URI-capable application with Open with…"
    )]
    NeedsLocalPath,
    /// The item could not be inspected; [`EntryError::needs_mount`] asks
    /// the caller to mount the share and try again.
    #[error(transparent)]
    Entry(#[from] EntryError),
    /// The location is not one the app can open.
    #[error(transparent)]
    Location(#[from] LocationError),
}

/// What activating (double-clicking or pressing Enter on) an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// Navigate into the folder, in the same tab.
    OpenFolder,
    /// Browse the ZIP archive, in the same tab.
    BrowseArchive,
    /// Open the file in its default application; the tab stays.
    OpenFile,
}

impl Activation {
    /// What activating `entry` does. `entry` must be freshly queried.
    ///
    /// # Errors
    ///
    /// [`OpenError::NotRegularOrFolder`] for special files, dangling links
    /// and items of unknown type that are not folders.
    pub fn for_entry(entry: &Entry) -> Result<Self, OpenError> {
        if opens_as_folder(entry) {
            return Ok(Self::OpenFolder);
        }
        if matches!(
            entry.kind,
            EntryKind::Special | EntryKind::Unknown | EntryKind::Symlink
        ) {
            return Err(OpenError::NotRegularOrFolder);
        }
        if is_zip(entry) {
            return Ok(Self::BrowseArchive);
        }
        Ok(Self::OpenFile)
    }
}

/// The application that opens a file: `default` if allowed, otherwise the
/// first allowed one of `candidates`.
///
/// Safety rule "never open a file with the app itself" (`choose_application` in
/// `activation.py`): the app is the `smb://` scheme handler, so it
/// can appear for a file on a share; handing the file back to it would
/// reopen the file as a folder, again and again. Its own IDs are skipped,
/// and so are applications that accept neither files nor URIs.
///
/// # Errors
///
/// [`OpenError::NoApplication`] when no allowed application remains.
pub fn choose_application<A: ApplicationInfo>(
    candidates: impl IntoIterator<Item = A>,
    default: Option<A>,
) -> Result<A, OpenError> {
    default
        .into_iter()
        .chain(candidates)
        .find(is_allowed_opener)
        .ok_or(OpenError::NoApplication)
}

/// True for an application other than this app that accepts files or
/// URIs.
fn is_allowed_opener<A: ApplicationInfo>(application: &A) -> bool {
    let is_openxplorer = application
        .id()
        .is_some_and(|id| OWN_DESKTOP_IDS.contains(&id.as_str()));
    !is_openxplorer && (application.supports_files() || application.supports_uris())
}

/// A folder, or a share or shortcut whose metadata says it navigates. A
/// regular file, special file or link never does, whatever its flag says.
fn opens_as_folder(entry: &Entry) -> bool {
    match entry.kind {
        EntryKind::Directory => true,
        EntryKind::File | EntryKind::Special | EntryKind::Symlink => false,
        EntryKind::Mountable | EntryKind::Shortcut | EntryKind::Unknown => entry.is_dir,
    }
}

/// A ZIP archive by content type or by a `.zip` name in any case.
fn is_zip(entry: &Entry) -> bool {
    let is_zip_type = entry
        .content_type
        .as_deref()
        .and_then(MimeType::from_name)
        .is_some_and(MimeType::is_zip);
    is_zip_type || entry.name.to_lowercase().ends_with(".zip")
}
