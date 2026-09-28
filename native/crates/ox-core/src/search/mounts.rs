// SPDX-License-Identifier: AGPL-3.0-only
//! The kernel's mount table, as far as the index needs it: where each
//! filesystem is mounted and its type.
//!
//! Ports `unescape_mount`, `parse_mounts`, `read_mounts` and
//! `mount_for_path` from `desktop/mount_support.py`. Nothing here mounts
//! anything or asks for privileges.
//!
//! The table is read as bytes: the kernel escapes only space, tab, newline
//! and backslash in paths, so a mount point may be any byte string,
//! including one that is not UTF-8.

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use super::error::SearchError;

/// Where the kernel lists this process's mounts.
const MOUNT_TABLE: &str = "/proc/self/mountinfo";

/// Separates a `mountinfo` line's optional fields from the filesystem type.
const FIELDS_SEPARATOR: &[u8] = b" - ";

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

/// This process's mounts.
///
/// # Errors
///
/// [`SearchError::Io`] when the table cannot be read. Callers must not
/// take that for "no mounts": other filesystems could then not be told
/// apart (see `IndexScope::current` and `RootStorage::current`).
pub(crate) fn read_mounts() -> Result<Vec<Mount>, SearchError> {
    read_mount_table(Path::new(MOUNT_TABLE))
}

/// The mounts listed in the `mountinfo` file at `path`.
fn read_mount_table(path: &Path) -> Result<Vec<Mount>, SearchError> {
    let table = fs::read(path).map_err(|error| SearchError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    Ok(parse_mounts(&table))
}

/// The mounts in `mountinfo` text; malformed lines are skipped.
///
/// A line reads `id parent major:minor root mount-point options ... - type
/// source super-options`, with spaces and other special characters in
/// paths written as octal escapes such as `\040`.
pub(crate) fn parse_mounts(table: &[u8]) -> Vec<Mount> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(parse_mount_line)
        .collect()
}

/// The mount a `mountinfo` line describes.
fn parse_mount_line(line: &[u8]) -> Option<Mount> {
    let separator = find(line, FIELDS_SEPARATOR)?;
    let before = &line[..separator];
    let after = &line[separator + FIELDS_SEPARATOR.len()..];
    let mount_point = fields(before).nth(4)?;
    let filesystem_type = std::str::from_utf8(fields(after).next()?).ok()?;
    let path = OsString::from_vec(unescape_octal(mount_point));
    Some(Mount::new(path, filesystem_type))
}

/// The fields of a `mountinfo` line, split at whitespace as Python's
/// `str.split()` does.
fn fields(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
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
fn unescape_octal(text: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some((&first, after_first)) = rest.split_first() {
        let escaped = if first == b'\\' {
            octal_byte(after_first)
        } else {
            None
        };
        if let Some(byte) = escaped {
            decoded.push(byte);
            rest = &after_first[3..];
        } else {
            decoded.push(first);
            rest = after_first;
        }
    }
    decoded
}

/// The byte that three octal digits at the start of `text` encode; `None`
/// when they are not three octal digits or exceed a byte.
fn octal_byte(text: &[u8]) -> Option<u8> {
    let digits = text.get(..3)?;
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }
    let digits = std::str::from_utf8(digits).ok()?;
    u8::from_str_radix(digits, 8).ok()
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn mount_points_and_types_are_read_with_escapes_decoded() {
        let table = b"\
36 35 98:0 / / rw,noatime master:1 - ext4 /dev/root rw
40 36 0:50 / /mnt/My\\040Share rw shared:2 - cifs //nas/share rw,vers=3.0
malformed line";

        let mounts = parse_mounts(table);

        assert_eq!(
            mounts,
            [Mount::new("/", "ext4"), Mount::new("/mnt/My Share", "cifs")]
        );
    }

    /// A mount point that is not UTF-8 is read as it is. Reading the table
    /// as text failed on it, and the failure counted as "no mounts", so a
    /// `/` root took in every other filesystem.
    ///
    /// parity: SRCH-031
    #[test]
    fn a_mount_point_that_is_not_utf8_is_read() {
        let table = b"41 36 0:51 / /media/caf\xe9 rw - vfat /dev/sdb1 rw\n";

        let mounts = parse_mounts(table);

        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path.as_os_str().as_bytes(), b"/media/caf\xe9");
        assert_eq!(mounts[0].filesystem_type, "vfat");
    }

    #[test]
    fn a_missing_mount_table_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("mountinfo");

        let refused = read_mount_table(&missing);

        assert!(matches!(refused, Err(SearchError::Io { path, .. }) if path == missing));
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
        assert_eq!(unescape_octal(b"a\\04"), b"a\\04");
        assert_eq!(unescape_octal(b"\\134x"), b"\\x");
    }
}
