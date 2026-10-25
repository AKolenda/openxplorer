// SPDX-License-Identifier: AGPL-3.0-only
//! Which mounts the privileged walk may follow a symbolic link on.
//!
//! Owner and mode say who could have written an inode only if the
//! filesystem holding it tells the truth, and on Linux an ordinary user
//! can supply a filesystem that does not. Two ways, both cheap:
//!
//! - **A filesystem image the user authored.** Make an ext4 image whose
//!   folders and symbolic links all claim `uid 0` and mode `0755`, with
//!   a link pointing at `/etc`. Put it on a USB stick or a loop file;
//!   `udisks2` mounts it under `/run/media/<user>/<label>`. The folders
//!   above that are root-owned and closed, so a walk that only looks at
//!   owner and mode walks straight in and believes every claim the image
//!   makes.
//! - **A FUSE filesystem.** With `allow_other`, the user's own process
//!   answers every `stat` and every `readlink`, and may answer
//!   differently each time it is asked.
//!
//! So trust is a property of the mount as well as of the inode. A link
//! is followed only on a mount that nobody but root could have created
//! and whose metadata nobody but root could have authored. There is no
//! kernel flag that says that, so this reads `/proc/self/mountinfo` and
//! refuses a mount that looks like anybody's but root's:
//!
//! 1. a FUSE filesystem (`fuse`, `fuseblk`, `fuse.*`), whose answers
//!    come from a process rather than from a disk;
//! 2. a mount at or below [`USER_MOUNT_FOLDERS`], where the desktop and
//!    administrators put what somebody plugged in or asked for;
//! 3. a mount carrying an option that says a user asked for it
//!    ([`USER_MOUNT_OPTIONS`]), which is how `udisks2` and `mount`'s
//!    `user` entries mark their work;
//! 4. a mount this version cannot find or cannot parse, and every mount
//!    at all when the kernel does not report mount IDs or the table
//!    cannot be read.
//!
//! The rule refuses more than it has to on purpose. `/mnt/data` may well
//! be a root-only disk an administrator mounted by hand, and a link on
//! it will not be followed; that costs a clear refusal naming the link,
//! and the fix is to delete through the real path. The other direction
//! costs a deletion in the wrong tree, as root. **Deleting on such a
//! mount is not affected at all** — only following a symbolic link that
//! lives on one.

use std::collections::HashMap;
use std::path::Path;

use crate::sizes::mounts::{fields, unescape_octal, MOUNT_TABLE};

/// What separates the optional fields of a mount table line from the
/// filesystem type, source and super options.
const FIELD_GROUP_SEPARATOR: &[u8] = b" - ";

/// The fields before the separator: mount id, parent id, `major:minor`,
/// root, mount point, mount options.
const MOUNT_ID_FIELD: usize = 0;
const MOUNT_POINT_FIELD: usize = 4;
const MOUNT_OPTIONS_FIELD: usize = 5;

/// The fields after the separator: filesystem type, source, super options.
const FILESYSTEM_FIELD: usize = 0;
const SUPER_OPTIONS_FIELD: usize = 2;

/// Filesystem types served by a process rather than by the kernel, whose
/// answers an unprivileged user can therefore write.
const FUSE_FILESYSTEMS: [&str; 2] = ["fuse", "fuseblk"];
/// What every other FUSE filesystem's type starts with (`fuse.sshfs`).
const FUSE_PREFIX: &str = "fuse.";

/// Where a desktop or an administrator mounts what somebody plugged in
/// or asked for. A mount at or below one of these is never trusted to
/// hold a link worth following.
const USER_MOUNT_FOLDERS: [&str; 4] = ["/run/media", "/media", "/mnt", "/run/user"];

/// Mount options that say a user, not root, asked for this mount.
/// `uhelper=udisks2` is what `udisks2` writes; `user=` and `users` are
/// what `mount` records for an `/etc/fstab` entry a user may mount.
const USER_MOUNT_OPTION_PREFIXES: [&str; 2] = ["uhelper=", "user="];
const USER_MOUNT_OPTION_NAMES: [&str; 3] = ["user", "users", "owner"];

/// Which mounts of this process a symbolic link may be followed on.
#[derive(Debug, Default)]
pub(crate) struct TrustedMounts {
    /// What each mount ID the table named was judged to be. A mount that
    /// is not here could not be classified and is never trusted.
    judged: HashMap<u64, bool>,
}

impl TrustedMounts {
    /// Reads and classifies the mount table of this process. A table
    /// that cannot be read leaves every mount untrusted, which turns
    /// link-following off rather than guessing.
    pub(crate) fn read() -> Self {
        match std::fs::read(MOUNT_TABLE) {
            Ok(table) => Self::parse(&table),
            Err(_) => Self::default(),
        }
    }

    /// Classifies every well-formed line of a mount table. A malformed
    /// line is left out, so the mount it describes is untrusted.
    pub(crate) fn parse(table: &[u8]) -> Self {
        let judged = table
            .split(|&byte| byte == b'\n')
            .filter_map(judge_line)
            .collect();
        Self { judged }
    }

    /// Whether a link on the mount `id` may be followed. An unknown
    /// mount, and `None` where the kernel does not report mount IDs at
    /// all, are both untrusted.
    pub(crate) fn allows_links_on(&self, id: Option<u64>) -> bool {
        id.is_some_and(|id| self.judged.get(&id).copied().unwrap_or(false))
    }
}

/// What one mount table line says its mount is, by mount ID.
fn judge_line(line: &[u8]) -> Option<(u64, bool)> {
    let separator = line
        .windows(FIELD_GROUP_SEPARATOR.len())
        .position(|window| window == FIELD_GROUP_SEPARATOR)?;
    let (leading, trailing) = line.split_at(separator);
    let trailing = &trailing[FIELD_GROUP_SEPARATOR.len()..];
    let before: Vec<&[u8]> = fields(leading).collect();
    let after: Vec<&[u8]> = fields(trailing).collect();
    let id = std::str::from_utf8(before.get(MOUNT_ID_FIELD)?)
        .ok()?
        .parse()
        .ok()?;
    let mount_point = unescape_octal(before.get(MOUNT_POINT_FIELD)?);
    let filesystem = after.get(FILESYSTEM_FIELD)?;
    // Both option lists matter: `mount` records a user's request in the
    // per-mount options, and some filesystems record it in the super
    // options instead. Super options are absent on some lines.
    let options = [
        before.get(MOUNT_OPTIONS_FIELD).copied().unwrap_or_default(),
        after.get(SUPER_OPTIONS_FIELD).copied().unwrap_or_default(),
    ];
    let trusted = !is_fuse(filesystem)
        && !is_user_mount_folder(&mount_point)
        && !options.iter().any(|options| asked_for_by_a_user(options));
    Some((id, trusted))
}

/// Whether a filesystem type is served by a process.
fn is_fuse(filesystem: &[u8]) -> bool {
    let Ok(filesystem) = std::str::from_utf8(filesystem) else {
        // A type this version cannot even read is one it cannot vouch for.
        return true;
    };
    FUSE_FILESYSTEMS.contains(&filesystem) || filesystem.starts_with(FUSE_PREFIX)
}

/// Whether `mount_point` is one of the folders a desktop or an
/// administrator mounts removable and on-request filesystems in, or
/// anything inside one.
fn is_user_mount_folder(mount_point: &[u8]) -> bool {
    let mount_point = Path::new(std::str::from_utf8(mount_point).unwrap_or("/"));
    USER_MOUNT_FOLDERS
        .iter()
        .any(|folder| mount_point.starts_with(folder))
}

/// Whether a comma-separated option list says a user asked for the mount.
fn asked_for_by_a_user(options: &[u8]) -> bool {
    let Ok(options) = std::str::from_utf8(options) else {
        return true;
    };
    options.split(',').any(|option| {
        USER_MOUNT_OPTION_NAMES.contains(&option)
            || USER_MOUNT_OPTION_PREFIXES
                .iter()
                .any(|prefix| option.starts_with(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table with the shapes that matter: the root filesystem, a
    /// root-mounted disk, a stick `udisks2` mounted, a FUSE filesystem,
    /// an `/etc/fstab` entry a user may mount, and a mount point with a
    /// space in it.
    const TABLE: &[u8] = b"\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
23 22 8:3 / /srv/data rw,relatime shared:2 - ext4 /dev/sdb1 rw
24 22 0:45 / /run/media/demo/STICK rw,nosuid,nodev,relatime,uhelper=udisks2 shared:3 - ext4 /dev/sdc1 rw
25 22 0:46 / /home/demo/remote rw,nosuid,nodev,relatime shared:4 - fuse.sshfs demo@host: rw,user_id=1000,allow_other
26 22 0:47 / /srv/scratch rw,relatime shared:5 - ext4 /dev/sdd1 rw,user=demo
27 22 8:4 / /mnt/backup\\040disk rw,relatime shared:6 - ext4 /dev/sde1 rw
28 22 0:48 / /srv/share rw,relatime shared:7 - fuseblk /dev/sdf1 rw
29 22 8:5 / /opt/tools rw,relatime shared:8 - ext4 /dev/sdg1 rw,users
";

    /// parity: XFER-015
    #[test]
    fn a_root_mounted_filesystem_is_the_only_kind_a_link_may_be_followed_on() {
        let mounts = TrustedMounts::parse(TABLE);

        // The root filesystem and a disk root mounted outside the user
        // mount folders.
        assert!(mounts.allows_links_on(Some(22)), "the root filesystem");
        assert!(
            mounts.allows_links_on(Some(23)),
            "a disk root mounted at /srv/data"
        );
        // Everything a user could have supplied or asked for.
        assert!(!mounts.allows_links_on(Some(24)), "a stick udisks2 mounted");
        assert!(!mounts.allows_links_on(Some(25)), "a FUSE filesystem");
        assert!(!mounts.allows_links_on(Some(26)), "a mount with user=");
        assert!(!mounts.allows_links_on(Some(27)), "a mount under /mnt");
        assert!(!mounts.allows_links_on(Some(28)), "a fuseblk filesystem");
        assert!(!mounts.allows_links_on(Some(29)), "a mount with users");
    }

    /// Anything the walk cannot place is untrusted: a mount the table
    /// does not name, and a kernel too old to report mount IDs.
    ///
    /// parity: XFER-015
    #[test]
    fn a_mount_that_cannot_be_classified_is_never_trusted() {
        let mounts = TrustedMounts::parse(TABLE);

        assert!(
            !mounts.allows_links_on(Some(999)),
            "a mount the table does not name"
        );
        assert!(!mounts.allows_links_on(None), "no mount ID from the kernel");
        assert!(
            !TrustedMounts::default().allows_links_on(Some(22)),
            "an unreadable mount table"
        );
    }

    /// A malformed line is skipped, so its mount stays untrusted rather
    /// than inheriting another line's verdict.
    ///
    /// parity: XFER-015
    #[test]
    fn a_malformed_line_leaves_its_mount_untrusted() {
        let mounts = TrustedMounts::parse(b"22 1 8:2 / / rw shared:1 ext4 /dev/sda2 rw\n30 1\n");

        assert!(!mounts.allows_links_on(Some(22)), "no field separator");
        assert!(!mounts.allows_links_on(Some(30)), "too few fields");
    }

    /// `user_xattr` is an ordinary ext4 super option and says nothing
    /// about who mounted the filesystem; only `user=`, `users`, `owner`
    /// and `uhelper=` do.
    ///
    /// parity: XFER-015
    #[test]
    fn an_everyday_option_that_merely_starts_with_user_is_not_a_user_mount() {
        let table = b"31 22 8:6 / /srv/plain rw,relatime shared:9 - ext4 /dev/sdh1 rw,user_xattr\n";

        assert!(TrustedMounts::parse(table).allows_links_on(Some(31)));
    }

    /// parity: XFER-015
    #[test]
    fn a_mount_point_with_a_space_is_read_through_its_escape() {
        // /mnt/backup disk is under /mnt, which the escape has to be
        // decoded to see.
        assert!(!TrustedMounts::parse(TABLE).allows_links_on(Some(27)));
    }
}
