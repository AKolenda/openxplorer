// SPDX-License-Identifier: AGPL-3.0-only
//! The primitives every deletion that walks a tree through pinned folder
//! descriptors is built from, shared by the unprivileged `file:` walk in
//! [`crate::gio_node`] and by the privileged `admin://` helper in
//! [`crate::admin_delete`].
//!
//! A walk by path asks the kernel to resolve the whole path again for
//! every step, so a folder replaced by a symbolic link between two steps
//! sends the next step somewhere else. A walk through descriptors resolves
//! one name at a time, relative to a folder it already holds open, and
//! never follows a link while doing so. What is left is checking that each
//! name still leads to the item that was opened, and that the walk has not
//! stepped onto another mount; both live here.

use std::ffi::OsStr;

use rustix::fd::BorrowedFd;
use rustix::fs::{self, AtFlags, Stat, StatxFlags};

use crate::transfer::{ItemIdentity, TransferError};

/// The mount an item is on: the kernel's mount ID where `statx` reports it
/// (Linux 5.8 and later), which also tells apart a bind mount of a folder
/// on the same filesystem, and the filesystem's device number otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mount {
    /// The mount ID, when the kernel reports it.
    pub(crate) id: Option<u64>,
    /// The device number of the filesystem.
    pub(crate) device: (u32, u32),
}

impl Mount {
    /// Whether both describe the same mount. Mount IDs decide when both
    /// have one: on one mount, a Btrfs subvolume has its own device number.
    pub(crate) fn is_same_as(self, other: Mount) -> bool {
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
pub(crate) fn mount_of<P: rustix::path::Arg + Copy>(
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

/// The device and inode `stat` describes.
pub(crate) fn identity_of(stat: &Stat) -> ItemIdentity {
    ItemIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    }
}

/// Refuses to continue when `name` in `parent` no longer leads to the
/// folder that was `opened`.
pub(crate) fn require_still_named(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    opened: &Stat,
) -> Result<(), TransferError> {
    let current = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    require_same_item(opened, &current)
}

/// Refuses to continue when a name no longer leads to the folder that was
/// opened: the tree changed during the deletion.
pub(crate) fn require_same_item(expected: &Stat, actual: &Stat) -> Result<(), TransferError> {
    if identity_of(expected) != identity_of(actual) {
        return Err(folder_changed());
    }
    Ok(())
}

/// The refusal that stops a walk whose tree changed under it.
pub(crate) fn folder_changed() -> TransferError {
    TransferError::failed(crate::i18n::gettext(
        "The folder changed during deletion. Remaining items were left in place.",
    ))
}

#[cfg(test)]
mod tests {
    use rustix::fd::AsFd;

    use super::*;
    use crate::test_support::temporary_folder;

    /// parity: XFER-015
    #[test]
    fn a_name_that_now_leads_elsewhere_is_refused() {
        let root = temporary_folder();
        let folder = root.path().join("folder");
        let other = root.path().join("other");
        std::fs::create_dir(&folder).expect("create the folder");
        // A folder of its own, renamed over the name below: recreating one
        // can reuse the inode that was just freed, which would not be a
        // swap at all.
        std::fs::create_dir(&other).expect("create the other folder");
        let parent = fs::open(
            root.path(),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("open the parent");
        let opened = fs::statat(&parent, "folder", AtFlags::SYMLINK_NOFOLLOW).expect("stat");

        let unchanged = require_still_named(parent.as_fd(), OsStr::new("folder"), &opened);
        std::fs::remove_dir(&folder).expect("remove the folder");
        std::fs::rename(&other, &folder).expect("the other folder takes its name");
        let swapped = require_still_named(parent.as_fd(), OsStr::new("folder"), &opened);

        assert_eq!(unchanged, Ok(()));
        assert_eq!(swapped, Err(folder_changed()));
    }

    /// parity: XFER-015
    #[test]
    fn a_folder_is_on_the_same_mount_as_itself() {
        let root = temporary_folder();
        let folder = fs::open(
            root.path(),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("open the folder");

        let mount = mount_of(folder.as_fd(), c"", AtFlags::EMPTY_PATH).expect("the mount is read");
        let again = mount_of(folder.as_fd(), c"", AtFlags::EMPTY_PATH).expect("the mount is read");

        assert!(mount.is_same_as(again));
    }
}
