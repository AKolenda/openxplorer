// SPDX-License-Identifier: AGPL-3.0-only
//! The drive a copy or move writes to went away while it ran: a USB stick
//! pulled out, a disk unmounted by the desktop or from a terminal.
//!
//! The items then fail with whatever the system said about the path they
//! were written to: "No such file or directory" once the desktop removed
//! the drive's folder, an I/O error, or "Transport endpoint is not
//! connected" from a FUSE driver that stopped. As "No such file or
//! directory" the question read "Item Not Found … This is no longer
//! located in <the source folder>", blaming an item that is still where it
//! was, and every entry of a folder being copied was asked about in turn.
//! Windows 11 says "Error 0x800701B1: A device which does not exist was
//! specified." in its "Interrupted Action" question; so does this engine,
//! once for the item, when the drive that held the destination folder is
//! gone. After Skip, Windows asks nothing more: it says the destination
//! "is unavailable" ("Location is not available") and OK ends the copy,
//! as [`ItemFailures`](super::item_failure) does here.
//!
//! A drive counts as gone when the mount that held the destination folder
//! when the run started has left the mount table, or, for a local drive,
//! when the folder answers with an error that only a missing device gives.
//! A folder the user deleted on a drive still mounted is not a gone drive.

use std::path::PathBuf;

use gio::prelude::*;

use super::error::TransferError;
use crate::network::{mount_for_path, parse_mount_table, MountEntry};

/// This process's mount table.
const MOUNT_INFO: &str = "/proc/self/mountinfo";

/// The errors a folder on a drive that went away answers with, besides
/// "No such file or directory" once its folder was removed: an I/O error,
/// no such device or address, a FUSE driver that stopped, a shut-down
/// transport.
const GONE_DEVICE_ERRORS: [i32; 5] = [
    rustix::io::Errno::IO.raw_os_error(),
    rustix::io::Errno::NODEV.raw_os_error(),
    rustix::io::Errno::NXIO.raw_os_error(),
    rustix::io::Errno::NOTCONN.raw_os_error(),
    rustix::io::Errno::SHUTDOWN.raw_os_error(),
];

/// Windows' words for a drive pulled out while it is written to
/// (`ERROR_DEV_NOT_EXIST`, "A device which does not exist was
/// specified.", as Windows 11's copy reports it), as the failure of an
/// item written to it. It is a [`TransferError::NotMounted`]: the whole
/// item fails at once, not entry by entry, as for a share that went away.
pub(crate) fn device_not_ready() -> TransferError {
    TransferError::NotMounted(crate::system_error::DEV_NOT_EXIST.message())
}

/// Whether `error`, as a question shows it, is [`device_not_ready`]'s.
pub(crate) fn is_device_gone_message(error: &str) -> bool {
    error == crate::system_error::DEV_NOT_EXIST.message()
}

/// The drive of a run's destination folder, as found when the run started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DestinationDrive {
    /// The destination folder, when it is on this computer.
    folder: Option<PathBuf>,
    /// Where the drive that holds it was mounted, unless that is the root
    /// filesystem, which never goes away.
    mount_point: Option<String>,
    /// Whether that drive is a network place, whose folder is never read
    /// here: reading a share that stopped answering can block for long.
    is_network: bool,
    /// The mount table and folder error tests say the system has now.
    #[cfg(test)]
    now: Option<(String, Option<i32>)>,
}

impl DestinationDrive {
    /// The drive of the destination folder `uri`, or of none (a Recycle
    /// Bin or delete run, or a folder not on this computer), from this
    /// process's mount table.
    pub(crate) fn of(uri: Option<&str>) -> Self {
        let table = std::fs::read_to_string(MOUNT_INFO).unwrap_or_default();
        Self::in_table(uri, &table)
    }

    /// As [`Self::of`], from the mount table `table`.
    fn in_table(uri: Option<&str>, table: &str) -> Self {
        let Some(folder) = uri.and_then(local_path) else {
            return Self::default();
        };
        let mounts = parse_mount_table(table);
        let mount = folder.to_str().and_then(|path| mount_for_path(path, &mounts));
        let mount_point = mount
            .filter(|mount| mount.path != "/")
            .map(|mount| mount.path.clone());
        let is_network =
            mount.is_some_and(|mount: &MountEntry| crate::drive::is_network_filesystem(&mount.filesystem));
        Self {
            folder: Some(folder),
            mount_point,
            is_network,
            #[cfg(test)]
            now: None,
        }
    }

    /// The drive of the destination folder `uri` in the mount table
    /// `table`, which the system says it has now too, with `folder_error`
    /// reading that folder; for tests.
    #[cfg(test)]
    pub(crate) fn fixed(uri: &str, table: &str, folder_error: Option<i32>) -> Self {
        Self {
            now: Some((table.to_owned(), folder_error)),
            ..Self::in_table(Some(uri), table)
        }
    }

    /// The same drive, now with the mount table `table`; for tests.
    #[cfg(test)]
    pub(crate) fn now(self, table: &str, folder_error: Option<i32>) -> Self {
        Self {
            now: Some((table.to_owned(), folder_error)),
            ..self
        }
    }

    /// Whether the drive went away since the run started.
    pub(crate) fn is_gone(&self) -> bool {
        if self.folder.is_none() {
            return false;
        }
        #[cfg(test)]
        if let Some((table, folder_error)) = &self.now {
            return self.is_gone_from(table, *folder_error);
        }
        let table = std::fs::read_to_string(MOUNT_INFO).unwrap_or_default();
        let folder_error = if self.is_network {
            None
        } else {
            self.folder
                .as_ref()
                .and_then(|folder| std::fs::metadata(folder).err())
                .and_then(|error| error.raw_os_error())
        };
        self.is_gone_from(&table, folder_error)
    }

    /// Whether the drive is gone, given the mount table `table` now and
    /// the error reading the destination folder now gave, if any.
    fn is_gone_from(&self, table: &str, folder_error: Option<i32>) -> bool {
        if self.folder.is_none() {
            return false;
        }
        let unmounted = self
            .mount_point
            .as_ref()
            .is_some_and(|point| !parse_mount_table(table).iter().any(|mount| mount.path == *point));
        unmounted || folder_error.is_some_and(|error| GONE_DEVICE_ERRORS.contains(&error))
    }

    /// `error`, the failure of an item written to this drive, as the user
    /// should hear it: "The device is not ready." when the drive is gone
    /// and the error is one a missing drive gives. A cancellation, a name
    /// conflict, staging left to recover by hand and the like keep theirs.
    pub(crate) fn explain(&self, error: TransferError) -> TransferError {
        if is_drive_error(&error) && self.is_gone() {
            device_not_ready()
        } else {
            error
        }
    }
}

/// Whether `error` is one an item gets when its drive is gone.
fn is_drive_error(error: &TransferError) -> bool {
    matches!(
        error,
        TransferError::NotFound(_)
            | TransferError::NotMounted(_)
            | TransferError::PermissionDenied(_)
            | TransferError::NoSpace(_)
            | TransferError::Busy(_)
            | TransferError::Failed(_)
    )
}

/// The path of the `file:` location `uri`, or `None` for any other.
fn local_path(uri: &str) -> Option<PathBuf> {
    if !uri.starts_with("file:") {
        return None;
    }
    gio::File::for_uri(uri).path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mount table with the root filesystem, a USB stick at
    /// /media/demo/STICK and an SMB share at /mnt/nas.
    pub(crate) const TABLE: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
95 22 8:17 / /media/demo/STICK rw,nosuid,nodev shared:50 - vfat /dev/sdb1 rw,uid=1000
96 22 0:51 / /mnt/nas rw,relatime shared:51 - cifs //nas/share rw
";

    /// The same table once the stick was pulled out.
    pub(crate) const WITHOUT_STICK: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
96 22 0:51 / /mnt/nas rw,relatime shared:51 - cifs //nas/share rw
";

    const ENOENT: i32 = 2;

    fn stick() -> DestinationDrive {
        DestinationDrive::in_table(Some("file:///media/demo/STICK/Photos"), TABLE)
    }

    /// A stick that left the mount table is gone; one still mounted whose
    /// destination folder was deleted is not.
    ///
    /// parity: OPS-047
    #[test]
    fn a_stick_that_left_the_mount_table_is_gone() {
        let drive = stick();
        assert!(drive.is_gone_from(WITHOUT_STICK, Some(ENOENT)));
        assert!(drive.is_gone_from(WITHOUT_STICK, None));
        assert!(!drive.is_gone_from(TABLE, Some(ENOENT)), "a deleted folder");
        assert!(!drive.is_gone_from(TABLE, None));
    }

    /// A drive still in the mount table whose folder answers with an
    /// error only a missing device gives (a FUSE driver that stopped, an
    /// I/O error) is gone too.
    ///
    /// parity: OPS-047
    #[test]
    fn a_drive_whose_folder_lost_its_device_is_gone() {
        let drive = stick();
        for error in GONE_DEVICE_ERRORS {
            assert!(drive.is_gone_from(TABLE, Some(error)), "errno {error}");
        }
    }

    /// The root filesystem, a share and a run without a destination on
    /// this computer never count as a gone drive by the mount table.
    ///
    /// parity: OPS-047
    #[test]
    fn the_root_a_share_and_other_places_are_never_gone_by_the_table() {
        let home = DestinationDrive::in_table(Some("file:///home/demo/Documents"), TABLE);
        assert!(!home.is_gone_from("", Some(ENOENT)));
        let share = DestinationDrive::in_table(Some("file:///mnt/nas/Backups"), TABLE);
        assert!(share.is_network);
        assert!(!share.is_gone_from(TABLE, None));
        for uri in [None, Some("smb://nas/share"), Some("mtp://[usb:001,010]/")] {
            let drive = DestinationDrive::in_table(uri, TABLE);
            assert!(drive.folder.is_none(), "{uri:?}");
            assert!(!drive.is_gone_from(WITHOUT_STICK, Some(ENOENT)));
        }
    }
}
