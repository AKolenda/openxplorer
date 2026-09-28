// SPDX-License-Identifier: AGPL-3.0-only
//! The kernel's mount table, and SMB shares mounted in it.
//!
//! Ports `unescape_mount`, `parse_mounts`, `read_mounts`, `is_below`,
//! `mount_for_path`, `remote_root` and `resolve_smb_path` in
//! `desktop/mount_support.py`, and the `stable` mounts `environment` in
//! `desktop/winspace.py` hands to the Network list. Reading the table
//! never mounts anything.

use std::fs;
use std::io;
use std::path::PathBuf;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use crate::location::{normalise, require_share, split_location, unquote_lossy, LocationError};
use crate::places::StableMount;

/// This process's mount table.
const MOUNT_INFO: &str = "/proc/self/mountinfo";

/// Characters Python's `quote(path, safe='/')` leaves alone: letters,
/// digits, `_.-~` and `/`.
const PATH_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

/// One line of `/proc/self/mountinfo`, unescaped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountEntry {
    /// The directory of the filesystem that is mounted; `/` unless only a
    /// subdirectory is bound here.
    pub root: String,
    /// Where it is mounted.
    pub path: String,
    /// The filesystem type, for example `cifs`.
    pub filesystem: String,
    /// What is mounted, for example `//nas/share`.
    pub source: String,
    /// The superblock options.
    pub options: String,
}

impl MountEntry {
    /// True for kernel SMB mounts (`cifs` and `smb3`).
    pub fn is_smb(&self) -> bool {
        matches!(self.filesystem.as_str(), "cifs" | "smb3")
    }

    /// This mount as a [`StableMount`] of the Network list, or `None` for
    /// a mount that is not a kernel SMB mount. The mount table names no
    /// label, so the row is named after the mount point.
    pub fn to_stable_mount(&self) -> Option<StableMount> {
        if !self.is_smb() {
            return None;
        }
        Some(StableMount {
            path: PathBuf::from(&self.path),
            label: String::new(),
            filesystem: self.filesystem.clone(),
        })
    }

    /// The SMB location mounted here: the share, or the folder of a bind
    /// mount of a subfolder. `None` for other filesystems and unreadable
    /// sources.
    pub fn remote_root(&self) -> Option<String> {
        if !self.is_smb() {
            return None;
        }
        let share = require_share(&self.source).ok()?;
        if self.root == "/" || self.root.is_empty() {
            return Some(share);
        }
        let subfolder = utf8_percent_encode(self.root.trim_start_matches('/'), PATH_SAFE);
        let bound_folder = format!("{}/{subfolder}", share.trim_end_matches('/'));
        normalise(&bound_folder).ok()
    }
}

/// Parses mount table `text`, skipping malformed lines.
pub fn parse_mount_table(text: &str) -> Vec<MountEntry> {
    text.lines().filter_map(parse_mount_line).collect()
}

/// Reads this process's mount table.
///
/// # Errors
///
/// The I/O error of reading `/proc/self/mountinfo`.
pub fn read_mount_table() -> io::Result<Vec<MountEntry>> {
    let text = fs::read_to_string(MOUNT_INFO)?;
    Ok(parse_mount_table(&text))
}

/// The kernel SMB mounts of this process's mount table, as the Network
/// list and Quick access read them (`stable` in `environment`).
///
/// # Errors
///
/// The I/O error of reading `/proc/self/mountinfo`.
pub fn read_stable_smb_mounts() -> io::Result<Vec<StableMount>> {
    let mounts = read_mount_table()?;
    Ok(mounts.iter().filter_map(MountEntry::to_stable_mount).collect())
}

/// One mountinfo line: `id parent major:minor root path options
/// [optional fields] - type source superblock-options`.
fn parse_mount_line(line: &str) -> Option<MountEntry> {
    let (mount_fields, filesystem_fields) = line.split_once(" - ")?;
    let mount_fields: Vec<&str> = mount_fields.split_whitespace().collect();
    let mut filesystem_fields = filesystem_fields.split_whitespace();
    Some(MountEntry {
        root: unescape_mount_field(mount_fields.get(3)?),
        path: unescape_mount_field(mount_fields.get(4)?),
        filesystem: filesystem_fields.next()?.to_owned(),
        source: unescape_mount_field(filesystem_fields.next()?),
        options: filesystem_fields.next().unwrap_or_default().to_owned(),
    })
}

/// Undoes the kernel's octal escapes (`\040` for a space), as
/// `unescape_mount` does: each becomes the character with that code.
fn unescape_mount_field(field: &str) -> String {
    let mut unescaped = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(backslash) = rest.find('\\') {
        unescaped.push_str(&rest[..backslash]);
        let escape = &rest[backslash + 1..];
        if let Some(character) = octal_character(escape) {
            unescaped.push(character);
            // The three digits are ASCII, so byte 3 is a character boundary.
            rest = &escape[3..];
        } else {
            unescaped.push('\\');
            rest = escape;
        }
    }
    unescaped.push_str(rest);
    unescaped
}

/// The character of three leading octal digits of `text`.
fn octal_character(text: &str) -> Option<char> {
    let digits = text.get(..3)?;
    if !digits.bytes().all(|digit| (b'0'..=b'7').contains(&digit)) {
        return None;
    }
    let code = u32::from_str_radix(digits, 8).ok()?;
    char::from_u32(code)
}

/// True when `path` is `root` or lies below it.
fn is_at_or_below(path: &str, root: &str) -> bool {
    let prefix = format!("{}/", root.trim_end_matches('/'));
    path == root || path.starts_with(&prefix)
}

/// The mount that holds `path`: the one with the longest mount point at
/// or above it.
pub fn mount_for_path<'a>(path: &str, mounts: &'a [MountEntry]) -> Option<&'a MountEntry> {
    let mut holding: Option<&MountEntry> = None;
    for mount in mounts.iter().filter(|mount| is_at_or_below(path, &mount.path)) {
        let is_longer = holding.is_none_or(|best| mount.path.len() > best.path.len());
        if is_longer {
            holding = Some(mount);
        }
    }
    holding
}

/// The local path of SMB location `uri` inside a kernel SMB mount: the
/// most specific mount of its share (bind mounts of subfolders included).
/// Host and share names compare case-insensitively; the path below keeps
/// its case.
///
/// # Errors
///
/// The [`LocationError`] of an address that is not an SMB shared folder.
pub fn resolve_smb_path(uri: &str, mounts: &[MountEntry]) -> Result<Option<PathBuf>, LocationError> {
    let wanted = SmbPath::parse(&require_share(uri)?)?;
    let mut best: Option<(usize, PathBuf)> = None;
    for mount in mounts {
        let Some(mounted) = mount.remote_root().and_then(|root| SmbPath::parse(&root).ok()) else {
            continue;
        };
        let Some(below) = wanted.components_below(&mounted) else {
            continue;
        };
        let depth = mounted.components.len();
        let is_more_specific = best.as_ref().is_none_or(|(best_depth, _)| depth > *best_depth);
        if is_more_specific {
            best = Some((depth, join_below(&mount.path, below)));
        }
    }
    Ok(best.map(|(_, local)| local))
}

/// `mount_path` followed by the `below` components, as `os.path.join`.
fn join_below(mount_path: &str, below: &[String]) -> PathBuf {
    let mut local = PathBuf::from(mount_path);
    for component in below {
        local.push(component);
    }
    local
}

/// An SMB location as `resolve_smb_path` compares it.
struct SmbPath {
    /// The authority, lower-cased (`netloc.lower()`).
    authority: String,
    /// The decoded path components; the first is the share.
    components: Vec<String>,
}

impl SmbPath {
    fn parse(uri: &str) -> Result<Self, LocationError> {
        let parts = split_location(uri.trim_end_matches('/'))?;
        let decoded = unquote_lossy(&parts.path);
        let components = decoded.trim_matches('/').split('/').map(str::to_owned).collect();
        Ok(Self {
            authority: parts.authority.to_lowercase(),
            components,
        })
    }

    /// The components of `self` below the mounted location `mounted`, or
    /// `None` when `self` is not inside it.
    fn components_below(&self, mounted: &SmbPath) -> Option<&[String]> {
        let (share, folders) = self.components.split_first()?;
        let (mounted_share, mounted_folders) = mounted.components.split_first()?;
        let same_share = glib::casefold(share) == glib::casefold(mounted_share);
        if self.authority != mounted.authority || !same_share {
            return None;
        }
        let below = folders.strip_prefix(mounted_folders)?;
        Some(below)
    }
}

#[cfg(test)]
mod tests;
