// SPDX-License-Identifier: AGPL-3.0-only
//! The error of every settings change and private-storage check.
//!
//! Mirrors the two exceptions `desktop/core.py` and
//! `desktop/private_storage.py` raise: `ValueError` for a request or stored
//! data that fails validation, and `OSError` for a file-system refusal,
//! which names the path it happened on.

use std::io;
use std::path::{Path, PathBuf};

use crate::location::LocationError;

/// Why a settings change or a private-storage check was refused.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The request or the stored data failed validation (Python's
    /// `ValueError`). The message is user-facing.
    #[error("{0}")]
    Invalid(String),
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

impl From<serde_json::Error> for SettingsError {
    fn from(error: serde_json::Error) -> Self {
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
