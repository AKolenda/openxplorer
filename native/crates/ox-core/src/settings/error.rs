// SPDX-License-Identifier: AGPL-3.0-only
//! The error of every settings change and private-storage check.
//!
//! Mirrors the exceptions `desktop/core.py` and `desktop/private_storage.py`
//! raise. Python raises `ValueError` both for a request or stored data that
//! fails validation and for a file private storage refuses; here they are
//! [`SettingsError::Invalid`] and [`SettingsError::Refused`]. A refusal
//! names the refused path, as Python's `OSError` ([`SettingsError::Io`])
//! does, so the settings warning can say which file was refused.

use std::io;
use std::path::{Path, PathBuf};

use crate::location::LocationError;

/// Why a settings change or a private-storage check was refused.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The request or the stored data failed validation (Python's
    /// `ValueError` in `core.py`). The message is user-facing.
    #[error("{0}")]
    Invalid(String),
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

/// The private-storage rule a file or directory broke, in the words of
/// `desktop/private_storage.py`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StorageRefusal {
    /// The application's own directory belongs to another user.
    #[error("Application state directory must be owned by this user.")]
    ForeignDirectory,
    /// A hard link, FIFO, device, or a file that belongs to another user.
    #[error("Application state must be an owned regular file, not a link or device.")]
    NotPrivateFile,
    /// Larger than the read limit. The message names 4 MiB whatever the
    /// limit, as `private_text` in `private_storage.py` does.
    #[error("Settings file exceeds the 4 MiB safety limit.")]
    TooLarge,
    /// Not UTF-8, which Python's `decode('utf-8')` refuses.
    #[error("The settings file is not valid UTF-8 text.")]
    NotText,
}

impl SettingsError {
    /// A validation error with a user-facing message.
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// A private-storage refusal of `path`.
    pub(super) fn refused(path: &Path, reason: StorageRefusal) -> Self {
        Self::Refused {
            path: path.to_path_buf(),
            reason,
        }
    }

    /// A file-system error on `path`.
    pub(super) fn io(path: &Path, error: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            error,
        }
    }

    /// Whether this is a missing file or directory, which Python reports as
    /// `FileNotFoundError` and most callers treat as "nothing there yet".
    pub(super) fn is_not_found(&self) -> bool {
        matches!(self, Self::Io { error, .. } if error.kind() == io::ErrorKind::NotFound)
    }
}

impl From<LocationError> for SettingsError {
    fn from(error: LocationError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// Adds the affected path to an I/O error, as Python's `OSError` does.
pub(super) trait WithPath<T> {
    /// This result, with an error turned into [`SettingsError::Io`] on
    /// `path`.
    fn with_path(self, path: &Path) -> Result<T, SettingsError>;
}

impl<T> WithPath<T> for io::Result<T> {
    fn with_path(self, path: &Path) -> Result<T, SettingsError> {
        self.map_err(|error| SettingsError::io(path, error))
    }
}
