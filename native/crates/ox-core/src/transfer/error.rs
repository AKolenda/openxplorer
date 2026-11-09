// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer error type and how backend errors map onto it.
//!
//! Ports the error interpretation of `v2.0.0:desktop/operations.py` (`Cancelled`,
//! `ReplaceUnsupported`, `is_not_found`) and the error codes of
//! `error_payload` in `v2.0.0:desktop/gio_backend.py` that transfers rely on.

use crate::location::LocationError;

/// A transfer failure. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    /// The user cancelled: returned by [`Cancellation::check`]. The engine
    /// reports a cancelled item as `cancelled`, not as an error.
    ///
    /// [`Cancellation::check`]: super::Cancellation::check
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
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
    /// The item, or the folder it goes into, may not be read or written
    /// (`G_IO_ERROR_PERMISSION_DENIED`, `EACCES`, `EPERM`).
    #[error("{0}")]
    PermissionDenied(String),
    /// The destination's drive is full (`G_IO_ERROR_NO_SPACE`, `ENOSPC`).
    #[error("{0}")]
    NoSpace(String),
    /// Something else holds the item (`G_IO_ERROR_BUSY`, `EBUSY`,
    /// `ETXTBSY`).
    #[error("{0}")]
    Busy(String),
    /// A folder would go into itself or a folder inside it (XFER-016):
    /// refused before anything changes, and never worth trying again.
    #[error("{0}")]
    InsideSource(String),
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

/// Maps `GLib` errors the way `v2.0.0:desktop/gio_backend.py` and
/// `v2.0.0:desktop/operations.py` interpret them: `NOT_FOUND` is a definite absence
/// (see `is_not_found` in `operations.py`), `EXISTS` a taken name,
/// `NOT_MOUNTED` a location to mount first and `NOT_SUPPORTED` an
/// unsupported operation. `PERMISSION_DENIED`, `NO_SPACE` and `BUSY` keep
/// their meaning for the questions about a failed item (OPS-047). Operation-specific meanings (for example
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
        let message = error.message().to_owned();
        match error.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotFound) => Self::NotFound(message),
            Some(gio::IOErrorEnum::Exists) => Self::Exists(message),
            Some(gio::IOErrorEnum::NotMounted) => Self::NotMounted(message),
            Some(gio::IOErrorEnum::NotSupported) => Self::NotSupported(message),
            Some(gio::IOErrorEnum::PermissionDenied) => Self::PermissionDenied(message),
            Some(gio::IOErrorEnum::NoSpace) => Self::NoSpace(message),
            Some(gio::IOErrorEnum::Busy) => Self::Busy(message),
            _ => Self::Failed(message),
        }
    }
}

/// Maps local I/O errors: `ENOENT` is a definite absence and `EEXIST` a
/// taken name, exactly like `FileNotFoundError`/`FileExistsError` in Python;
/// a denied permission, a full drive and a busy item keep their meaning as
/// GIO's do.
impl From<std::io::Error> for TransferError {
    fn from(error: std::io::Error) -> Self {
        let message = error.to_string();
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound(message),
            std::io::ErrorKind::AlreadyExists => Self::Exists(message),
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied(message),
            std::io::ErrorKind::StorageFull => Self::NoSpace(message),
            std::io::ErrorKind::ResourceBusy | std::io::ErrorKind::ExecutableFileBusy => Self::Busy(message),
            _ => Self::Failed(message),
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
        Self::Failed(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `GLib` error code and the transfer error it must become.
    struct GlibCase {
        code: gio::IOErrorEnum,
        expected: TransferError,
    }

    /// parity: OPS-037
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
                expected: TransferError::PermissionDenied("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::NoSpace,
                expected: TransferError::NoSpace("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::Busy,
                expected: TransferError::Busy("m".into()),
            },
            GlibCase {
                code: gio::IOErrorEnum::Failed,
                expected: TransferError::Failed("m".into()),
            },
        ];
        for case in cases {
            let error = glib::Error::new(case.code, "m");
            assert_eq!(TransferError::from(error), case.expected);
        }
    }

    /// parity: OPS-037
    #[test]
    fn io_errors_keep_their_meaning() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(TransferError::from(missing).is_not_found());
        let taken = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert!(matches!(TransferError::from(taken), TransferError::Exists(_)));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(
            TransferError::from(denied),
            TransferError::PermissionDenied(_)
        ));
        let full = std::io::Error::from_raw_os_error(28);
        assert!(matches!(TransferError::from(full), TransferError::NoSpace(_)));
        let busy = std::io::Error::from_raw_os_error(16);
        assert!(matches!(TransferError::from(busy), TransferError::Busy(_)));
    }

    #[test]
    fn location_errors_keep_their_message() {
        let error = LocationError::new("Invalid location.");
        assert_eq!(
            TransferError::from(error),
            TransferError::failed("Invalid location.")
        );
    }

    /// A write to a drive mounted read-only shows the system's own error,
    /// from GIO and from local I/O alike, as Windows Explorer does.
    ///
    /// parity: DEV-015
    #[test]
    fn a_read_only_file_system_shows_the_system_error() {
        let from_gio = TransferError::from(glib::Error::new(
            gio::IOErrorEnum::ReadOnly,
            "Read-only file system",
        ));
        let read_only = std::io::Error::from_raw_os_error(30);
        let expected = read_only.to_string();
        let from_io = TransferError::from(read_only);
        assert_eq!(from_gio.to_string(), "Read-only file system");
        assert_eq!(from_io.to_string(), expected);
    }
}
