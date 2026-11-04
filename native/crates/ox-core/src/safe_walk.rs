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
        Err(errno) if has_no_statx(errno) => {
            let status = fs::statat(dir, name, flags)?;
            Ok(Mount {
                id: None,
                device: (fs::major(status.st_dev), fs::minor(status.st_dev)),
            })
        }
        Err(errno) => Err(errno.into()),
    }
}

/// Whether `errno` from `statx` means this kernel has no `statx` to
/// ask, rather than that the question itself failed.
///
/// Only these two are a missing syscall: `ENOSYS` from a kernel before
/// Linux 4.11, and `EPERM` from a seccomp filter that refuses it, which
/// is how several container runtimes answer. Anything else — a bad
/// descriptor, a refused lookup, an I/O error — is a real failure and
/// is returned as one. Falling back on every error would quietly turn
/// such a failure into a mount with no ID, and a caller comparing
/// mounts would then compare device numbers alone without being told.
fn has_no_statx(errno: rustix::io::Errno) -> bool {
    matches!(errno, rustix::io::Errno::NOSYS | rustix::io::Errno::PERM)
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

    /// `statx` is only given up on when the kernel has none to offer.
    /// Any other failure is a failure, not a reason to fall back to
    /// `statat` and report a mount without an ID.
    ///
    /// parity: XFER-015
    #[test]
    fn only_a_missing_statx_falls_back_to_statat() {
        use rustix::io::Errno;

        assert!(has_no_statx(Errno::NOSYS), "a kernel before Linux 4.11");
        assert!(has_no_statx(Errno::PERM), "a seccomp filter that refuses statx");
        for errno in [Errno::BADF, Errno::ACCESS, Errno::IO, Errno::NOENT, Errno::LOOP] {
            assert!(
                !has_no_statx(errno),
                "{errno:?} is a failure, not a missing syscall"
            );
        }
    }

    /// `mount_of` asks `has_no_statx` which failures mean the kernel
    /// has none, and reports every other failure as one. The dangerous
    /// outcome it must never produce is `Ok` with no mount ID: a caller
    /// comparing mounts reads that as "this kernel cannot tell me", and
    /// falls back to comparing device numbers alone, without being told
    /// that what really happened was an error.
    ///
    /// The limit of what can be checked here: on an ordinary kernel
    /// there is no call for which `statx` fails and `statat` succeeds,
    /// so the two arms cannot be told apart by their result: the
    /// missing name below fails both ways. What this test pins is the
    /// shape: a live `statx` on a real folder, an error and not a mount
    /// for a failure, and the classification that decides which arm a
    /// failure takes.
    ///
    /// parity: XFER-015
    #[test]
    fn mount_of_reports_a_failing_statx_instead_of_a_mount_without_an_id() {
        use rustix::fs::{Mode, OFlags};
        use rustix::io::Errno;

        let folder = temporary_folder();
        let open = fs::open(
            folder.path(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("the folder is opened");

        let real = mount_of(open.as_fd(), c"", AtFlags::EMPTY_PATH).expect("a real folder");
        let missing = mount_of(open.as_fd(), c"not here", AtFlags::SYMLINK_NOFOLLOW);

        assert!(
            real.id.is_some(),
            "statx reports no mount ID on this kernel, so this test cannot tell the two arms \
             apart at all"
        );
        assert!(
            missing.is_err(),
            "a failing statx became a mount without an ID: {missing:?}"
        );
        assert!(
            !has_no_statx(Errno::NOENT),
            "a name that is not there is classified as a missing statx, so the error above \
             would be turned into a mount without an ID on any kernel where statat succeeds"
        );
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
