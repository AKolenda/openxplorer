// SPDX-License-Identifier: AGPL-3.0-only
//! Drives the kernel mounted read-only, and why.
//!
//! The usual case on a dual-boot computer: Windows was hibernated, or shut
//! down with Fast startup (which hibernates the kernel), and left its NTFS
//! drive marked as in use. Linux then mounts it read-only to keep its
//! files safe, and every change fails with a bare "Read-only file system".
//! The window asks GIO's `filesystem::readonly` and `filesystem::type` for
//! the folder it shows, turns off the commands that would change it and
//! says why; a write that still fails says so in plain words.

/// A drive mounted read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyDrive {
    /// A Windows (NTFS) drive: Windows is hibernated or used Fast startup.
    Windows,
    /// Any other drive mounted read-only.
    Other,
}

/// True for the `filesystem::type` values of an NTFS drive: the kernel's
/// `ntfs3` and `ntfs`, and `fuseblk`, the type ntfs-3g mounts show.
fn is_windows_file_system(kind: &str) -> bool {
    matches!(kind.to_ascii_lowercase().as_str(), "ntfs" | "ntfs3" | "fuseblk")
}

impl ReadOnlyDrive {
    /// The drive, if a file system with GIO's `filesystem::readonly`
    /// answer `read_only` and `filesystem::type` answer `kind` is
    /// mounted read-only.
    pub fn from_filesystem(read_only: bool, kind: Option<&str>) -> Option<Self> {
        if !read_only {
            return None;
        }
        Some(if kind.is_some_and(is_windows_file_system) {
            Self::Windows
        } else {
            Self::Other
        })
    }

    /// Why a command that would change the drive is turned off.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Windows => crate::i18n::gettext_static(
                "Windows is hibernated or used Fast startup, so this drive is read-only.",
            ),
            Self::Other => crate::i18n::gettext_static("This drive is read-only."),
        }
    }

    /// What to do about it, for the status bar's tooltip.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Windows => crate::i18n::gettext_static(
                "Windows was hibernated or shut down with Fast startup, so Linux mounted this drive read-only to keep its files safe. Start Windows and shut it down fully (hold Shift while choosing Shut down, or turn off Fast startup), then mount the drive again.",
            ),
            Self::Other => crate::i18n::gettext_static(
                "This drive is mounted read-only, so files on it cannot be created, changed or deleted.",
            ),
        }
    }
}

/// What a write that failed on a read-only drive says, in place of the
/// system's bare "Read-only file system".
pub fn read_only_failure() -> String {
    crate::i18n::gettext(
        "The drive is read-only, so nothing on it can be changed. If it is a Windows drive, Windows is hibernated or used Fast startup: shut Windows down fully, then mount the drive again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_ntfs_drive_is_blamed_on_windows() {
        for kind in ["ntfs3", "ntfs", "fuseblk", "NTFS3"] {
            assert_eq!(
                ReadOnlyDrive::from_filesystem(true, Some(kind)),
                Some(ReadOnlyDrive::Windows),
                "{kind}"
            );
        }
        for kind in [Some("ext4"), Some("vfat"), None] {
            assert_eq!(
                ReadOnlyDrive::from_filesystem(true, kind),
                Some(ReadOnlyDrive::Other)
            );
        }
        assert_eq!(ReadOnlyDrive::from_filesystem(false, Some("ntfs3")), None);
    }

    #[test]
    fn the_reasons_say_what_happened() {
        assert!(ReadOnlyDrive::Windows.reason().contains("hibernated"));
        assert!(ReadOnlyDrive::Windows.explanation().contains("Fast startup"));
        assert_eq!(ReadOnlyDrive::Other.reason(), "This drive is read-only.");
    }
}
