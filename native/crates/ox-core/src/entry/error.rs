// SPDX-License-Identifier: AGPL-3.0-only
//! Why a folder could not be listed, or an item inspected or pinned.
//!
//! Ports `error_payload` in `v2.0.0:desktop/gio_backend.py`: GIO failures are
//! sorted into the cases the interface handles differently, and each has
//! the code the Python backend reports.

use crate::location::LocationError;

/// Why a folder could not be listed, or an item inspected or pinned.
///
/// Messages come from GIO, from the `location` module's validation or from
/// the Python app's wording, and are shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// The location belongs to a volume or share that is not mounted yet;
    /// mount it (asking for credentials if needed) and try again once.
    #[error("{0}")]
    NotMounted(String),
    /// The folder exists but may not be read.
    #[error("{0}")]
    PermissionDenied(String),
    /// Nothing exists at this location.
    #[error("{0}")]
    NotFound(String),
    /// The location is a file, not a folder.
    #[error("{0}")]
    NotDirectory(String),
    /// The backend cannot list this location.
    #[error("{0}")]
    NotSupported(String),
    /// The work was cancelled; not an error to show.
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
    Cancelled,
    /// An address that is not a supported location, refused by the
    /// `location` module before GIO was asked.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A file was offered for Quick access (`verify_pin` in Python).
    #[error(
        "{}",
        crate::i18n::gettext("Only folders and network shares can be pinned to Quick access.")
    )]
    NotPinnable,
    /// Any other failure.
    #[error("{0}")]
    Failed(String),
}

impl EntryError {
    /// The code the Python backend reports for this error (`not-mounted`,
    /// `permission-denied`, ...). Validation failures are plain `error`s
    /// there too.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotMounted(_) => "not-mounted",
            Self::PermissionDenied(_) => "permission-denied",
            Self::NotFound(_) => "not-found",
            Self::NotDirectory(_) => "not-directory",
            Self::NotSupported(_) => "not-supported",
            Self::Cancelled => "cancelled",
            Self::Location(_) | Self::NotPinnable | Self::Failed(_) => "error",
        }
    }

    /// True when mounting the location may make the request succeed. The
    /// Python app mounts and retries exactly once in that case
    /// (`start_worker` and `retry_list` in `v2.0.0:desktop/winspace.py`).
    pub fn needs_mount(&self) -> bool {
        matches!(self, Self::NotMounted(_))
    }
}

impl From<glib::Error> for EntryError {
    /// Sorts a GIO error into the cases the interface handles differently.
    fn from(error: glib::Error) -> Self {
        let message = error.message().to_owned();
        match error.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotMounted) => Self::NotMounted(message),
            Some(gio::IOErrorEnum::PermissionDenied) => Self::PermissionDenied(message),
            Some(gio::IOErrorEnum::NotFound) => Self::NotFound(message),
            Some(gio::IOErrorEnum::NotDirectory) => Self::NotDirectory(message),
            Some(gio::IOErrorEnum::NotSupported) => Self::NotSupported(message),
            Some(gio::IOErrorEnum::Cancelled) => Self::Cancelled,
            _ => Self::Failed(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_error(kind: gio::IOErrorEnum) -> EntryError {
        EntryError::from(glib::Error::new(kind, "message"))
    }

    struct Case {
        kind: gio::IOErrorEnum,
        code: &'static str,
    }

    /// parity: OPS-037
    #[test]
    fn gio_errors_are_sorted_by_kind() {
        let cases = [
            Case {
                kind: gio::IOErrorEnum::NotMounted,
                code: "not-mounted",
            },
            Case {
                kind: gio::IOErrorEnum::PermissionDenied,
                code: "permission-denied",
            },
            Case {
                kind: gio::IOErrorEnum::NotFound,
                code: "not-found",
            },
            Case {
                kind: gio::IOErrorEnum::NotDirectory,
                code: "not-directory",
            },
            Case {
                kind: gio::IOErrorEnum::NotSupported,
                code: "not-supported",
            },
            Case {
                kind: gio::IOErrorEnum::Cancelled,
                code: "cancelled",
            },
            Case {
                kind: gio::IOErrorEnum::Failed,
                code: "error",
            },
        ];
        for case in cases {
            assert_eq!(io_error(case.kind).code(), case.code, "{:?}", case.kind);
        }
    }

    /// parity: OPS-037
    #[test]
    fn only_not_mounted_asks_for_a_mount() {
        assert!(io_error(gio::IOErrorEnum::NotMounted).needs_mount());
        assert!(!io_error(gio::IOErrorEnum::NotFound).needs_mount());
    }

    #[test]
    fn messages_are_shown_as_given() {
        assert_eq!(io_error(gio::IOErrorEnum::NotFound).to_string(), "message");
        assert_eq!(EntryError::Cancelled.to_string(), "Operation cancelled.");
    }

    /// parity: OPS-037
    #[test]
    fn refused_addresses_keep_the_location_error() {
        let error = EntryError::from(LocationError::new("Enter a folder location."));
        assert_eq!(
            error,
            EntryError::Location(LocationError::new("Enter a folder location."))
        );
        assert_eq!(error.to_string(), "Enter a folder location.");
        assert_eq!(error.code(), "error");
    }

    /// parity: OPS-037
    #[test]
    fn refused_pins_are_plain_errors_in_the_python_wording() {
        let error = EntryError::NotPinnable;
        assert_eq!(
            error.to_string(),
            "Only folders and network shares can be pinned to Quick access."
        );
        assert_eq!(error.code(), "error");
    }
}
