// SPDX-License-Identifier: AGPL-3.0-only
//! Permanent deletion of local items relative to pinned folder descriptors.
//!
//! Ports `GioNode._delete_recursive` in `desktop/gio_backend.py` for
//! `file:` items, with stronger protection than the Python app's
//! path-based walk.
//!
//! Rules enforced here:
//! - The selected item's folder is opened once, following symbolic links
//!   like the path the user saw (`~/Music` may be a link to a data drive).
//!   Everything below it is reached through descriptors without following
//!   links, so renaming a folder or swapping it for a link during the
//!   deletion cannot redirect it into another tree.
//! - A symbolic link is removed as a link; its target is never touched.
//! - A folder is checked to still be the folder that was opened before each
//!   of its items and before it is removed; otherwise the deletion stops.
//! - The write guard is asked about every item before it is removed, and
//!   the nesting limit applies.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use gio::prelude::*;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Stat};

use crate::transfer::{nesting_error, Cancellation, TransferError, WriteGuard, MAX_DEPTH};

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
    let parent = path
        .parent()
        .ok_or_else(|| TransferError::failed("Filesystem roots cannot be deleted."))?;
    let name = path
        .file_name()
        .ok_or_else(|| TransferError::failed("Choose a file or folder to delete."))?;
    let parent_folder = open_parent(parent)?;
    Deletion { cancel, guard }.delete_at(parent_folder.as_fd(), name, path, 0)
}

/// Pins the folder that contains the selected item, resolving symbolic
/// links in its path once, as the user's view of the path does.
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

/// One user-confirmed permanent deletion.
struct Deletion<'a> {
    cancel: &'a Cancellation,
    guard: Option<&'a WriteGuard>,
}

impl Deletion<'_> {
    /// Deletes `name` inside the pinned folder `parent`; `path` is its full
    /// path, used only for the write guard.
    fn delete_at(
        &self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        if let Some(guard) = self.guard {
            guard(gio::File::for_path(path).uri().as_str())?;
        }
        let seen = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(seen.st_mode) == FileType::Directory {
            return self.delete_folder(parent, name, path, &seen, depth);
        }
        // unlinkat removes a link itself. If the name raced into a folder,
        // it fails rather than traversing or recursively removing it.
        self.cancel.check()?;
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
        for entry in Dir::read_from(&folder)? {
            self.cancel.check()?;
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
        self.cancel.check()?;
        require_same_item(&opened, &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?)?;
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR)?;
        Ok(())
    }
}

/// Refuses to continue when a name no longer leads to the folder that was
/// opened: the tree changed during the deletion.
fn require_same_item(expected: &Stat, actual: &Stat) -> Result<(), TransferError> {
    if expected.st_dev != actual.st_dev || expected.st_ino != actual.st_ino {
        return Err(TransferError::failed(
            "The folder changed during deletion. Remaining items were left in place.",
        ));
    }
    Ok(())
}
