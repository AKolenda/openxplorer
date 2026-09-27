// SPDX-License-Identifier: AGPL-3.0-only
//! Deletion of local items relative to pinned folder descriptors: the
//! user's confirmed permanent deletions and the engine's own staging.
//!
//! Ports `GioNode._delete_recursive` in `desktop/gio_backend.py` and
//! `_clean_staging` in `desktop/operations.py` for `file:` items, with
//! stronger protection than the Python app's path-based walks.
//!
//! Rules enforced here:
//! - The folder that holds the item is opened once, following symbolic
//!   links like the path the user saw (`~/Music` may be a link to a data
//!   drive). Everything below it is reached through descriptors without
//!   following links, so renaming a folder or swapping it for a link during
//!   the deletion cannot redirect it into another tree.
//! - A symbolic link is removed as a link; its target is never touched.
//! - A folder is checked to still be the folder that was opened before each
//!   of its items and before it is removed; otherwise the deletion stops.
//! - A user deletion asks the write guard about every item and can be
//!   cancelled; the nesting limit applies.
//! - Staging cleanup starts only while the staging name still leads to the
//!   folder the engine created, and makes each of its folders owner-only
//!   before emptying it (a copied read-only mode must not block cleanup).
//!   It ignores cancellation: it runs after a cancelled copy.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use gio::prelude::*;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Stat};

use crate::transfer::{
    nesting_error, Cancellation, ItemIdentity, TransferError, WriteGuard, MAX_DEPTH, STAGING_LEVELS,
};

/// Owner-only access for staging folders being emptied.
const PRIVATE_FOLDER_MODE: u32 = 0o700;

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
    let deletion = Deletion {
        cancel: Some(cancel),
        guard,
        max_depth: MAX_DEPTH,
        folders: FolderAccess::AsFound,
        root_identity: None,
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
    let deletion = Deletion {
        cancel: None,
        guard: None,
        // The staging folder and its payload add two levels to the tree.
        max_depth: MAX_DEPTH + STAGING_LEVELS,
        folders: FolderAccess::MadePrivate,
        root_identity: created,
    };
    deletion.delete_path(path)
}

/// Pins the folder that contains the item, resolving symbolic links in its
/// path once, as the user's view of the path does.
fn open_parent(path: &Path) -> Result<OwnedFd, TransferError> {
    // GIO paths are absolute and free of `..`. Anything else means the path
    // is not the one the user saw, and `..` after a link would resolve to
    // the link target's parent, not the folder shown.
    let has_parent_reference = path.components().any(|part| part == Component::ParentDir);
    if !path.is_absolute() || has_parent_reference {
        return Err(TransferError::failed(
            "Open the actual folder before deleting its contents.",
        ));
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
    folders: FolderAccess,
    /// The identity the deleted item itself must have, when known.
    root_identity: Option<ItemIdentity>,
}

impl Deletion<'_> {
    /// Deletes the item at the absolute `path`.
    fn delete_path(&self, path: &Path) -> Result<(), TransferError> {
        let parent = path
            .parent()
            .ok_or_else(|| TransferError::failed("Filesystem roots cannot be deleted."))?;
        let name = path
            .file_name()
            .ok_or_else(|| TransferError::failed("Choose a file or folder to delete."))?;
        let parent_folder = open_parent(parent)?;
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
        self.check_cancelled()?;
        if depth > self.max_depth {
            return Err(nesting_error());
        }
        if let Some(guard) = self.guard {
            guard(gio::File::for_path(path).uri().as_str())?;
        }
        let seen = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if depth == 0 {
            self.require_root_identity(&seen)?;
        }
        if FileType::from_raw_mode(seen.st_mode) == FileType::Directory {
            return self.delete_folder(parent, name, path, &seen, depth);
        }
        // unlinkat removes a link itself. If the name raced into a folder,
        // it fails rather than traversing or recursively removing it.
        self.check_cancelled()?;
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
        require_same_item(seen, &opened)?;
        if self.folders == FolderAccess::MadePrivate {
            // Through the pinned descriptor, so the mode lands on this folder.
            fs::fchmod(&folder, Mode::from_raw_mode(PRIVATE_FOLDER_MODE))?;
        }
        for entry in Dir::read_from(&folder)? {
            self.check_cancelled()?;
            let entry = entry?;
            let child_name = OsStr::from_bytes(entry.file_name().to_bytes());
            if child_name == "." || child_name == ".." {
                continue;
            }
            // Stop if the folder's name was swapped after opening. All I/O
            // still goes through its descriptor, so even a swap after this
            // check cannot redirect the next item's deletion.
            require_same_item(&opened, &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?)?;
            self.delete_at(folder.as_fd(), child_name, &path.join(child_name), depth + 1)?;
        }
        self.check_cancelled()?;
        require_same_item(&opened, &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?)?;
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR)?;
        Ok(())
    }

    fn check_cancelled(&self) -> Result<(), TransferError> {
        match self.cancel {
            Some(cancel) => cancel.check(),
            None => Ok(()),
        }
    }

    /// Refuses an item other than the one the engine recorded, for example
    /// another folder moved in under the staging name by someone who can
    /// write to the destination.
    fn require_root_identity(&self, seen: &Stat) -> Result<(), TransferError> {
        match self.root_identity {
            Some(expected) if expected != identity_of(seen) => Err(TransferError::failed(
                "Another item now has the staging folder's name. It was left in place.",
            )),
            _ => Ok(()),
        }
    }
}

fn identity_of(stat: &Stat) -> ItemIdentity {
    ItemIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    }
}

/// Refuses to continue when a name no longer leads to the folder that was
/// opened: the tree changed during the deletion.
fn require_same_item(expected: &Stat, actual: &Stat) -> Result<(), TransferError> {
    if identity_of(expected) != identity_of(actual) {
        return Err(TransferError::failed(
            "The folder changed during deletion. Remaining items were left in place.",
        ));
    }
    Ok(())
}
