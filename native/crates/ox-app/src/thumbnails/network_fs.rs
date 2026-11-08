// SPDX-License-Identifier: AGPL-3.0-only
//! Whether a file of this computer's tree is really on another one: a
//! share mounted by the kernel (NFS, CIFS/SMB, AFS, Ceph) or a FUSE mount
//! of a network client (sshfs, rclone, the `GVfs` folder of mounts). With
//! network previews off, such a file keeps its icon wherever it is listed
//! (VIEW-058), as the folder it is in would: a recent item or a search
//! result may be listed in a folder that is not on the network.
//!
//! Asking the file system blocks, for as long as a server takes to
//! answer: run it off the main thread.

use std::path::Path;

/// `statfs`'s numbers for the kernel's network file systems: NFS, SMB,
/// CIFS, SMB2/3, AFS, kAFS and Ceph.
const NETWORK_MAGICS: [u64; 7] = [
    0x6969,
    0x517B,
    0xFF53_4D42,
    0xFE53_4D42,
    0x5346_414F,
    0x6B41_4653,
    0x00C3_6400,
];

/// `statfs`'s number for every FUSE file system, local ones included
/// (NTFS and exFAT drives, the document portal).
const FUSE_MAGIC: u64 = 0x6573_5546;

/// The FUSE file system types, as the mount table names them, of network
/// clients.
const NETWORK_FUSE_TYPES: [&str; 6] = [
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.gvfsd-fuse",
    "fuse.s3fs",
    "fuse.curlftpfs",
    "fuse.davfs",
];

/// Whether the file at `path` is on a network file system. A path that
/// cannot be asked about counts as one: its icon is shown rather than
/// anything read from it. Blocks.
pub(crate) fn is_on_network_filesystem(path: &Path) -> bool {
    let Ok(stat) = rustix::fs::statfs(path) else {
        return true;
    };
    // The magic numbers are positive; f_type's width and sign differ
    // between targets.
    let magic = u64::try_from(stat.f_type).unwrap_or_default();
    if NETWORK_MAGICS.contains(&magic) {
        return true;
    }
    if magic != FUSE_MAGIC {
        return false;
    }
    // statfs followed the links on the way; the mount table names the
    // real path's mount.
    let Ok(real) = std::fs::canonicalize(path) else {
        return true;
    };
    let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    mount_type_of(&real, &table).is_none_or(|kind| NETWORK_FUSE_TYPES.contains(&kind.as_str()))
}

/// The file system type of the mount `path` is on, from the mount table
/// `mountinfo` (`/proc/self/mountinfo`): the deepest mount point that
/// holds it. `None` when the table names none.
fn mount_type_of(path: &Path, mountinfo: &str) -> Option<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let point = unescape(fields.get(4)?);
            let separator = fields.iter().position(|field| *field == "-")?;
            let kind = fields.get(separator + 1)?;
            path.starts_with(&point)
                .then(|| (point.len(), (*kind).to_owned()))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, kind)| kind)
}

/// A mount table field with its octal escapes (`\040` for a space) undone.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let octal = bytes
            .get(at + 1..at + 4)
            .filter(|digits| bytes[at] == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit)));
        if let Some(digits) = octal {
            let value = digits
                .iter()
                .fold(0_u32, |value, digit| value * 8 + u32::from(digit - b'0'));
            out.push(u8::try_from(value).unwrap_or(b'?'));
            at += 4;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FUSE mount is a network one only for a network client's type,
    /// found by the deepest mount point holding the path; spaces in mount
    /// points are unescaped.
    ///
    /// parity: VIEW-058
    #[test]
    fn network_fuse_mounts_are_told_from_local_ones() {
        let table = "\
22 1 8:1 / / rw - ext4 /dev/sda1 rw
40 22 0:40 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
41 22 0:41 / /home/demo/Remote\\040files rw - fuse.sshfs demo@files.example: rw
42 22 8:17 / /media/demo/USB\\040stick rw - fuseblk /dev/sdb1 rw
43 22 0:43 / /run/user/1000/doc rw - fuse.portal portal rw";
        let kind = |path: &str| mount_type_of(Path::new(path), table);
        assert_eq!(
            kind("/run/user/1000/gvfs/smb-share:server=nas,share=photos/a.jpg").as_deref(),
            Some("fuse.gvfsd-fuse")
        );
        assert_eq!(
            kind("/home/demo/Remote files/b.png").as_deref(),
            Some("fuse.sshfs")
        );
        assert_eq!(kind("/media/demo/USB stick/c.jpg").as_deref(), Some("fuseblk"));
        assert_eq!(
            kind("/run/user/1000/doc/1a2b/d.jpg").as_deref(),
            Some("fuse.portal")
        );
        assert_eq!(kind("/home/demo/Pictures/e.jpg").as_deref(), Some("ext4"));
        assert!(!NETWORK_FUSE_TYPES.contains(&"fuseblk"));
        assert!(!NETWORK_FUSE_TYPES.contains(&"fuse.portal"));
    }

    /// A file of this computer's own disk is not on the network; one that
    /// cannot be asked about counts as if it were.
    ///
    /// parity: VIEW-058
    #[test]
    fn a_local_file_is_not_on_the_network() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        assert!(!is_on_network_filesystem(folder.path()));
        assert!(is_on_network_filesystem(
            &folder.path().join("missing").join("x.jpg")
        ));
    }
}
