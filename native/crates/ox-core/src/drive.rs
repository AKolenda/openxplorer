// SPDX-License-Identifier: AGPL-3.0-only
//! Which local drive an item is on, which decides what a plain drag does
//! (DND-017): Windows Explorer moves items dragged within a drive and
//! copies them to another drive or to and from a network place.
//!
//! A drive is one mount of a local filesystem: its device number and,
//! where the kernel reports it (Linux 5.8 and later), its mount ID. Two
//! mount points of one filesystem (a bind mount) share a device number,
//! but a rename between them fails, so both must match. A kernel mount of
//! a network share (CIFS/SMB3, NFS and the like) or a FUSE mount other
//! than a local disk's (`fuseblk`, as ntfs-3g mounts one) is a network
//! place, never a drive, so a drag there, or from there, copies.
//!
//! An overlay filesystem (overlayfs, as containers and live sessions use)
//! is one drive, but the kernel refuses to rename a folder on it (EXDEV)
//! unless it was mounted with `redirect_dir=on`, so a folder dragged there
//! counts as going to another drive and is copied, as a plain drag to
//! another drive is.

use std::path::Path;

use rustix::fs::{AtFlags, StatxFlags, CWD};

use crate::network::{mount_for_path, parse_mount_table};

/// This process's mount table.
const MOUNT_INFO: &str = "/proc/self/mountinfo";

/// What separates the optional fields of a mount table line from the
/// filesystem type.
const SEPARATOR: &str = " - ";

/// The drive an item is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drive {
    /// The kernel's mount ID, when it reports one.
    mount: Option<u64>,
    /// The device number of the filesystem.
    device: u64,
    /// Whether the filesystem is a network place rather than a drive.
    is_network: bool,
    /// A folder the kernel refuses to move by renaming, on an overlay
    /// filesystem without `redirect_dir=on`.
    is_unrenamable_folder: bool,
}

impl Drive {
    /// True when the item on this drive and the folder on `other` are on
    /// the same local drive, so a rename can move the item into it: the
    /// same filesystem through the same mount, neither a network place,
    /// and not a folder an overlay filesystem refuses to rename.
    pub fn is_same_local_drive(&self, other: &Drive) -> bool {
        let same_mount = match (self.mount, other.mount) {
            (Some(own), Some(theirs)) => own == theirs,
            _ => true,
        };
        !self.is_network
            && !other.is_network
            && !self.is_unrenamable_folder
            && self.device == other.device
            && same_mount
    }
}

/// The drive of the item at `path`, or `None` when it cannot be read. A
/// symbolic link is on the drive of the folder holding it unless `follow`
/// is true, when it is where the link points. Never triggers an automount.
/// A path the mount table places on a network mount is not read. It
/// reads file metadata, which can block on a share that stopped
/// answering, so call it off the GTK thread.
pub fn drive_of(path: &Path, follow: bool) -> Option<Drive> {
    let table = std::fs::read_to_string(MOUNT_INFO).ok();
    if table
        .as_deref()
        .is_some_and(|table| is_below_network_mount(table, path))
    {
        return Some(Drive {
            mount: None,
            device: 0,
            is_network: true,
            is_unrenamable_folder: false,
        });
    }
    let mut flags = AtFlags::NO_AUTOMOUNT;
    if !follow {
        flags |= AtFlags::SYMLINK_NOFOLLOW;
    }
    let (mount, device, mode) = match rustix::fs::statx(CWD, path, flags, StatxFlags::MNT_ID) {
        Ok(status) => {
            let has_id = status.stx_mask & StatxFlags::MNT_ID.bits() != 0;
            let device = rustix::fs::makedev(status.stx_dev_major, status.stx_dev_minor);
            (
                has_id.then_some(status.stx_mnt_id),
                device,
                u32::from(status.stx_mode),
            )
        }
        Err(rustix::io::Errno::NOSYS | rustix::io::Errno::PERM) => {
            let status = rustix::fs::statat(CWD, path, flags).ok()?;
            (None, status.st_dev, status.st_mode)
        }
        Err(_) => return None,
    };
    let is_folder = rustix::fs::FileType::from_raw_mode(mode) == rustix::fs::FileType::Directory;
    let filesystem = mount
        .zip(table.as_deref())
        .and_then(|(mount, table)| filesystem_of_mount(table, mount));
    // Without the mount table, the filesystem's magic number tells: the
    // folder holding a link that is not followed decides.
    let (is_network, is_unrenamable_folder) = if let Some((filesystem, options)) = filesystem {
        let refuses = is_folder && refuses_folder_renames(&filesystem, &options);
        (is_network_filesystem(&filesystem), refuses)
    } else {
        let folder = if follow { Some(path) } else { path.parent() };
        let status = folder.and_then(|folder| rustix::fs::statfs(folder).ok());
        // `f_type` is narrower than i64 on some targets.
        #[allow(clippy::useless_conversion)]
        let magic = status.map(|status| i64::from(status.f_type));
        let is_network = magic.is_none_or(is_network_magic);
        // Without its options, an overlay is taken to refuse, as it does
        // by default.
        (is_network, is_folder && magic == Some(OVERLAY_MAGIC))
    };
    Some(Drive {
        mount,
        device,
        is_network,
        is_unrenamable_folder,
    })
}

/// True when `path` lies, by its name alone, on a network mount of the
/// mount table `table`, so nothing needs to read it: reading a path on a
/// share that stopped answering can block for long.
fn is_below_network_mount(table: &str, path: &Path) -> bool {
    let Some(text) = path.to_str().filter(|_| path.is_absolute()) else {
        return false;
    };
    let mounts = parse_mount_table(table);
    mount_for_path(text, &mounts).is_some_and(|mount| is_network_filesystem(&mount.filesystem))
}

/// The filesystem type of the mount `mount` in the mount table `table`,
/// and its filesystem's options (the line's last field).
fn filesystem_of_mount(table: &str, mount: u64) -> Option<(String, String)> {
    table.lines().find_map(|line| {
        let id: u64 = line.split_whitespace().next()?.parse().ok()?;
        if id != mount {
            return None;
        }
        let (_, after) = line.split_once(SEPARATOR)?;
        let mut fields = after.split_whitespace();
        let filesystem = fields.next()?.to_owned();
        let options = fields.nth(1).unwrap_or_default().to_owned();
        Some((filesystem, options))
    })
}

/// `statfs`'s `f_type` of an overlay filesystem.
const OVERLAY_MAGIC: i64 = 0x794C_7630;

/// True when the filesystem `filesystem`, mounted with the filesystem
/// options `options`, refuses to rename a folder (EXDEV): an overlay,
/// unless `redirect_dir=on` lets it.
fn refuses_folder_renames(filesystem: &str, options: &str) -> bool {
    filesystem == "overlay" && !options.split(',').any(|option| option == "redirect_dir=on")
}

/// True for the filesystem types of network places: kernel network
/// filesystems, and FUSE mounts other than a local disk's (`fuseblk`),
/// such as SSHFS, rclone or `GVfs`.
fn is_network_filesystem(filesystem: &str) -> bool {
    const NETWORK: &[&str] = &[
        "cifs",
        "smb3",
        "smbfs",
        "nfs",
        "nfs4",
        "afs",
        "ceph",
        "9p",
        "glusterfs",
        "lustre",
        "davfs",
        "ncpfs",
        "coda",
        "fuse",
    ];
    NETWORK.contains(&filesystem) || filesystem.starts_with("fuse.")
}

/// True for the magic numbers (`statfs`'s `f_type`) of network and FUSE
/// filesystems, for when the mount table cannot tell.
fn is_network_magic(magic: i64) -> bool {
    const NETWORK: &[i64] = &[
        0xFF53_4D42, // CIFS
        0xFE53_4D42, // SMB2 and SMB3
        0x517B,      // SMB
        0x6969,      // NFS
        0x5346_414F, // AFS
        0x00C3_6400, // Ceph
        0x0102_1997, // 9P
        0x6573_5546, // FUSE
        0x7375_7245, // Coda
        0x564C,      // NCP
    ];
    NETWORK.contains(&magic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local drive with mount ID `mount` and device number `device`.
    fn local(mount: Option<u64>, device: u64) -> Drive {
        Drive {
            mount,
            device,
            is_network: false,
            is_unrenamable_folder: false,
        }
    }

    /// parity: DND-017
    #[test]
    fn items_in_one_folder_are_on_one_local_drive() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("Notes.txt");
        std::fs::write(&item, "notes").expect("the item is written");

        let folder_drive = drive_of(folder.path(), true).expect("the folder is read");
        let item_drive = drive_of(&item, false).expect("the item is read");

        assert!(item_drive.is_same_local_drive(&folder_drive));
        assert_eq!(drive_of(&folder.path().join("Gone.txt"), false), None);
    }

    /// Two mount points of one filesystem share its device number, but a
    /// rename between them fails, so they are two drives.
    ///
    /// parity: DND-017
    #[test]
    fn a_bind_mount_of_the_same_filesystem_is_another_drive() {
        assert!(!local(Some(41), 7).is_same_local_drive(&local(Some(42), 7)));
        assert!(local(Some(41), 7).is_same_local_drive(&local(Some(41), 7)));
        assert!(
            !local(Some(41), 7).is_same_local_drive(&local(Some(41), 8)),
            "a subvolume on one mount has its own device number"
        );
        assert!(
            local(None, 7).is_same_local_drive(&local(Some(42), 7)),
            "without mount IDs the device number decides"
        );
    }

    /// A kernel mount of a share is a network place, so a drag within it
    /// copies, as a drag within a network folder does.
    ///
    /// parity: DND-017
    #[test]
    fn a_network_mount_is_never_a_drive_to_move_on() {
        let share = Drive {
            mount: Some(50),
            device: 60,
            is_network: true,
            is_unrenamable_folder: false,
        };
        assert!(!share.is_same_local_drive(&share));
    }

    /// parity: DND-017
    #[test]
    fn network_filesystems_are_told_from_local_ones() {
        for network in ["cifs", "smb3", "nfs", "nfs4", "fuse.sshfs", "fuse.rclone", "fuse"] {
            assert!(is_network_filesystem(network), "{network}");
        }
        for local in [
            "ext4", "btrfs", "xfs", "tmpfs", "vfat", "exfat", "ntfs3", "fuseblk",
        ] {
            assert!(!is_network_filesystem(local), "{local}");
        }
        assert!(is_network_magic(0xFF53_4D42), "CIFS");
        assert!(!is_network_magic(0xEF53), "ext4");
    }

    /// parity: DND-017
    #[test]
    fn the_mount_table_names_each_mounts_filesystem() {
        let table = "\
22 1 0:21 / /proc rw,nosuid shared:5 - proc proc rw
41 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
97 41 0:55 / /mnt/share rw,relatime shared:60 - cifs //nas/share rw,vers=3.1.1
98 41 8:2 /srv /mnt/srv rw - ext4 /dev/sda2 rw
";
        let filesystem = |mount| filesystem_of_mount(table, mount).map(|(filesystem, _)| filesystem);
        assert_eq!(filesystem(97).as_deref(), Some("cifs"));
        assert_eq!(filesystem(98).as_deref(), Some("ext4"));
        assert_eq!(filesystem(99), None);
        assert_eq!(
            filesystem_of_mount(table, 97)
                .map(|(_, options)| options)
                .as_deref(),
            Some("rw,vers=3.1.1")
        );
    }

    /// A folder on an overlay filesystem (containers, live sessions) is
    /// on the same drive as the folder it is dragged to, but the kernel
    /// refuses to rename it (EXDEV), so a plain drag copies it, as to
    /// another drive, instead of asking about moving by copying. Files on
    /// it, and folders on an overlay mounted with `redirect_dir=on`, move.
    ///
    /// parity: DND-017
    #[test]
    fn a_folder_on_an_overlay_is_copied_by_a_plain_drag() {
        let table = "\
41 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
52 41 0:42 / /work rw,relatime - overlay overlay rw,lowerdir=/l,upperdir=/u,workdir=/w,uuid=on
53 41 0:43 / /work2 rw,relatime - overlay overlay rw,lowerdir=/l,upperdir=/u,workdir=/w,redirect_dir=on
";
        let refuses = |mount| {
            let (filesystem, options) = filesystem_of_mount(table, mount).expect("a mount");
            refuses_folder_renames(&filesystem, &options)
        };
        assert!(refuses(52));
        assert!(!refuses(53), "redirect_dir=on renames folders");
        assert!(!refuses(41));

        let folder = Drive {
            is_unrenamable_folder: true,
            ..local(Some(52), 42)
        };
        let destination = Drive {
            is_unrenamable_folder: true,
            ..local(Some(52), 42)
        };
        assert!(!folder.is_same_local_drive(&destination));
        assert!(
            local(Some(52), 42).is_same_local_drive(&destination),
            "a file moves into the folder"
        );
    }

    /// A path on a share the mount table already names is a network place
    /// without being read, so a share that stopped answering is never
    /// asked.
    ///
    /// parity: DND-017
    #[test]
    fn a_path_on_a_known_network_mount_is_told_by_its_name() {
        let table = "\
41 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
97 41 0:55 / /mnt/share rw,relatime shared:60 - cifs //nas/share rw,vers=3.1.1
98 41 8:2 /srv /mnt/srv rw - ext4 /dev/sda2 rw
99 41 0:60 / /mnt/my\\040files rw - fuse.sshfs host:/files rw
";
        let on = |path: &str| is_below_network_mount(table, Path::new(path));

        assert!(on("/mnt/share"));
        assert!(on("/mnt/share/Docs/Notes.txt"));
        assert!(on("/mnt/my files/Notes.txt"), "escaped mount points");
        assert!(!on("/mnt/shared/Notes.txt"), "only below the mount point");
        assert!(!on("/mnt/srv/Notes.txt"));
        assert!(!on("/home/user/Notes.txt"));
    }
}
