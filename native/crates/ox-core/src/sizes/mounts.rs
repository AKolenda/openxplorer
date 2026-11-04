// SPDX-License-Identifier: AGPL-3.0-only
//! The mount points of this process, so a local scan does not enter
//! another filesystem mounted inside the scanned folder.
//!
//! Ports the mount points part of `read_mounts`, `parse_mounts` and
//! `unescape_mount` in `v2.0.0:desktop/mount_support.py`. Paths are kept as bytes,
//! so mount points whose names are not UTF-8 are found too.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// The kernel's mount table of this process.
pub(crate) const MOUNT_TABLE: &str = "/proc/self/mountinfo";

/// What separates the optional fields of a mount table line from the
/// filesystem type, source and options.
const FIELD_GROUP_SEPARATOR: &[u8] = b" - ";

/// The position of the mount point among the fields before the separator:
/// mount id, parent id, `major:minor`, root, mount point.
const MOUNT_POINT_FIELD: usize = 4;

/// The position of the filesystem's `major:minor` device number among the
/// fields before the separator.
const DEVICE_FIELD: usize = 2;

/// The length of an octal escape: a backslash and three digits.
const ESCAPE_LENGTH: usize = 4;

/// One line of the mount table: where a filesystem is mounted and what
/// it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mount {
    /// The kernel's mount ID, as libmount's `utab` names the mount.
    pub(crate) id: u64,
    /// The filesystem's device number, `(major, minor)`: the block device
    /// for a filesystem on one, an anonymous number for others (Btrfs,
    /// overlays, memory filesystems).
    pub(crate) device: (u32, u32),
    /// Where it is mounted, with the kernel's escapes decoded.
    pub(crate) point: PathBuf,
    /// The filesystem type, such as `ext4` or `fuse.sshfs`.
    pub(crate) filesystem: String,
    /// What is mounted, such as `/dev/sda2`, with escapes decoded.
    pub(crate) source: Vec<u8>,
    /// The filesystem's own options, such as `rw,xattr,posixacl`.
    pub(crate) options: String,
}

/// The mount points in `/proc/self/mountinfo`.
///
/// # Errors
///
/// The error of reading the mount table.
pub(crate) fn read_mount_points() -> io::Result<HashSet<PathBuf>> {
    Ok(read_mounts()?.into_iter().map(|mount| mount.point).collect())
}

/// Every mount in `/proc/self/mountinfo`, in the table's order: a mount
/// listed later on the same mount point is on top.
///
/// # Errors
///
/// The error of reading the mount table.
pub(crate) fn read_mounts() -> io::Result<Vec<Mount>> {
    let table = fs::read(MOUNT_TABLE)?;
    Ok(parse_mounts(&table))
}

/// The mount point of each well-formed line of a mount table. Malformed
/// lines are skipped, as in Python.
#[cfg(test)]
fn parse_mount_points(table: &[u8]) -> HashSet<PathBuf> {
    parse_mounts(table).into_iter().map(|mount| mount.point).collect()
}

/// Each well-formed line of a mount table. Malformed lines are skipped.
pub(crate) fn parse_mounts(table: &[u8]) -> Vec<Mount> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(mount_of_line)
        .collect()
}

/// One mount table line, with its octal escapes (`\040` for a space)
/// decoded.
fn mount_of_line(line: &[u8]) -> Option<Mount> {
    let separator = line
        .windows(FIELD_GROUP_SEPARATOR.len())
        .position(|window| window == FIELD_GROUP_SEPARATOR)?;
    let (leading, trailing) = line.split_at(separator);
    let trailing = &trailing[FIELD_GROUP_SEPARATOR.len()..];
    let mut trailing = fields(trailing);
    let filesystem = trailing.next()?;
    let source = trailing.next()?;
    let options = trailing.next().unwrap_or_default();
    let leading: Vec<&[u8]> = fields(leading).collect();
    let escaped = leading.get(MOUNT_POINT_FIELD)?;
    let id = std::str::from_utf8(leading.first()?).ok()?.parse().ok()?;
    let (major, minor) = std::str::from_utf8(leading.get(DEVICE_FIELD)?)
        .ok()?
        .split_once(':')?;
    Some(Mount {
        id,
        device: (major.parse().ok()?, minor.parse().ok()?),
        point: PathBuf::from(OsString::from_vec(unescape_octal(escaped))),
        filesystem: String::from_utf8_lossy(&unescape_octal(filesystem)).into_owned(),
        source: unescape_octal(source),
        options: String::from_utf8_lossy(options).into_owned(),
    })
}

/// The space-separated fields of `text`. The kernel escapes only a space,
/// a tab, a newline and a backslash in a path, so any other character,
/// a carriage return included, is part of the field it is in.
fn fields(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.split(|&byte| byte == b' ').filter(|field| !field.is_empty())
}

/// Decodes the kernel's `\ooo` octal escapes, which `/etc/fstab` and
/// libmount's `utab` use too; anything else stays as it is.
pub(crate) fn unescape_octal(escaped: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(escaped.len());
    let mut position = 0;
    while position < escaped.len() {
        if let Some(byte) = escape_at(escaped, position) {
            decoded.push(byte);
            position += ESCAPE_LENGTH;
        } else {
            decoded.push(escaped[position]);
            position += 1;
        }
    }
    decoded
}

/// The byte of the octal escape at `position` in `text`, if there is one
/// and its value fits in a byte.
fn escape_at(text: &[u8], position: usize) -> Option<u8> {
    if text[position] != b'\\' {
        return None;
    }
    let digits = text.get(position + 1..position + ESCAPE_LENGTH)?;
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }
    let digits = std::str::from_utf8(digits).ok()?;
    u8::from_str_radix(digits, 8).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &[u8] = b"\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
61 22 0:52 / /mnt/nas\\040share rw,nosuid shared:33 - cifs //nas/share rw,vers=3.1.1
62 22 0:53 /sub /home/demo/Caf\xc3\xa9 rw - ext4 /dev/sdb1 rw
not a mount line
63 22 0:54 / /missing/type rw -
";

    #[test]
    fn mount_points_are_read_with_escapes_decoded() {
        let mount_points = parse_mount_points(TABLE);

        let expected: HashSet<PathBuf> = ["/", "/mnt/nas share", "/home/demo/Café"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(mount_points, expected);
    }

    #[test]
    fn each_mount_keeps_its_filesystem_source_and_options() {
        let mounts = parse_mounts(TABLE);

        assert_eq!(mounts.len(), 3);
        assert_eq!(
            mounts[1],
            Mount {
                id: 61,
                device: (0, 52),
                point: PathBuf::from("/mnt/nas share"),
                filesystem: "cifs".into(),
                source: b"//nas/share".to_vec(),
                options: "rw,vers=3.1.1".into(),
            }
        );
    }

    #[test]
    fn a_carriage_return_in_a_mount_point_stays_in_it() {
        let mounts = parse_mounts(b"70 22 0:60 / /srv/disk\rcrafted rw - fuse.demo demo rw\n");

        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].point, PathBuf::from("/srv/disk\rcrafted"));
        assert_eq!(mounts[0].filesystem, "fuse.demo");
    }

    #[test]
    fn only_complete_octal_escapes_are_decoded() {
        assert_eq!(unescape_octal(b"a\\134b\\09\\1"), b"a\\b\\09\\1");
    }

    #[test]
    fn the_mount_table_of_this_process_has_a_root() {
        let mount_points = read_mount_points().expect("the mount table is readable");

        assert!(mount_points.contains(&PathBuf::from("/")));
    }
}
