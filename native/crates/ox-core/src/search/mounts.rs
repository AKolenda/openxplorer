// SPDX-License-Identifier: AGPL-3.0-only
//! The kernel's mount table, as far as the index needs it: where each
//! filesystem is mounted and its type.
//!
//! Ports `unescape_mount`, `parse_mounts`, `read_mounts` and
//! `mount_for_path` from `desktop/mount_support.py`. Nothing here mounts
//! anything or asks for privileges.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the kernel lists this process's mounts.
const MOUNT_TABLE: &str = "/proc/self/mountinfo";

/// One mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mount {
    /// Where it is mounted.
    pub(crate) path: PathBuf,
    /// The filesystem type, for example `ext4` or `cifs`.
    pub(crate) filesystem_type: String,
}

impl Mount {
    /// A mount at `path` of `filesystem_type`.
    pub(crate) fn new(path: impl Into<PathBuf>, filesystem_type: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            filesystem_type: filesystem_type.into(),
        }
    }
}

/// This process's mounts. An unreadable table counts as no mounts, so
/// nothing is excluded or polled because of a mount.
pub(crate) fn read_mounts() -> Vec<Mount> {
    fs::read_to_string(MOUNT_TABLE)
        .map(|table| parse_mounts(&table))
        .unwrap_or_default()
}

/// The mounts in `mountinfo` text; malformed lines are skipped.
///
/// A line reads `id parent major:minor root mount-point options ... - type
/// source super-options`, with spaces and other special characters in
/// paths written as octal escapes such as `\040`.
pub(crate) fn parse_mounts(table: &str) -> Vec<Mount> {
    table.lines().filter_map(parse_mount_line).collect()
}

/// The mount a `mountinfo` line describes.
fn parse_mount_line(line: &str) -> Option<Mount> {
    let (before, after) = line.split_once(" - ")?;
    let mount_point = before.split_whitespace().nth(4)?;
    let filesystem_type = after.split_whitespace().next()?;
    Some(Mount::new(unescape_octal(mount_point), filesystem_type))
}

/// The mount that holds `path`: the one with the longest mount point at
/// or above it.
pub(crate) fn mount_for_path<'a>(path: &Path, mounts: &'a [Mount]) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.path))
        .max_by_key(|mount| mount.path.as_os_str().len())
}

/// Decodes the `\ooo` escapes of a `mountinfo` path.
fn unescape_octal(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(backslash) = rest.find('\\') {
        decoded.push_str(&rest[..backslash]);
        let escape = &rest[backslash + 1..];
        if let Some(character) = octal_character(escape) {
            decoded.push(character);
            rest = &escape[3..];
        } else {
            decoded.push('\\');
            rest = escape;
        }
    }
    decoded.push_str(rest);
    decoded
}

/// The character three octal digits at the start of `text` encode.
fn octal_character(text: &str) -> Option<char> {
    let digits = text.get(..3)?;
    if !digits.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return None;
    }
    let code = u32::from_str_radix(digits, 8).ok()?;
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_points_and_types_are_read_with_escapes_decoded() {
        let table = "\
36 35 98:0 / / rw,noatime master:1 - ext4 /dev/root rw
40 36 0:50 / /mnt/My\\040Share rw shared:2 - cifs //nas/share rw,vers=3.0
malformed line";

        let mounts = parse_mounts(table);

        assert_eq!(
            mounts,
            [Mount::new("/", "ext4"), Mount::new("/mnt/My Share", "cifs")]
        );
    }

    #[test]
    fn the_deepest_mount_holds_a_path() {
        let mounts = [Mount::new("/", "ext4"), Mount::new("/mnt/data", "nfs4")];

        let holder = mount_for_path(Path::new("/mnt/data/projects"), &mounts);
        let other = mount_for_path(Path::new("/mnt/data2"), &mounts);

        assert_eq!(holder.map(|mount| mount.filesystem_type.as_str()), Some("nfs4"));
        assert_eq!(other.map(|mount| mount.filesystem_type.as_str()), Some("ext4"));
    }

    #[test]
    fn an_incomplete_escape_is_kept() {
        assert_eq!(unescape_octal("a\\04"), "a\\04");
        assert_eq!(unescape_octal("\\134x"), "\\x");
    }
}
