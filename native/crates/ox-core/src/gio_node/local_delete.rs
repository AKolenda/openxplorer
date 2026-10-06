// SPDX-License-Identifier: AGPL-3.0-only
//! Deletion of local items relative to pinned folder descriptors: the
//! user's confirmed permanent deletions and the engine's own staging.
//!
//! Ports `GioNode._delete_recursive` in `v2.0.0:desktop/gio_backend.py` and
//! `_clean_staging` in `v2.0.0:desktop/operations.py` for `file:` items, with
//! stronger protection than the Python app's path-based walks.
//!
//! Rules enforced here (XFER-015 for the user's deletions, XFER-002 for
//! staging cleanup):
//! - The folder that holds the item is opened once, following symbolic
//!   links like the path the user saw (`~/Music` may be a link to a data
//!   drive). Everything below it is reached through descriptors without
//!   following links, so renaming a folder or swapping it for a link during
//!   the deletion cannot redirect it into another tree.
//! - A symbolic link is removed as a link; its target is never touched.
//! - A folder is checked to still be the folder that was opened before each
//!   of its items and before it is removed; otherwise the deletion stops.
//! - A user deletion asks the write guard about every item (XFER-020) and
//!   can be cancelled; the nesting limit applies.
//! - Staging cleanup starts only while the staging name still leads to the
//!   folder the engine created, and makes each of its folders owner-only
//!   before emptying it (a copied read-only mode must not block cleanup).
//!   It ignores cancellation: it runs after a cancelled copy.
//! - A deletion never leaves the drive the item is on. A mount point (a
//!   drive, share or bind mount) is refused as the item, and a mount inside
//!   the item stops the deletion before anything is removed when the mount
//!   table shows it, or before the walk enters it otherwise.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use gio::prelude::*;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Stat, StatxFlags};

use crate::transfer::{
    check_cancelled, nesting_error, Cancellation, ItemIdentity, TransferError, WriteGuard, MAX_DEPTH,
    PRIVATE_DIRECTORY_MODE, STAGING_LEVELS,
};

/// Permanently deletes the local item at `path` and everything inside it.
///
/// # Errors
///
/// The first failure, the guard's refusal, the nesting limit or
/// [`TransferError::Cancelled`]; items not yet reached stay in place.
pub(super) fn delete_tree(
    path: &Path,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
) -> Result<(), TransferError> {
    cancel.check()?;
    refuse_mounts_in(path)?;
    let mut deletion = Deletion {
        cancel: Some(cancel),
        guard,
        max_depth: MAX_DEPTH,
        folders: FolderAccess::AsFound,
        root_identity: None,
        drive: None,
    };
    deletion.delete_path(path)
}

/// Removes the staging tree at `path`, which the engine created. With
/// `created`, the tree is removed only while its name still leads to that
/// folder.
///
/// # Errors
///
/// The first item that cannot be removed, or a staging name that now leads
/// to another item; the rest stays for the caller to report.
pub(super) fn delete_staging(path: &Path, created: Option<ItemIdentity>) -> Result<(), TransferError> {
    let mut deletion = Deletion {
        cancel: None,
        guard: None,
        // The staging folder and its payload add two levels to the tree.
        max_depth: MAX_DEPTH + STAGING_LEVELS,
        folders: FolderAccess::MadePrivate,
        root_identity: created,
        drive: None,
    };
    deletion.delete_path(path)
}

/// Pins the folder that contains the item, resolving symbolic links in its
/// path once, as the user's view of the path does. The descriptor is
/// `O_PATH`: it only anchors the `*at` calls below it.
fn open_parent(path: &Path) -> Result<OwnedFd, TransferError> {
    // XFER-015: GIO paths are absolute and free of `..`. Anything else means
    // the path is not the one the user saw, and `..` after a link would
    // resolve to the link target's parent, not the folder shown.
    let has_parent_reference = path.components().any(|part| part == Component::ParentDir);
    if !path.is_absolute() || has_parent_reference {
        return Err(TransferError::failed(crate::i18n::gettext(
            "Open the actual folder before deleting its contents.",
        )));
    }
    let flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC;
    fs::open(path, flags, Mode::empty()).map_err(|errno| {
        TransferError::failed(format!(
            "The folder {} could not be opened for deletion. {}",
            path.display(),
            std::io::Error::from(errno)
        ))
    })
}

/// Whether folders keep their modes while they are emptied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderAccess {
    /// The user's folders are never changed; a read-only one stops the
    /// deletion.
    AsFound,
    /// The engine's own staging folders become owner-only first.
    MadePrivate,
}

/// One descriptor-pinned deletion and its rules.
struct Deletion<'a> {
    /// Stops between items once cancelled; `None` for staging cleanup.
    cancel: Option<&'a Cancellation>,
    /// Asked about every item before it is removed.
    guard: Option<&'a WriteGuard>,
    /// The deepest nesting walked.
    max_depth: usize,
    /// Whether folders are made owner-only before they are emptied.
    folders: FolderAccess,
    /// The identity the deleted item itself must have, when known.
    root_identity: Option<ItemIdentity>,
    /// The mount of the folder that holds the item, once it is opened;
    /// nothing on another mount is deleted.
    drive: Option<Mount>,
}

impl Deletion<'_> {
    /// Deletes the item at the absolute `path`.
    fn delete_path(&mut self, path: &Path) -> Result<(), TransferError> {
        let parent = path.parent().ok_or_else(|| {
            TransferError::failed(crate::i18n::gettext("Filesystem roots cannot be deleted."))
        })?;
        let name = path.file_name().ok_or_else(|| {
            TransferError::failed(crate::i18n::gettext("Choose a file or folder to delete."))
        })?;
        let parent_folder = open_parent(parent)?;
        self.drive = Some(mount_of(parent_folder.as_fd(), c"", AtFlags::EMPTY_PATH)?);
        self.delete_at(parent_folder.as_fd(), name, path, 0)
    }

    /// Deletes `name` inside the pinned folder `parent`; `path` is its full
    /// path, used only for the write guard.
    fn delete_at(
        &self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        depth: usize,
    ) -> Result<(), TransferError> {
        check_cancelled(self.cancel)?;
        if depth > self.max_depth {
            return Err(nesting_error());
        }
        if let Some(guard) = self.guard {
            guard(gio::File::for_path(path).uri().as_str())?;
        }
        let seen = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if depth == 0 {
            // XFER-002: cleanup refuses a folder moved in under the staging
            // name; only the folder the engine created is removed.
            self.require_root_identity(&seen)?;
            // A file can be a mount point too (a bind mount of a file).
            self.require_same_drive(parent, name, path, depth)?;
        }
        if FileType::from_raw_mode(seen.st_mode) == FileType::Directory {
            return self.delete_folder(parent, name, path, &seen, depth);
        }
        // unlinkat removes a link itself. If the name raced into a folder,
        // it fails rather than traversing or recursively removing it.
        check_cancelled(self.cancel)?;
        fs::unlinkat(parent, name, AtFlags::empty())?;
        Ok(())
    }

    /// Empties and removes the folder `name`, which was `seen` as a folder
    /// without following links.
    fn delete_folder(
        &self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        seen: &Stat,
        depth: usize,
    ) -> Result<(), TransferError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let folder = fs::openat(parent, name, flags, Mode::empty())?;
        let opened = fs::fstat(&folder)?;
        // XFER-015: the name was swapped for another folder between the
        // `lstat` and the open; stop before touching anything inside it.
        require_same_item(seen, &opened)?;
        // A drive mounted on this folder (also one mounted after the mount
        // table was read): the descriptor is on that drive, so stop before
        // reading anything inside it.
        self.require_drive(mount_of(folder.as_fd(), c"", AtFlags::EMPTY_PATH)?, path, depth)?;
        if self.folders == FolderAccess::MadePrivate {
            // Through the pinned descriptor, so the mode lands on this folder.
            fs::fchmod(&folder, Mode::from_raw_mode(PRIVATE_DIRECTORY_MODE))?;
        }
        for entry in Dir::read_from(&folder)? {
            check_cancelled(self.cancel)?;
            let entry = entry?;
            let child_name = OsStr::from_bytes(entry.file_name().to_bytes());
            if child_name == "." || child_name == ".." {
                continue;
            }
            // Stop if the folder's name was swapped after opening. All I/O
            // still goes through its descriptor, so even a swap after this
            // check cannot redirect the next item's deletion.
            require_still_named(parent, name, &opened)?;
            self.delete_at(folder.as_fd(), child_name, &path.join(child_name), depth + 1)?;
        }
        check_cancelled(self.cancel)?;
        require_still_named(parent, name, &opened)?;
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR)?;
        Ok(())
    }

    /// Refuses to go on when `name` in `parent` is on another mount than
    /// the folder that holds the deleted item: it is a mount point.
    fn require_same_drive(
        &self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        depth: usize,
    ) -> Result<(), TransferError> {
        let mount = mount_of(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
        self.require_drive(mount, path, depth)
    }

    /// Refuses `mount`, the mount of the item at `path`, unless it is the
    /// drive the deletion started on.
    fn require_drive(&self, mount: Mount, path: &Path, depth: usize) -> Result<(), TransferError> {
        match self.drive {
            Some(drive) if !drive.is_same_as(mount) => Err(if depth == 0 {
                mount_point_refusal(path)
            } else {
                contained_mount_refusal(path)
            }),
            _ => Ok(()),
        }
    }

    /// Refuses an item other than the one the engine recorded, for example
    /// another folder moved in under the staging name by someone who can
    /// write to the destination.
    fn require_root_identity(&self, seen: &Stat) -> Result<(), TransferError> {
        match self.root_identity {
            Some(expected) if expected != identity_of(seen) => Err(TransferError::failed(
                crate::i18n::gettext("Another item now has the staging folder's name. It was left in place."),
            )),
            _ => Ok(()),
        }
    }
}

/// The mount an item is on: the kernel's mount ID where `statx` reports it
/// (Linux 5.8 and later), which also tells apart a bind mount of a folder
/// on the same filesystem, and the filesystem's device number otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mount {
    /// The mount ID, when the kernel reports it.
    id: Option<u64>,
    /// The device number of the filesystem.
    device: (u32, u32),
}

impl Mount {
    /// Whether both describe the same mount. Mount IDs decide when both
    /// have one: on one mount, a Btrfs subvolume has its own device number.
    fn is_same_as(self, other: Mount) -> bool {
        match (self.id, other.id) {
            (Some(own), Some(theirs)) => own == theirs,
            _ => self.device == other.device,
        }
    }
}

/// The mount of `name` in `dir` (with [`AtFlags::EMPTY_PATH`] and an empty
/// name, of `dir` itself). Never triggers an automount. Where `statx` is
/// unavailable (before Linux 4.11, or blocked by a container's seccomp
/// filter), only the device number is known.
fn mount_of<P: rustix::path::Arg + Copy>(
    dir: BorrowedFd<'_>,
    name: P,
    flags: AtFlags,
) -> Result<Mount, TransferError> {
    let flags = flags | AtFlags::NO_AUTOMOUNT;
    match fs::statx(dir, name, flags, StatxFlags::MNT_ID) {
        Ok(status) => {
            let has_id = status.stx_mask & StatxFlags::MNT_ID.bits() != 0;
            Ok(Mount {
                id: has_id.then_some(status.stx_mnt_id),
                device: (status.stx_dev_major, status.stx_dev_minor),
            })
        }
        Err(rustix::io::Errno::NOSYS | rustix::io::Errno::PERM) => {
            let status = fs::statat(dir, name, flags)?;
            Ok(Mount {
                id: None,
                device: (fs::major(status.st_dev), fs::minor(status.st_dev)),
            })
        }
        Err(errno) => Err(errno.into()),
    }
}

/// Refuses to delete the item at `path` when the mount table shows that it
/// is a mount point or that something is mounted inside it. Nothing has
/// been deleted yet. When the mount table cannot be read, the walk's own
/// checks still keep the deletion on its drive.
fn refuse_mounts_in(path: &Path) -> Result<(), TransferError> {
    let Ok(mount_points) = crate::sizes::mounts::read_mount_points() else {
        return Ok(());
    };
    let Some(item) = resolved_item_path(path) else {
        return Ok(());
    };
    match first_mount_in(&item, &mount_points) {
        Some(mount_point) if *mount_point == item => Err(mount_point_refusal(path)),
        Some(mount_point) => {
            let inside = mount_point.strip_prefix(&item).unwrap_or(mount_point);
            Err(TransferError::failed(crate::i18n::format_message(
                "A drive or share is mounted at {path}, inside the item being deleted. Nothing was \
                 deleted. Unmount it first.",
                &[("path", &path.join(inside).display().to_string())],
            )))
        }
        None => Ok(()),
    }
}

/// `path` with the symbolic links of its folder resolved, as the mount
/// table names mount points. The item itself is not followed: a link is
/// deleted as a link.
fn resolved_item_path(path: &Path) -> Option<PathBuf> {
    let folder = std::fs::canonicalize(path.parent()?).ok()?;
    Some(folder.join(path.file_name()?))
}

/// The mount point that is `item` itself, or else the first one inside it
/// in path order.
fn first_mount_in<'m>(
    item: &Path,
    mount_points: impl IntoIterator<Item = &'m PathBuf>,
) -> Option<&'m PathBuf> {
    mount_points
        .into_iter()
        .filter(|mount_point| mount_point.starts_with(item))
        .min()
}

/// The refusal to delete a mount point.
fn mount_point_refusal(path: &Path) -> TransferError {
    TransferError::failed(crate::i18n::format_message(
        "{path} is where a drive or share is mounted, so it was not deleted. Unmount it first.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal to enter the mount point at `path` inside the deleted item.
fn contained_mount_refusal(path: &Path) -> TransferError {
    TransferError::failed(crate::i18n::format_message(
        "A drive or share is mounted at {path}. Deleting never goes into another drive, so it \
         stopped there. Unmount it first.",
        &[("path", &path.display().to_string())],
    ))
}

/// The device and inode `stat` describes.
fn identity_of(stat: &Stat) -> ItemIdentity {
    ItemIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    }
}

/// Refuses to continue when `name` in `parent` no longer leads to the
/// folder that was `opened`.
fn require_still_named(parent: BorrowedFd<'_>, name: &OsStr, opened: &Stat) -> Result<(), TransferError> {
    let current = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    require_same_item(opened, &current)
}

/// Refuses to continue when a name no longer leads to the folder that was
/// opened: the tree changed during the deletion.
fn require_same_item(expected: &Stat, actual: &Stat) -> Result<(), TransferError> {
    if identity_of(expected) != identity_of(actual) {
        return Err(TransferError::failed(crate::i18n::gettext(
            "The folder changed during deletion. Remaining items were left in place.",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod mount_tests;

#[cfg(test)]
mod tests {
    //! GIO only produces absolute paths without `..`, so the integration
    //! tests in `tests/gio_node_cases/removal.rs` cannot reach the refusal
    //! of other paths; it is tested here.

    use super::*;
    use crate::test_support::temporary_folder;

    /// parity: XFER-015
    #[test]
    fn a_relative_or_dotdot_path_is_refused_before_anything_is_opened() {
        let refusal = TransferError::failed("Open the actual folder before deleting its contents.");

        let relative = open_parent(Path::new("relative")).err();
        let through_parent = open_parent(Path::new("/tmp/a/../b")).err();

        assert_eq!(relative, Some(refusal.clone()));
        assert_eq!(through_parent, Some(refusal));
    }

    /// parity: XFER-015
    #[test]
    fn an_absolute_folder_path_is_opened() {
        let root = temporary_folder();

        let opened = open_parent(root.path());

        assert!(opened.is_ok(), "{:?}", opened.err());
    }
}
