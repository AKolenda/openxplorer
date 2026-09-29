// SPDX-License-Identifier: AGPL-3.0-only
//! Editing POSIX permissions from Properties (PROP-007), as the
//! Permissions tab of Dolphin and Files does it: owner, group and others
//! each get "No Access", "Can Only View" or "Can View & Modify", a file can
//! be made executable, a folder can let only owners rename and delete its
//! content (the sticky bit), and a folder's change can reach everything
//! inside it.
//!
//! Permissions are set through GIO's `unix::mode` without following links;
//! links have no permissions of their own and are left alone, and the
//! simple choices keep the setuid and setgid bits. Dolphin's Advanced
//! Permissions set every bit, and the group (and, for the superuser, the
//! owner) is changed through `unix::gid` and `unix::uid`; [`accounts`]
//! lists the choices.

mod accounts;

pub use accounts::{current_user_is_superuser, group_choices, user_choices, Account};

use gio::prelude::*;

use crate::entry::EntryError;
use crate::location::normalise;
use crate::transfer::Cancellation;

/// The permission bits of `unix::mode`.
const PERMISSION_BITS: u32 = 0o7777;
/// The sticky bit: only owners rename and delete a folder's content.
const STICKY: u32 = 0o1000;
/// The attributes a change reads of each item.
const MODE_ATTRIBUTES: &str = "standard::type,standard::name,unix::mode,unix::uid,unix::gid";

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

/// How a request changes the permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChange {
    /// The three accesses of the tab.
    Simple(PermissionChange),
    /// Dolphin's Advanced Permissions: exactly these `rwx`, setuid, setgid
    /// and sticky bits for the item. What is inside a folder gets the same
    /// `rwx` bits and keeps its own special bits, and a file inside gets
    /// `x` only where it had some before, as `chmod -R a=rwX` does.
    Advanced(u32),
}

impl ModeChange {
    /// The bits for the item itself.
    fn for_item(self, mode: u32, is_folder: bool) -> u32 {
        match self {
            Self::Simple(change) => change.apply_to(mode, is_folder),
            Self::Advanced(bits) => bits & PERMISSION_BITS,
        }
    }

    /// The bits for an item inside the folder the change was asked for.
    fn for_content(self, mode: u32, is_folder: bool) -> u32 {
        match self {
            Self::Simple(change) => change.apply_to(mode, is_folder),
            Self::Advanced(bits) => {
                let special = mode & 0o7000;
                let access = bits & 0o777;
                if is_folder || mode & 0o111 != 0 {
                    special | access
                } else {
                    special | (access & !0o111)
                }
            }
        }
    }
}

/// Everything the Permissions tab applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The new permission bits.
    pub mode: ModeChange,
    /// The new owner's user id; only the superuser may change it.
    pub owner: Option<u32>,
    /// The new group id: one of the user's groups.
    pub group: Option<u32>,
    /// Whether a folder's change reaches everything inside it.
    pub recursive: bool,
}

impl PermissionRequest {
    /// A request that only changes the bits, as `change` says.
    pub fn simple(change: PermissionChange, recursive: bool) -> Self {
        Self {
            mode: ModeChange::Simple(change),
            owner: None,
            group: None,
            recursive,
        }
    }
}

/// Applies `request` to the item at `uri`, and when it is recursive to
/// everything inside a folder, without following links. Blocking; see
/// [`apply_in_background`].
///
/// # Errors
///
/// The first item that could not be read or changed, or
/// [`EntryError::Cancelled`].
pub fn apply(uri: &str, request: &PermissionRequest, cancel: &Cancellation) -> Result<(), EntryError> {
    let file = gio::File::for_uri(&normalise(uri)?);
    let info = file.query_info(
        MODE_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    apply_to_item(&file, &info, request, true, cancel)
}

/// Changes `file` (the item asked for when `is_top`), then what is inside
/// it when the request is recursive.
fn apply_to_item(
    file: &gio::File,
    info: &gio::FileInfo,
    request: &PermissionRequest,
    is_top: bool,
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
    set_ownership(file, info, request, cancel)?;
    let mode = info.attribute_uint32("unix::mode") & PERMISSION_BITS;
    let wanted = if is_top {
        request.mode.for_item(mode, is_folder)
    } else {
        request.mode.for_content(mode, is_folder)
    };
    if wanted != mode {
        file.set_attribute_uint32(
            "unix::mode",
            wanted,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
    }
    if !(is_folder && request.recursive) {
        return Ok(());
    }
    let children = file.enumerate_children(
        MODE_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    while let Some(child_info) = children.next_file(Some(cancel.cancellable()))? {
        let child = file.child(child_info.name());
        apply_to_item(&child, &child_info, request, false, cancel)?;
    }
    Ok(())
}

/// Sets the group and owner the request names, where they differ. The
/// group goes first: a user who may not change the owner may still change
/// the group.
fn set_ownership(
    file: &gio::File,
    info: &gio::FileInfo,
    request: &PermissionRequest,
    cancel: &Cancellation,
) -> Result<(), EntryError> {
    for (attribute, wanted) in [("unix::gid", request.group), ("unix::uid", request.owner)] {
        let Some(wanted) = wanted else {
            continue;
        };
        if info.has_attribute(attribute) && info.attribute_uint32(attribute) == wanted {
            continue;
        }
        file.set_attribute_uint32(
            attribute,
            wanted,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
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
    request: PermissionRequest,
    cancel: Cancellation,
) -> Result<(), EntryError> {
    match gio::spawn_blocking(move || apply(&uri, &request, &cancel)).await {
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

        let request = PermissionRequest::simple(change, true);
        apply(&file_uri(&folder), &request, &Cancellation::new()).expect("applies");

        assert_eq!(mode_of(&folder), 0o750);
        assert_eq!(mode_of(&folder.join("Sub")), 0o750);
        assert_eq!(mode_of(&folder.join("plan.txt")), 0o640);
        assert_eq!(
            mode_of(&root.path().join("outside.txt")),
            0o600,
            "links are not followed"
        );
    }

    /// Advanced Permissions set every bit of the folder, give its files
    /// `x` only where they had it, and the group changes with them.
    ///
    /// parity: PROP-007
    #[test]
    fn advanced_permissions_and_the_group_reach_a_folders_content() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("a folder");
        let folder = root.path().join("Tools");
        fs::create_dir(&folder).expect("folder");
        fs::write(folder.join("notes.txt"), b"n").expect("file");
        fs::write(folder.join("run.sh"), b"r").expect("file");
        fs::set_permissions(folder.join("run.sh"), fs::Permissions::from_mode(0o700)).expect("mode");
        let group = rustix::process::getegid().as_raw();
        let request = PermissionRequest {
            mode: ModeChange::Advanced(0o2771),
            owner: None,
            group: Some(group),
            recursive: true,
        };

        apply(&file_uri(&folder), &request, &Cancellation::new()).expect("applies");

        assert_eq!(mode_of(&folder), 0o2771, "setgid and every rwx bit");
        assert_eq!(mode_of(&folder.join("notes.txt")), 0o660, "no x for a plain file");
        assert_eq!(mode_of(&folder.join("run.sh")), 0o771);
        assert_eq!(fs::metadata(&folder).expect("metadata").gid(), group);
    }
}
