// SPDX-License-Identifier: AGPL-3.0-only
//! Why a file operation was refused or failed.
//!
//! Ports `error_payload` in `desktop/gio_backend.py` for the operations of
//! this module: every failure carries the message the user sees and maps to
//! the code the Python bridge reported (OPS-037). GIO, transfer, location
//! and local I/O errors convert with `?`, so each operation reports the
//! first failure in the wording of the layer that refused it.

use crate::location::LocationError;
use crate::transfer::TransferError;

/// Why a file operation was refused or failed. `Display` is the message
/// shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpsError {
    /// The user cancelled; not an error to show.
    #[error("Operation cancelled.")]
    Cancelled,
    /// The share or device is not mounted. Mount it and ask again once, as
    /// the Python bridge does for its read-only requests.
    #[error("{0}")]
    NotMounted(String),
    /// Nothing exists at the location.
    #[error("{0}")]
    NotFound(String),
    /// The backend refused access.
    #[error("{0}")]
    PermissionDenied(String),
    /// The name is taken; nothing was overwritten.
    #[error("{0}")]
    Exists(String),
    /// The backend cannot do this here, for example Trash on a share.
    #[error("{0}")]
    NotSupported(String),
    /// A folder was expected.
    #[error("{0}")]
    NotDirectory(String),
    /// A refusal in the app's wording, or any other failure.
    #[error("{0}")]
    Failed(String),
}

impl OpsError {
    /// A refusal or failure with a user-facing message.
    pub(crate) fn failed(message: impl Into<String>) -> Self {
        OpsError::Failed(message.into())
    }

    /// The code the Python bridge reported for this error (`not-mounted`,
    /// `exists`, ...); refusals are plain `error`s there too.
    pub fn code(&self) -> &'static str {
        match self {
            OpsError::Cancelled => "cancelled",
            OpsError::NotMounted(_) => "not-mounted",
            OpsError::NotFound(_) => "not-found",
            OpsError::PermissionDenied(_) => "permission-denied",
            OpsError::Exists(_) => "exists",
            OpsError::NotSupported(_) => "not-supported",
            OpsError::NotDirectory(_) => "not-directory",
            OpsError::Failed(_) => "error",
        }
    }

    /// True when mounting the location may make the request succeed.
    pub fn needs_mount(&self) -> bool {
        matches!(self, OpsError::NotMounted(_))
    }

    /// True for the user's own cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, OpsError::Cancelled)
    }
}

/// Keeps the meaning the transfer engine gave a failure. A replacement the
/// backend cannot do in one step is "not supported" to the user, and a
/// failure that needs manual recovery keeps its message.
impl From<TransferError> for OpsError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::Cancelled => OpsError::Cancelled,
            TransferError::NotFound(message) => OpsError::NotFound(message),
            TransferError::Exists(message) => OpsError::Exists(message),
            TransferError::NotMounted(message) => OpsError::NotMounted(message),
            TransferError::NotSupported(message) | TransferError::ReplaceUnsupported(message) => {
                OpsError::NotSupported(message)
            }
            TransferError::RecoveryRequired(message) | TransferError::Failed(message) => {
                OpsError::Failed(message)
            }
        }
    }
}

/// Sorts a GIO error the way `error_payload` does. Unlike the transfer
/// engine, which only trusts its own cancellation token, these operations
/// pass the user's cancellable to every GIO call, so `CANCELLED` is the
/// user's cancellation.
impl From<glib::Error> for OpsError {
    fn from(error: glib::Error) -> Self {
        let message = error.message().to_owned();
        match error.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::Cancelled) => OpsError::Cancelled,
            Some(gio::IOErrorEnum::NotMounted) => OpsError::NotMounted(message),
            Some(gio::IOErrorEnum::NotFound) => OpsError::NotFound(message),
            Some(gio::IOErrorEnum::PermissionDenied) => OpsError::PermissionDenied(message),
            Some(gio::IOErrorEnum::Exists) => OpsError::Exists(message),
            Some(gio::IOErrorEnum::NotSupported) => OpsError::NotSupported(message),
            Some(gio::IOErrorEnum::NotDirectory) => OpsError::NotDirectory(message),
            _ => OpsError::Failed(message),
        }
    }
}

/// A location or name the `location` module refused, with its message
/// (Python raises the same `ValueError`).
impl From<LocationError> for OpsError {
    fn from(error: LocationError) -> Self {
        OpsError::Failed(error.into_message())
    }
}

/// Local I/O errors keep the meanings `FileNotFoundError`,
/// `FileExistsError` and `PermissionError` have in Python.
impl From<std::io::Error> for OpsError {
    fn from(error: std::io::Error) -> Self {
        let message = error.to_string();
        match error.kind() {
            std::io::ErrorKind::NotFound => OpsError::NotFound(message),
            std::io::ErrorKind::AlreadyExists => OpsError::Exists(message),
            std::io::ErrorKind::PermissionDenied => OpsError::PermissionDenied(message),
            _ => OpsError::Failed(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One GIO error code and the code the bridge reported for it.
    struct GioCase {
        kind: gio::IOErrorEnum,
        code: &'static str,
    }

    /// parity: OPS-037
    #[test]
    fn gio_errors_keep_the_bridge_codes() {
        let cases = [
            GioCase {
                kind: gio::IOErrorEnum::NotMounted,
                code: "not-mounted",
            },
            GioCase {
                kind: gio::IOErrorEnum::Cancelled,
                code: "cancelled",
            },
            GioCase {
                kind: gio::IOErrorEnum::NotFound,
                code: "not-found",
            },
            GioCase {
                kind: gio::IOErrorEnum::PermissionDenied,
                code: "permission-denied",
            },
            GioCase {
                kind: gio::IOErrorEnum::Exists,
                code: "exists",
            },
            GioCase {
                kind: gio::IOErrorEnum::NotSupported,
                code: "not-supported",
            },
            GioCase {
                kind: gio::IOErrorEnum::NotDirectory,
                code: "not-directory",
            },
            GioCase {
                kind: gio::IOErrorEnum::Failed,
                code: "error",
            },
        ];
        for case in cases {
            let error = OpsError::from(glib::Error::new(case.kind, "message"));

            assert_eq!(error.code(), case.code, "{:?}", case.kind);
        }
    }

    /// One transfer error and the operation error it becomes.
    struct TransferCase {
        error: TransferError,
        expected: OpsError,
    }

    /// parity: OPS-037
    #[test]
    fn transfer_errors_keep_their_meaning_and_message() {
        let cases = [
            TransferCase {
                error: TransferError::Cancelled,
                expected: OpsError::Cancelled,
            },
            TransferCase {
                error: TransferError::Exists("taken".into()),
                expected: OpsError::Exists("taken".into()),
            },
            TransferCase {
                error: TransferError::NotMounted("mount".into()),
                expected: OpsError::NotMounted("mount".into()),
            },
            TransferCase {
                error: TransferError::ReplaceUnsupported("one step".into()),
                expected: OpsError::NotSupported("one step".into()),
            },
            TransferCase {
                error: TransferError::RecoveryRequired("backup at x".into()),
                expected: OpsError::Failed("backup at x".into()),
            },
        ];
        for case in cases {
            assert_eq!(OpsError::from(case.error), case.expected);
        }
    }

    /// parity: OPS-037
    #[test]
    fn only_not_mounted_asks_for_a_mount() {
        assert!(OpsError::NotMounted("m".into()).needs_mount());
        assert!(!OpsError::NotFound("m".into()).needs_mount());
        assert!(OpsError::Cancelled.is_cancelled());
        assert_eq!(OpsError::Cancelled.to_string(), "Operation cancelled.");
    }

    #[test]
    fn local_io_errors_keep_their_meaning() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let taken = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);

        assert_eq!(OpsError::from(missing).code(), "not-found");
        assert_eq!(OpsError::from(taken).code(), "exists");
        assert_eq!(OpsError::from(denied).code(), "permission-denied");
    }

    #[test]
    fn refused_locations_keep_their_message() {
        let error = OpsError::from(LocationError::new("Invalid location."));

        assert_eq!(error, OpsError::Failed("Invalid location.".into()));
        assert_eq!(error.code(), "error");
    }
}
