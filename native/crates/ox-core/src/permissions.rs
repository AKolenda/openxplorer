// SPDX-License-Identifier: AGPL-3.0-only
//! Editing POSIX permissions from Properties (PROP-007), as the
//! Permissions tab of Dolphin and Files does it: owner, group and others
//! each get "No Access", "Can Only View" or "Can View & Modify", a file can
//! be made executable, a folder can let only owners rename and delete its
//! content (the sticky bit), and a folder's change can reach everything
//! inside it.
//!
//! Permissions are set through GIO's `unix::mode` without following links;
//! links have no permissions of their own and are left alone, and so are
//! the setuid and setgid bits.

use gio::prelude::*;

use crate::entry::EntryError;
use crate::location::normalise;
use crate::transfer::Cancellation;

/// The permission bits of `unix::mode`.
const PERMISSION_BITS: u32 = 0o7777;
/// The sticky bit: only owners rename and delete a folder's content.
const STICKY: u32 = 0o1000;
/// The attributes a change reads of each item.
const MODE_ATTRIBUTES: &str = "standard::type,standard::name,unix::mode";

/// Who a permission is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionClass {
    /// The item's owner.
    Owner,
    /// The item's group.
    Group,
    /// Everyone else.
    Others,
}

impl PermissionClass {
    /// Every class, in the order the tab lists them.
    pub const ALL: [Self; 3] = [Self::Owner, Self::Group, Self::Others];

    /// How far the class's `rwx` bits are shifted in a mode.
    const fn shift(self) -> u32 {
        match self {
            Self::Owner => 6,
            Self::Group => 3,
            Self::Others => 0,
        }
    }
}

/// What a class may do with an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Nothing.
    None,
    /// Read a file, or list and enter a folder.
    View,
    /// Read and change it.
    ViewAndModify,
}

impl Access {
    /// Every access, in the order the tab offers them.
    pub const ALL: [Self; 3] = [Self::None, Self::View, Self::ViewAndModify];

    /// The words the tab shows, as Dolphin names them.
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No Access",
            Self::View => "Can Only View",
            Self::ViewAndModify => "Can View & Modify",
        }
    }

    /// What `class` may do under `mode`: modify with write access, view
    /// with read access, else nothing.
    pub fn of(mode: u32, class: PermissionClass) -> Self {
        let bits = (mode >> class.shift()) & 0o7;
        match (bits & 0o4 != 0, bits & 0o2 != 0) {
            (true, true) => Self::ViewAndModify,
            (true, false) => Self::View,
            _ => Self::None,
        }
    }

    /// The `rwx` bits this access gives: a folder also gets `x` to be
    /// entered; a file keeps `x` as `executable` says.
    const fn bits(self, is_folder: bool, executable: bool) -> u32 {
        let run = if is_folder || executable { 0o1 } else { 0 };
        match self {
            Self::None => 0,
            Self::View => 0o4 | run,
            Self::ViewAndModify => 0o6 | run,
        }
    }
}

/// The permissions the tab applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionChange {
    /// The owner's access.
    pub owner: Access,
    /// The group's access.
    pub group: Access,
    /// Everyone else's access.
    pub others: Access,
    /// A file may be run by those who may view it; ignored for folders.
    pub executable: bool,
    /// Only owners rename and delete a folder's content; ignored for
    /// files.
    pub owners_only_delete: bool,
}

impl PermissionChange {
    /// The change that keeps `mode` of a file or folder as it is, as the
    /// tab first shows it.
    pub fn of_mode(mode: u32, is_folder: bool) -> Self {
        Self {
            owner: Access::of(mode, PermissionClass::Owner),
            group: Access::of(mode, PermissionClass::Group),
            others: Access::of(mode, PermissionClass::Others),
            executable: !is_folder && mode & 0o100 != 0,
            owners_only_delete: is_folder && mode & STICKY != 0,
        }
    }

    fn access(self, class: PermissionClass) -> Access {
        match class {
            PermissionClass::Owner => self.owner,
            PermissionClass::Group => self.group,
            PermissionClass::Others => self.others,
        }
    }

    /// `mode` with this change applied to a file or folder: the `rwx`
    /// and sticky bits are replaced; setuid and setgid stay.
    pub fn apply_to(self, mode: u32, is_folder: bool) -> u32 {
        let mut result = mode & 0o6000;
        for class in PermissionClass::ALL {
            let bits = self.access(class).bits(is_folder, self.executable);
            result |= bits << class.shift();
        }
        if is_folder && self.owners_only_delete {
            result |= STICKY;
        }
        result
    }
}

/// Applies `change` to the item at `uri`, and with `recursive` to
/// everything inside a folder, without following links. Blocking; see
/// [`apply_in_background`].
///
/// # Errors
///
/// The first item that could not be read or changed, or
/// [`EntryError::Cancelled`].
pub fn apply(
    uri: &str,
    change: PermissionChange,
    recursive: bool,
    cancel: &Cancellation,
) -> Result<(), EntryError> {
    let file = gio::File::for_uri(&normalise(uri)?);
    let info = file.query_info(
        MODE_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    apply_to_item(&file, &info, change, recursive, cancel)
}

/// Changes `file`, then what is inside it when `recursive`.
fn apply_to_item(
    file: &gio::File,
    info: &gio::FileInfo,
    change: PermissionChange,
    recursive: bool,
    cancel: &Cancellation,
) -> Result<(), EntryError> {
    if cancel.is_cancelled() {
        return Err(EntryError::Cancelled);
    }
    let kind = info.file_type();
    // Links have no permissions of their own; special files are skipped.
    if !matches!(kind, gio::FileType::Regular | gio::FileType::Directory) || !info.has_attribute("unix::mode")
    {
        return Ok(());
    }
    let is_folder = kind == gio::FileType::Directory;
    let mode = info.attribute_uint32("unix::mode") & PERMISSION_BITS;
    let wanted = change.apply_to(mode, is_folder);
    if wanted != mode {
        file.set_attribute_uint32(
            "unix::mode",
            wanted,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
    }
    if !(is_folder && recursive) {
        return Ok(());
    }
    let children = file.enumerate_children(
        MODE_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    while let Some(child_info) = children.next_file(Some(cancel.cancellable()))? {
        let child = file.child(child_info.name());
        apply_to_item(&child, &child_info, change, true, cancel)?;
    }
    Ok(())
}

/// [`apply`] on a GIO worker thread, for the main loop to await.
///
/// # Errors
///
/// As [`apply`].
pub async fn apply_in_background(
    uri: String,
    change: PermissionChange,
    recursive: bool,
    cancel: Cancellation,
) -> Result<(), EntryError> {
    match gio::spawn_blocking(move || apply(&uri, change, recursive, &cancel)).await {
        Ok(result) => result,
        // A panicking change is a bug; it surfaces where it is awaited.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::location::file_uri;

    fn mode_of(path: &std::path::Path) -> u32 {
        fs::symlink_metadata(path).expect("metadata").permissions().mode() & PERMISSION_BITS
    }

    /// parity: PROP-007
    #[test]
    fn a_mode_reads_as_three_accesses_and_those_who_may_view_may_run() {
        let change = PermissionChange::of_mode(0o754, false);
        assert_eq!(
            (change.owner, change.group, change.others, change.executable),
            (Access::ViewAndModify, Access::View, Access::View, true)
        );
        assert_eq!(change.apply_to(0o754, false), 0o755, "viewers may run it too");
        let folder = PermissionChange::of_mode(0o1750, true);
        assert!(folder.owners_only_delete);
        assert_eq!(folder.others, Access::None);
        assert_eq!(folder.apply_to(0o1750, true), 0o1750);
        let private = PermissionChange {
            group: Access::None,
            others: Access::None,
            ..PermissionChange::of_mode(0o4644, false)
        };
        assert_eq!(private.apply_to(0o4644, false), 0o4600, "setuid stays");
    }

    /// parity: PROP-007
    #[test]
    fn a_folder_change_reaches_its_content_but_never_a_link_target() {
        let root = tempfile::tempdir().expect("a folder");
        let folder = root.path().join("Shared");
        fs::create_dir(&folder).expect("folder");
        fs::write(folder.join("plan.txt"), b"plan").expect("file");
        fs::create_dir(folder.join("Sub")).expect("subfolder");
        fs::write(root.path().join("outside.txt"), b"x").expect("file");
        fs::set_permissions(root.path().join("outside.txt"), fs::Permissions::from_mode(0o600))
            .expect("mode");
        std::os::unix::fs::symlink(root.path().join("outside.txt"), folder.join("link")).expect("link");
        let change = PermissionChange {
            owner: Access::ViewAndModify,
            group: Access::View,
            others: Access::None,
            executable: false,
            owners_only_delete: false,
        };

        apply(&file_uri(&folder), change, true, &Cancellation::new()).expect("applies");

        assert_eq!(mode_of(&folder), 0o750);
        assert_eq!(mode_of(&folder.join("Sub")), 0o750);
        assert_eq!(mode_of(&folder.join("plan.txt")), 0o640);
        assert_eq!(mode_of(&root.path().join("outside.txt")), 0o600, "links are not followed");
    }
}
