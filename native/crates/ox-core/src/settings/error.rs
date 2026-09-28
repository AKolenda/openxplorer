// SPDX-License-Identifier: AGPL-3.0-only
//! The error of every settings change.
//!
//! Mirrors the exceptions `desktop/core.py` raises, including those of the
//! private storage it relies on (`desktop/private_storage.py`, ported in
//! `crate::private_storage`). Python raises `ValueError` for a request or
//! stored data that fails validation, for a location or label it refuses,
//! and for a file private storage refuses; here they are
//! [`SettingsError::Invalid`], [`SettingsError::Location`] and
//! [`SettingsError::Refused`]. A refusal names the refused path, as
//! Python's `OSError` ([`SettingsError::Io`]) does, so the settings warning
//! can say which file was refused.

use std::io;
use std::path::PathBuf;

use crate::location::LocationError;
use crate::private_storage::{StorageError, StorageRefusal};

/// Why a settings change was refused or could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The request or the stored data failed validation (Python's
    /// `ValueError` in `core.py`). The message is user-facing.
    #[error("{0}")]
    Invalid(String),
    /// A location or label the location rules refuse, for example one with
    /// credentials, an unsupported scheme or control characters. The
    /// message is user-facing.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// Private storage refused `path` (Python's `ValueError` in
    /// `private_storage.py`): it is not private, or its contents cannot be
    /// read safely.
    #[error("{reason} ({})", path.display())]
    Refused {
        /// The file or directory that was refused.
        path: PathBuf,
        /// Which private-storage rule it broke.
        reason: StorageRefusal,
    },
    /// The file system refused an operation on `path` (Python's `OSError`),
    /// for example because `settings.lock` is a symlink.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

impl SettingsError {
    /// A validation error with a user-facing message.
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Whether this is a missing file or directory, which Python reports as
    /// `FileNotFoundError` and the settings reader treats as a first start.
    pub(super) fn is_not_found(&self) -> bool {
        matches!(self, Self::Io { error, .. } if error.kind() == io::ErrorKind::NotFound)
    }
}

/// A private-storage error keeps its path, reason and message.
impl From<StorageError> for SettingsError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Refused { path, reason } => Self::Refused { path, reason },
            StorageError::Io { path, error } => Self::Io { path, error },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// The settings warning shows either type, so both must say the same.
    #[test]
    fn a_storage_error_keeps_its_message_as_a_settings_error() {
        let path = Path::new("/state/settings.json");
        let storage_errors = [
            StorageError::refused(path, StorageRefusal::NotPrivateFile),
            StorageError::io(path, io::Error::from_raw_os_error(libc::ELOOP)),
        ];
        for storage_error in storage_errors {
            let message = storage_error.to_string();

            let settings_error = SettingsError::from(storage_error);

            assert_eq!(settings_error.to_string(), message);
        }
    }
}
