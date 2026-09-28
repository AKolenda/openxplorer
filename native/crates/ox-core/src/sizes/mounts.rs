// SPDX-License-Identifier: AGPL-3.0-only
//! The mount points of this process, so a local scan does not enter
//! another filesystem mounted inside the scanned folder.
//!
//! Ports the mount points part of `read_mounts`, `parse_mounts` and
//! `unescape_mount` in `desktop/mount_support.py`. Paths are kept as bytes,
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

/// The fields after the separator a line must have: filesystem type and
/// source.
const REQUIRED_TRAILING_FIELDS: usize = 2;

/// The length of an octal escape: a backslash and three digits.
const ESCAPE_LENGTH: usize = 4;

/// The mount points in `/proc/self/mountinfo`.
///
/// # Errors
///
/// The error of reading the mount table.
pub(crate) fn read_mount_points() -> io::Result<HashSet<PathBuf>> {
    let table = fs::read(MOUNT_TABLE)?;
    Ok(parse_mount_points(&table))
}

/// The mount point of each well-formed line of a mount table. Malformed
/// lines are skipped, as in Python.
fn parse_mount_points(table: &[u8]) -> HashSet<PathBuf> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(mount_point_of_line)
        .collect()
}

/// The mount point of one mount table line, with its octal escapes
/// (`\040` for a space) decoded.
fn mount_point_of_line(line: &[u8]) -> Option<PathBuf> {
    let separator = line
        .windows(FIELD_GROUP_SEPARATOR.len())
        .position(|window| window == FIELD_GROUP_SEPARATOR)?;
    let (leading, trailing) = line.split_at(separator);
    let trailing = &trailing[FIELD_GROUP_SEPARATOR.len()..];
    if fields(trailing).count() < REQUIRED_TRAILING_FIELDS {
        return None;
    }
    let escaped = fields(leading).nth(MOUNT_POINT_FIELD)?;
    Some(PathBuf::from(OsString::from_vec(unescape_octal(escaped))))
}

/// The whitespace-separated fields of `text`.
fn fields(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
}

/// Decodes the kernel's `\ooo` octal escapes; anything else stays as it is.
fn unescape_octal(escaped: &[u8]) -> Vec<u8> {
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
    fn only_complete_octal_escapes_are_decoded() {
        assert_eq!(unescape_octal(b"a\\134b\\09\\1"), b"a\\b\\09\\1");
    }

    #[test]
    fn the_mount_table_of_this_process_has_a_root() {
        let mount_points = read_mount_points().expect("the mount table is readable");

        assert!(mount_points.contains(&PathBuf::from("/")));
    }
}
