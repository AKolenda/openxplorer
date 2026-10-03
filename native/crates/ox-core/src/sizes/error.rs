// SPDX-License-Identifier: AGPL-3.0-only
//! Why a folder-size scan could not start or read its folder.
//!
//! Mirrors the `ValueError`s `scan_folder` in `v2.0.0:desktop/folder_sizes.py`
//! raises, word for word. Problems below the scanned folder never end a
//! scan; they are counted in its result instead.

use crate::entry::EntryError;
use crate::location::LocationError;

/// Why a folder-size scan could not start or read its folder. `Display`
/// is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SizeError {
    /// An address the location rules refuse.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A whole SMB server holds shares, which are measured one at a time.
    #[error(
        "{}",
        crate::i18n::gettext("Open or select a share first, not the whole SMB server.")
    )]
    ServerRoot,
    /// An entry or time limit of zero.
    #[error("{}", crate::i18n::gettext("Scan limits must be positive."))]
    InvalidLimits,
    /// Only a real folder is measured, never a file or a link to a folder.
    #[error(
        "{}",
        crate::i18n::gettext("Select a directory, not a file or symbolic link.")
    )]
    NotAFolder,
    /// The scanned folder itself could not be read. When
    /// [`EntryError::needs_mount`] is true, mount the location and scan
    /// again, as the Python app does.
    #[error(transparent)]
    Read(#[from] EntryError),
}
