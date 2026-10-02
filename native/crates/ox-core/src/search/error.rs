// SPDX-License-Identifier: AGPL-3.0-only
//! The error of every search cache operation.
//!
//! Mirrors the exceptions `v2.0.0:desktop/search_index.py` and
//! `v2.0.0:desktop/index_service.py` raise. Messages that Python wrote itself are
//! kept word for word, because the Search cache settings show them; GIO,
//! location and storage failures keep the wording of the module that
//! reports them.

use std::io;
use std::path::PathBuf;

use gio::prelude::*;

use crate::entry::EntryError;
use crate::location::LocationError;
use crate::private_storage::{StorageError, StorageRefusal};

/// Why a search cache operation was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// An address the location rules refuse.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// An SMB server's share list was offered as a root (`configure` in
    /// `search_index.py`): only a shared folder holds files to index.
    #[error(
        "{}",
        crate::i18n::gettext("Open a share first. Cache a shared folder, not the server’s share list.")
    )]
    ServerList,
    /// A phone, camera or iOS device was offered as a root (`cacheSet` in
    /// `winspace.py`).
    #[error("{}", crate::i18n::gettext("Connected-device search caching is not supported. Copy files to local storage before indexing them."))]
    DeviceLocation,
    /// A scan was started for a folder that is not an enabled root.
    #[error("{}", crate::i18n::gettext("This folder is not enabled for caching."))]
    NotEnabled,
    /// The search text is longer than [`MAX_QUERY_CHARS`](super::MAX_QUERY_CHARS).
    #[error("{}", crate::i18n::gettext("Search must be at most 512 characters."))]
    QueryTooLong,
    /// A search with no words was to be saved (SRCH-038).
    #[error(
        "{}",
        crate::i18n::gettext("Type what to search for before saving the search.")
    )]
    EmptySearch,
    /// The search or scan was cancelled; not an error to show.
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
    Cancelled,
    /// A root would hold more than a million entries.
    #[error(
        "{}",
        crate::i18n::gettext(
            "One-million-entry limit reached. Select smaller roots; additional entries were not indexed."
        )
    )]
    EntryLimit,
    /// One folder holds more than a million entries.
    #[error(
        "{}",
        crate::i18n::gettext("Directory exceeds the one-million-entry safety limit.")
    )]
    FolderTooLarge,
    /// A live update found more than 10,000 new folders.
    #[error(
        "{}",
        crate::i18n::gettext("Many new directories appeared; use Refresh for a complete scan.")
    )]
    TooManyNewFolders,
    /// A folder could not be read, for example because its share is not
    /// mounted or access is denied. The message is GIO's.
    #[error(transparent)]
    Read(#[from] EntryError),
    /// Private storage refused the cache directory or a database file.
    #[error("{reason} ({})", path.display())]
    Refused {
        /// The file or directory that was refused.
        path: PathBuf,
        /// Which private-storage rule it broke.
        reason: StorageRefusal,
    },
    /// The file system refused an operation on `path`, for example because
    /// `search.sqlite3` is a symlink.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
    /// SQLite reported an error.
    #[error("{}", crate::i18n::format_message("The search cache database failed: {error}", &[("error", &.0.to_string())]))]
    Database(#[from] rusqlite::Error),
    /// The index service could not start its worker thread.
    #[error("{}", crate::i18n::format_message("The search index could not start: {error}", &[("error", &.0.to_string())]))]
    WorkerStart(io::Error),
}

impl SearchError {
    /// Whether the operation stopped because it was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled | Self::Read(EntryError::Cancelled))
    }

    /// Whether a folder read found no folder at its location: the item was
    /// deleted or renamed, or is a file.
    pub(crate) fn is_missing_folder(&self) -> bool {
        matches!(
            self,
            Self::Read(EntryError::NotFound(_) | EntryError::NotDirectory(_))
        )
    }
}

/// A private-storage error keeps its path, reason and message.
impl From<StorageError> for SearchError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Refused { path, reason } => Self::Refused { path, reason },
            StorageError::Io { path, error } => Self::Io { path, error },
        }
    }
}

/// Stops between two steps once `cancellable` was cancelled, as
/// `Cancellation.check` does in Python.
///
/// # Errors
///
/// [`SearchError::Cancelled`] once `cancellable` was cancelled.
pub(crate) fn check_cancelled(cancellable: &gio::Cancellable) -> Result<(), SearchError> {
    if cancellable.is_cancelled() {
        return Err(SearchError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rustix::io::Errno;
    use std::path::Path;

    use super::*;

    #[test]
    fn a_storage_error_keeps_its_message() {
        let path = Path::new("/cache/winspace/search.sqlite3");
        let storage_errors = [
            StorageError::refused(path, StorageRefusal::NotPrivateFile),
            StorageError::io(path, io::Error::from(Errno::LOOP)),
        ];
        for storage_error in storage_errors {
            let message = storage_error.to_string();

            let search_error = SearchError::from(storage_error);

            assert_eq!(search_error.to_string(), message);
        }
    }

    #[test]
    fn a_cancelled_read_counts_as_cancelled() {
        assert!(SearchError::Cancelled.is_cancelled());
        assert!(SearchError::Read(EntryError::Cancelled).is_cancelled());
        assert!(!SearchError::NotEnabled.is_cancelled());
    }

    #[test]
    fn only_a_deleted_item_or_a_file_is_a_missing_folder() {
        let missing = [
            EntryError::NotFound("No such file or directory".to_owned()),
            EntryError::NotDirectory("Not a directory".to_owned()),
        ];
        let other = EntryError::NotMounted("Location is not mounted".to_owned());

        for error in missing {
            assert!(SearchError::Read(error).is_missing_folder());
        }
        assert!(!SearchError::Read(other).is_missing_folder());
        assert!(!SearchError::Cancelled.is_missing_folder());
    }

    #[test]
    fn checking_a_cancelled_token_stops() {
        let cancellable = gio::Cancellable::new();
        assert!(check_cancelled(&cancellable).is_ok());

        cancellable.cancel();

        assert!(matches!(
            check_cancelled(&cancellable),
            Err(SearchError::Cancelled)
        ));
    }
}
