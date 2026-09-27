// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer error type and how backend errors map onto it.
//!
//! Ports the error interpretation of `desktop/operations.py` (`Cancelled`,
//! `ReplaceUnsupported`, `is_not_found`) and the error codes of
//! `error_payload` in `desktop/gio_backend.py` that transfers rely on.

use crate::location::LocationError;

/// A transfer failure. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    /// The user cancelled: returned by [`Cancellation::check`]. The engine
    /// reports a cancelled item as `cancelled`, not as an error.
    ///
    /// [`Cancellation::check`]: super::Cancellation::check
    #[error("Operation cancelled.")]
    Cancelled,
    /// A definite "does not exist" (`G_IO_ERROR_NOT_FOUND`, `ENOENT`).
    #[error("{0}")]
    NotFound(String),
    /// The name is taken; nothing was overwritten.
    #[error("{0}")]
    Exists(String),
    /// The location belongs to a share or device that is not mounted
    /// (`G_IO_ERROR_NOT_MOUNTED`). The caller mounts it and asks again, as
    /// the Python app's `mount_retry` does.
    #[error("{0}")]
    NotMounted(String),
    /// The backend cannot do this here (for example a cross-filesystem move).
    #[error("{0}")]
    NotSupported(String),
    /// The backend cannot replace in one step; the engine then uses
    /// reversible renames instead.
    #[error("{0}")]
    ReplaceUnsupported(String),
    /// A recovery location or permission problem the user must see even
    /// when cancellation arrived during the commit.
    #[error("{0}")]
    RecoveryRequired(String),
    /// Any other backend or validation failure.
    #[error("{0}")]
    Failed(String),
}

impl TransferError {
    /// A validation or safety failure with a user-facing message.
    pub fn failed(message: impl Into<String>) -> Self {
        TransferError::Failed(message.into())
    }

    /// A definite "does not exist", as opposed to "could not check".
    pub fn is_not_found(&self) -> bool {
        matches!(self, TransferError::NotFound(_))
    }

    /// True for a user cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, TransferError::Cancelled)
    }
}

/// Maps GLib errors the way `desktop/gio_backend.py` and
/// `desktop/operations.py` interpret them: `NOT_FOUND` is a definite absence
/// (see `is_not_found` in `operations.py`), `EXISTS` a taken name,
/// `NOT_MOUNTED` a location to mount first and `NOT_SUPPORTED` an
/// unsupported operation. Operation-specific meanings (for example
/// `WOULD_RECURSE` on a move) are mapped by the caller before falling back
/// to this conversion.
///
/// `G_IO_ERROR_CANCELLED` deliberately stays an ordinary failure with the
/// backend's message. A device can report "Operation was cancelled" for a
/// call the user never cancelled; like the Python engine, the transfer
/// engine counts an item as cancelled only when the user's
/// [`Cancellation`](super::Cancellation) is cancelled.
impl From<glib::Error> for TransferError {
    fn from(error: glib::Error) -> Self {
        let message = error.message().to_string();
        match error.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotFound) => TransferError::NotFound(message),
            Some(gio::IOErrorEnum::Exists) => TransferError::Exists(message),
            Some(gio::IOErrorEnum::NotMounted) => TransferError::NotMounted(message),
            Some(gio::IOErrorEnum::NotSupported) => TransferError::NotSupported(message),
            _ => TransferError::Failed(message),
        }
    }
}

/// Maps local I/O errors: `ENOENT` is a definite absence and `EEXIST` a
/// taken name, exactly like `FileNotFoundError`/`FileExistsError` in Python.
impl From<std::io::Error> for TransferError {
    fn from(error: std::io::Error) -> Self {
        let message = error.to_string();
        match error.kind() {
            std::io::ErrorKind::NotFound => TransferError::NotFound(message),
            std::io::ErrorKind::AlreadyExists => TransferError::Exists(message),
            _ => TransferError::Failed(message),
        }
    }
}

/// Preserves missing-file and name-conflict errors from descriptor-relative I/O.
impl From<rustix::io::Errno> for TransferError {
    fn from(error: rustix::io::Errno) -> Self {
        std::io::Error::from(error).into()
    }
}

/// A location or name the engine cannot accept, with the location module's
/// user-facing message (Python raises the same `ValueError` inside the item).
impl From<LocationError> for TransferError {
    fn from(error: LocationError) -> Self {
        TransferError::Failed(error.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One GLib error code and the transfer error it must become.
    struct GlibCase {
        code: gio::IOErrorEnum,
        expected: TransferError,
    }

    #[test]
    fn glib_errors_keep_their_meaning() {
        let cases = [
            GlibCase {
                code: gio::IOErrorEnum::Cancelled,
                expected: TransferError::Failed("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::NotFound,
                expected: TransferError::NotFound("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::Exists,
                expected: TransferError::Exists("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::NotMounted,
                expected: TransferError::NotMounted("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::NotSupported,
                expected: TransferError::NotSupported("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::PermissionDenied,
                expected: TransferError::Failed("m".into()),
            },
        ];
        for case in cases {
            let error = glib::Error::new(case.code, "m");
            assert_eq!(TransferError::from(error), case.expected);
        }
    }

    #[test]
    fn io_errors_keep_their_meaning() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(TransferError::from(missing).is_not_found());
        let taken = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert!(matches!(TransferError::from(taken), TransferError::Exists(_)));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(TransferError::from(denied), TransferError::Failed(_)));
    }

    #[test]
    fn location_errors_keep_their_message() {
        let error = LocationError::new("Invalid location.");
        assert_eq!(
            TransferError::from(error),
            TransferError::failed("Invalid location.")
        );
    }
}
