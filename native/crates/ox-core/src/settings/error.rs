// SPDX-License-Identifier: AGPL-3.0-only
//! The error of every settings change.
//!
//! Mirrors the exceptions `desktop/core.py` raises, including those of the
//! private storage it relies on (`desktop/private_storage.py`, ported in
//! `crate::private_storage`). Python raises `ValueError` for a request or
//! stored data that fails validation, for a location or label it refuses,
//! and for a file private storage refuses; here they are
//! [`SettingsError::Invalid`], [`SettingsError::Location`] and
//! [`SettingsError::Storage`], which also carries Python's `OSError`. A
//! storage error names the refused path, so the settings warning can say
//! which file was refused.

use crate::location::LocationError;
use crate::private_storage::StorageError;

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
    /// Private storage refused a file (Python's `ValueError` in
    /// `private_storage.py`): it is not private, or its contents cannot be
    /// read safely; or the file system refused an operation on it (Python's
    /// `OSError`), for example because `settings.lock` is a symlink.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl SettingsError {
    /// A validation error with a user-facing message.
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Whether this is a missing file or directory, which Python reports as
    /// `FileNotFoundError` and the settings reader treats as a first start.
    pub(super) fn is_not_found(&self) -> bool {
        matches!(self, Self::Storage(error) if error.is_not_found())
    }
}

#[cfg(test)]
mod tests {
    use rustix::io::Errno;
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::private_storage::StorageRefusal;

    /// The settings warning shows either type, so both must say the same.
    #[test]
    fn a_storage_error_keeps_its_message_as_a_settings_error() {
        let path = Path::new("/state/settings.json");
        let storage_errors = [
            StorageError::refused(path, StorageRefusal::NotPrivateFile),
            StorageError::io(path, io::Error::from(Errno::LOOP)),
        ];
        for storage_error in storage_errors {
            let message = storage_error.to_string();

            let settings_error = SettingsError::from(storage_error);

            assert_eq!(settings_error.to_string(), message);
        }
    }
}
