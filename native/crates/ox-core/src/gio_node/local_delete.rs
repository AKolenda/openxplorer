// SPDX-License-Identifier: AGPL-3.0-only
//! Local deletion relative to pinned directory descriptors.
//!
//! Every descent opens exactly one child with NOFOLLOW. Renaming an ancestor
//! or replacing it with a symbolic link cannot redirect later deletions into
//! another tree. No pathname-based recursive deletion is used here.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use gio::prelude::*;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Stat};

use crate::transfer::{Cancellation, TransferError, WriteGuard, MAX_DEPTH};

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
    let directory = open_parent(parent)?;
    Deletion { cancel, guard }.delete_at(directory.as_fd(), name, path, 0)
}

/// Pins the containing folder without traversing symbolic links anywhere
/// in its ancestry. Opening the actual folder first is required for paths
/// reached through an alias.
fn open_parent(path: &Path) -> Result<OwnedFd, TransferError> {
    if !path.is_absolute() {
        return Err(TransferError::failed(
            "Permanent deletion requires an absolute local path.",
        ));
    }
    let flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut parent = fs::open("/", flags, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                parent = fs::openat(&parent, name, flags, Mode::empty())?;
            }
            _ => {
                return Err(TransferError::failed(
                    "Open the actual folder before deleting its contents.",
                ))
            }
        }
    }
    Ok(parent)
}

struct Deletion<'a> {
    cancel: &'a Cancellation,
    guard: Option<&'a WriteGuard>,
}

impl Deletion<'_> {
    fn delete_at(
        &self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(TransferError::failed(
                "Folder nesting exceeds this build’s safety limit (128).",
            ));
        }
        if let Some(guard) = self.guard {
            guard(gio::File::for_path(path).uri().as_str())?;
        }
        let metadata = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory {
            // unlinkat removes a link itself. If it raced into a directory,
            // it fails rather than traversing or recursively removing it.
            self.cancel.check()?;
            fs::unlinkat(parent, name, AtFlags::empty())?;
            return Ok(());
        }
        let directory = fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let opened = fs::fstat(&directory)?;
        require_same_item(&metadata, &opened)?;
        let entries = Dir::read_from(&directory)?;
        for entry in entries {
            self.cancel.check()?;
            let entry = entry?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            // Stop if the selected directory name was swapped after opening.
            // All actual I/O still uses its descriptor, so even a swap after
            // this check cannot redirect the next child's deletion.
            require_same_item(&opened, &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?)?;
            let child_name = OsStr::from_bytes(bytes);
            self.delete_at(directory.as_fd(), child_name, &path.join(child_name), depth + 1)?;
        }
        self.cancel.check()?;
        require_same_item(&opened, &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?)?;
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR)?;
        Ok(())
    }
}

fn require_same_item(expected: &Stat, actual: &Stat) -> Result<(), TransferError> {
    if expected.st_dev != actual.st_dev || expected.st_ino != actual.st_ino {
        return Err(TransferError::failed(
            "The folder changed during deletion. Remaining items were left in place.",
        ));
    }
    Ok(())
}
