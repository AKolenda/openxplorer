// SPDX-License-Identifier: AGPL-3.0-only
//! Windows' permission names over Linux permission bits, for the Security
//! tab.
//!
//! Linux has three entries per item, the owner, the group and everyone
//! else, each with read, write and execute; there are no deny entries and
//! no inheritance. Windows' names are read from those bits and never
//! invent more:
//!
//! | Name | Bits of the entry |
//! |---|---|
//! | Full control | `rwx` |
//! | Modify | `rw`, and `x` for a folder, which must be entered to change it |
//! | Read & execute | `r-x` |
//! | List folder contents (folders) | `r-x` |
//! | Read | `r--` |
//! | Write | `-w-` |
//! | Special permissions | set-user-ID for the owner, set-group-ID for the group, sticky for others |
//!
//! Ticking a name in Edit... gives every bit it needs; clearing it takes
//! away the one bit that tells it from the name below it (Full control
//! loses execute on a file and write on a folder, Modify loses write,
//! Read & execute and List folder contents lose execute, Read loses read,
//! Write loses write). Special permissions are changed in Advanced.

use ox_core::permissions::PermissionClass;

/// Set-user-ID, set-group-ID and sticky.
const SET_UID: u32 = 0o4000;
const SET_GID: u32 = 0o2000;
const STICKY: u32 = 0o1000;

/// A permission as Windows names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    FullControl,
    Modify,
    ReadAndExecute,
    ListFolderContents,
    Read,
    Write,
    Special,
}

impl Level {
    /// The names a file or a folder lists, in Windows' order.
    pub(super) fn listed(is_folder: bool) -> Vec<Self> {
        let mut levels = vec![Self::FullControl, Self::Modify, Self::ReadAndExecute];
        if is_folder {
            levels.push(Self::ListFolderContents);
        }
        levels.extend([Self::Read, Self::Write, Self::Special]);
        levels
    }

    /// The name shown.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::FullControl => ox_core::i18n::gettext_static("Full control"),
            Self::Modify => ox_core::i18n::gettext_static("Modify"),
            Self::ReadAndExecute => ox_core::i18n::gettext_static("Read & execute"),
            Self::ListFolderContents => ox_core::i18n::gettext_static("List folder contents"),
            Self::Read => ox_core::i18n::gettext_static("Read"),
            Self::Write => ox_core::i18n::gettext_static("Write"),
            Self::Special => ox_core::i18n::gettext_static("Special permissions"),
        }
    }

    /// The `rwx` bits the name needs; none for Special permissions.
    fn bits(self, is_folder: bool) -> u32 {
        match self {
            Self::FullControl => 0o7,
            Self::Modify if is_folder => 0o7,
            Self::Modify => 0o6,
            Self::ReadAndExecute | Self::ListFolderContents => 0o5,
            Self::Read => 0o4,
            Self::Write => 0o2,
            Self::Special => 0,
        }
    }

    /// The bit clearing the name takes away.
    fn key_bit(self, is_folder: bool) -> u32 {
        match self {
            Self::FullControl if is_folder => 0o2,
            Self::FullControl | Self::ReadAndExecute | Self::ListFolderContents => 0o1,
            Self::Modify | Self::Write => 0o2,
            Self::Read => 0o4,
            Self::Special => 0,
        }
    }

    /// Whether `class` is allowed this under `mode`.
    pub(super) fn is_allowed(self, mode: u32, class: PermissionClass, is_folder: bool) -> bool {
        if self == Self::Special {
            return mode & special_bit(class) != 0;
        }
        let bits = self.bits(is_folder);
        class_bits(mode, class) & bits == bits
    }

    /// `mode` with this name ticked or cleared for `class`; Special
    /// permissions are left as they are.
    pub(super) fn set(self, mode: u32, class: PermissionClass, is_folder: bool, allowed: bool) -> u32 {
        let shift = shift(class);
        if allowed {
            mode | (self.bits(is_folder) << shift)
        } else {
            mode & !(self.key_bit(is_folder) << shift)
        }
    }
}

/// How far `class`'s `rwx` bits are shifted.
const fn shift(class: PermissionClass) -> u32 {
    match class {
        PermissionClass::Owner => 6,
        PermissionClass::Group => 3,
        PermissionClass::Others => 0,
    }
}

/// `class`'s `rwx` bits in `mode`.
fn class_bits(mode: u32, class: PermissionClass) -> u32 {
    (mode >> shift(class)) & 0o7
}

/// The special bit Special permissions shows for `class`.
pub(super) const fn special_bit(class: PermissionClass) -> u32 {
    match class {
        PermissionClass::Owner => SET_UID,
        PermissionClass::Group => SET_GID,
        PermissionClass::Others => STICKY,
    }
}

/// The highest name `class` is allowed under `mode`, for Advanced's
/// permission entries; "None" without any.
pub(super) fn access_summary(mode: u32, class: PermissionClass, is_folder: bool) -> &'static str {
    Level::listed(is_folder)
        .into_iter()
        .filter(|level| *level != Level::Special)
        .find(|level| level.is_allowed(mode, class, is_folder))
        .map_or_else(|| ox_core::i18n::gettext_static("None"), Level::label)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: PermissionClass = PermissionClass::Owner;
    const GROUP: PermissionClass = PermissionClass::Group;
    const OTHERS: PermissionClass = PermissionClass::Others;

    fn allowed(mode: u32, class: PermissionClass, is_folder: bool) -> Vec<&'static str> {
        Level::listed(is_folder)
            .into_iter()
            .filter(|level| level.is_allowed(mode, class, is_folder))
            .map(Level::label)
            .collect()
    }

    /// The names read the bits and invent nothing.
    ///
    /// parity: PROP-007
    #[test]
    fn windows_names_read_the_linux_bits() {
        assert_eq!(allowed(0o644, OWNER, false), ["Modify", "Read", "Write"]);
        assert_eq!(allowed(0o644, GROUP, false), ["Read"]);
        assert_eq!(
            allowed(0o755, OWNER, true),
            [
                "Full control",
                "Modify",
                "Read & execute",
                "List folder contents",
                "Read",
                "Write"
            ]
        );
        assert_eq!(
            allowed(0o755, OTHERS, true),
            ["Read & execute", "List folder contents", "Read"]
        );
        assert_eq!(allowed(0o1777, OTHERS, true).last(), Some(&"Special permissions"));
        assert!(allowed(0o640, OTHERS, false).is_empty());
        assert_eq!(access_summary(0o640, GROUP, false), "Read");
        assert_eq!(access_summary(0o640, OTHERS, false), "None");
    }

    /// Ticking gives every bit a name needs; clearing takes one away.
    ///
    /// parity: PROP-007
    #[test]
    fn ticking_and_clearing_a_name_change_its_bits() {
        assert_eq!(Level::FullControl.set(0o640, GROUP, false, true), 0o670);
        assert_eq!(Level::FullControl.set(0o770, GROUP, false, false), 0o760);
        assert_eq!(Level::Modify.set(0o644, OWNER, false, false), 0o444);
        assert_eq!(Level::Read.set(0o640, OTHERS, false, true), 0o644);
        assert_eq!(Level::ReadAndExecute.set(0o755, OTHERS, true, false), 0o754);
        assert_eq!(Level::Special.set(0o4755, OWNER, false, false), 0o4755);
    }
}
