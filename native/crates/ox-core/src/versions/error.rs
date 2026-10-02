// SPDX-License-Identifier: AGPL-3.0-only
//! Why a previous-versions request was refused or could not be completed.
//!
//! Mirrors the `ValueError`s `v2.0.0:desktop/previous_versions.py` raises, word
//! for word, plus the private-storage failures of saving the snapshot
//! sources (`crate::private_storage`), which name the affected path as
//! Python's `OSError` does.

use std::io;
use std::path::PathBuf;

use crate::location::LocationError;
use crate::private_storage::{StorageError, StorageRefusal};

/// Why a previous-versions request was refused or could not be completed.
/// `Display` is the user-facing message.
#[derive(Debug, thiserror::Error)]
pub enum VersionsError {
    /// A location or snapshot name the location rules refuse, for example
    /// one with credentials or control characters.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// The location is inside a snapshot or backup folder (PROP-024).
    #[error("{}", crate::i18n::gettext("Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first."))]
    ReadOnly,
    /// "Restore a copy" was given a destination inside a snapshot or backup
    /// folder (PROP-025).
    #[error(
        "{}",
        crate::i18n::gettext("Choose a folder outside the snapshot collection.")
    )]
    RestoreIntoSnapshot,
    /// A snapshot layout other than `direct` or `snapper`.
    #[error("{}", crate::i18n::gettext("Unknown snapshot folder layout."))]
    UnknownLayout,
    /// The snapshot folder is the live folder or contains it, so the live
    /// folder would become read-only.
    #[error(
        "{}",
        crate::i18n::gettext("The snapshot folder must not contain the current live folder.")
    )]
    SnapshotFolderContainsLiveFolder,
    /// Saving another source would exceed
    /// [`MAX_SOURCES`](super::MAX_SOURCES).
    #[error("{}", crate::i18n::gettext("At most 64 snapshot sources are supported."))]
    TooManySources,
    /// The user cancelled the lookup; not an error to show.
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
    Cancelled,
    /// Private storage refused `path` while saving the snapshot sources.
    #[error("{reason} ({})", path.display())]
    Refused {
        /// The file or directory that was refused.
        path: PathBuf,
        /// Which private-storage rule it broke.
        reason: StorageRefusal,
    },
    /// The file system refused an operation on `path` while saving the
    /// snapshot sources.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

/// A private-storage error keeps its path, reason and message.
impl From<StorageError> for VersionsError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Refused { path, reason } => Self::Refused { path, reason },
            StorageError::Io { path, error } => Self::Io { path, error },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::MAX_SOURCES;

    #[test]
    fn the_source_limit_message_names_the_limit() {
        let message = VersionsError::TooManySources.to_string();

        assert!(message.contains(&MAX_SOURCES.to_string()), "{message}");
    }
}
