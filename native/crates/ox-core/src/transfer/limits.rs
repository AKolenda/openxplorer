// SPDX-License-Identifier: AGPL-3.0-only
//! What the destination's file system can hold (XFER-028): its free space,
//! the FAT file size limit, and whether it stores names with the
//! characters Windows file systems forbid and symbolic links.
//!
//! Beyond the Python app, which showed only the raw GIO error of the item
//! that failed. Follows KIO's `CopyJob` (`ERR_DISK_FULL` after the stat
//! phase, `ERR_FILE_TOO_LARGE_FOR_FAT32`, `handleMsdosFsQuirks`). The
//! questions about names and links are in `unstorable`.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::guard::MAX_DEPTH;
use super::names::child_node;
use super::node::{Node, NodeFactory, NodeKind};
use super::types::{ConflictPolicy, TransferMode};
use crate::format::pretty_bytes;

/// The largest file FAT can store: 4 GiB less one byte.
pub const FAT_MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// The printable characters FAT, exFAT and NTFS forbid in names.
const FORBIDDEN_CHARACTERS: &[u8] = b"\"*:<>?\\|";

/// The file system a folder is on, as far as transfers need it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesystemInfo {
    /// GIO's `filesystem::type`, for example `ext4`, `msdos` or `exfat`.
    pub kind: Option<String>,
    /// The free bytes.
    pub free: Option<u64>,
    /// GIO's `id::filesystem`: two items with the same id are on one file
    /// system, so moving between them is a rename.
    pub id: Option<String>,
}

/// What a destination file system cannot store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StorageRules {
    /// The largest file it stores, when it has a limit (FAT).
    pub(crate) max_file_size: Option<u64>,
    /// True when it forbids [`FORBIDDEN_CHARACTERS`] and control
    /// characters in names (FAT, exFAT, NTFS).
    pub(crate) restricts_names: bool,
    /// False when it cannot store symbolic links (FAT, exFAT).
    pub(crate) stores_links: bool,
}

impl Default for StorageRules {
    /// A file system that stores everything, such as ext4 or Btrfs.
    fn default() -> Self {
        Self {
            max_file_size: None,
            restricts_names: false,
            stores_links: true,
        }
    }
}

impl StorageRules {
    /// The rules of the file system GIO names `kind`. `fuseblk` is any
    /// FUSE block file system: NTFS through ntfs-3g in practice, which
    /// stores links, but also exFAT through exfat-fuse, whose links are
    /// then not detected. GIO does not report the FUSE subtype. Items
    /// already on the destination's file system are never asked about
    /// (`Unstorable::start_item`).
    pub(crate) fn of(kind: Option<&str>) -> Self {
        let kind = kind.map(str::to_ascii_lowercase);
        match kind.as_deref() {
            Some("msdos" | "vfat" | "fat" | "fat32") => Self {
                max_file_size: Some(FAT_MAX_FILE_SIZE),
                restricts_names: true,
                stores_links: false,
            },
            Some("exfat") => Self {
                max_file_size: None,
                restricts_names: true,
                stores_links: false,
            },
            Some("ntfs" | "ntfs3" | "fuseblk") => Self {
                restricts_names: true,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    /// The refusal of the file `name` of `size` bytes, when it is too large
    /// to be stored here.
    pub(crate) fn check_file_size(&self, name: &str, size: u64) -> Result<(), TransferError> {
        match self.max_file_size {
            Some(limit) if size > limit => Err(TransferError::failed(format!(
                "{name} is too large for the destination file system, which only supports files \
                 up to 4 GiB."
            ))),
            _ => Ok(()),
        }
    }

    /// True when `name` cannot be stored here as it is.
    pub(crate) fn forbids_name(&self, name: &OsStr) -> bool {
        self.restricts_names && name.as_bytes().iter().copied().any(is_forbidden)
    }
}

/// True for a byte FAT, exFAT and NTFS forbid in names. Every such byte is
/// ASCII, so it never splits a UTF-8 character.
fn is_forbidden(byte: u8) -> bool {
    byte < 0x20 || byte == 0x7f || FORBIDDEN_CHARACTERS.contains(&byte)
}

/// `name` with every forbidden character replaced by `_`.
pub(crate) fn replace_forbidden_characters(name: &OsStr) -> OsString {
    let bytes = name
        .as_bytes()
        .iter()
        .map(|&byte| if is_forbidden(byte) { b'_' } else { byte })
        .collect();
    OsString::from_vec(bytes)
}

/// A copy or move of some items into one folder, as the free-space check
/// sees it.
pub(crate) struct Incoming<'a> {
    /// Copy or move.
    pub(crate) mode: TransferMode,
    /// What happens to taken names.
    pub(crate) policy: ConflictPolicy,
    /// The folder the items go into.
    pub(crate) folder: &'a dyn Node,
    /// Its file system.
    pub(crate) filesystem: &'a FilesystemInfo,
}

impl Incoming<'_> {
    /// Refuses the run before anything is written when the items that will
    /// be written do not fit in the destination's free space. Items a move
    /// only renames, and items Skip will leave alone, need no space. An
    /// item that cannot be measured counts as empty: its copy reports the
    /// problem. `on_folder` is called before each folder is listed, so the
    /// caller can show that a long walk is under way. Returns what the run
    /// writes, when the destination's free space is known and so the
    /// items were measured.
    ///
    /// # Errors
    ///
    /// "Not enough free space on …", or [`TransferError::Cancelled`].
    pub(crate) fn check_free_space(
        &self,
        factory: &NodeFactory,
        uris: &[&str],
        cancel: &Cancellation,
        on_folder: &mut dyn FnMut(),
    ) -> Result<Option<u64>, TransferError> {
        let Some(free) = self.filesystem.free else {
            return Ok(None);
        };
        let mut needed = 0_u64;
        for uri in uris {
            let Ok(source) = factory(uri) else {
                continue;
            };
            if !self.writes(source.as_ref(), cancel) {
                continue;
            }
            needed = needed.saturating_add(tree_size(source.as_ref(), cancel, 0, on_folder)?);
            if needed > free {
                return Err(TransferError::failed(format!(
                    "Not enough free space on {}: {} needed, {} free.",
                    self.folder.display_name(),
                    pretty_bytes(needed),
                    pretty_bytes(free)
                )));
            }
        }
        Ok(Some(needed))
    }

    /// True when the run will write `source`'s bytes into the folder. A
    /// move counts only when both file systems are known to differ: one
    /// that turns out to need a copy still stops at a full disk, item by
    /// item, without publishing anything partial.
    fn writes(&self, source: &dyn Node, cancel: &Cancellation) -> bool {
        if self.mode == TransferMode::Move {
            let source_id = source.filesystem(Some(cancel)).and_then(|info| info.id);
            let crosses_filesystems = matches!(
                (&source_id, &self.filesystem.id),
                (Some(source_id), Some(destination_id)) if source_id != destination_id
            );
            if !crosses_filesystems {
                return false;
            }
        }
        // Checked under the source's own name. A name the destination
        // forbids is never taken there, so an item that will be renamed
        // always counts: the check is conservative.
        if self.policy == ConflictPolicy::Skip {
            let is_taken = child_node(self.folder, source.name())
                .is_ok_and(|destination| destination.exists(Some(cancel)));
            return !is_taken;
        }
        true
    }
}

/// The bytes of the files in `node`'s tree, links not followed.
/// `on_folder` is called before each folder is listed.
fn tree_size(
    node: &dyn Node,
    cancel: &Cancellation,
    depth: usize,
    on_folder: &mut dyn FnMut(),
) -> Result<u64, TransferError> {
    cancel.check()?;
    let Ok(info) = node.info(Some(cancel)) else {
        return Ok(0);
    };
    match info.kind {
        NodeKind::File => Ok(info.size),
        NodeKind::Directory if depth < MAX_DEPTH => {
            on_folder();
            let Ok(children) = node.children(Some(cancel)) else {
                return Ok(0);
            };
            let mut total = 0_u64;
            for child in children {
                total = total.saturating_add(tree_size(child.as_ref(), cancel, depth + 1, on_folder)?);
            }
            Ok(total)
        }
        _ => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: XFER-028
    #[test]
    fn windows_file_systems_restrict_names_and_fat_restricts_sizes_and_links() {
        let fat = StorageRules::of(Some("msdos"));
        let exfat = StorageRules::of(Some("exfat"));
        let ntfs = StorageRules::of(Some("fuseblk"));
        let ext4 = StorageRules::of(Some("ext4"));

        assert!(fat.forbids_name(OsStr::new("a:b.txt")) && exfat.forbids_name(OsStr::new("why?")));
        assert!(ntfs.forbids_name(OsStr::new("tab\there")));
        assert!(!fat.forbids_name(OsStr::new("Holiday (2026).jpg")));
        assert!(!ext4.forbids_name(OsStr::new("a:b.txt")));
        assert!(!fat.stores_links && !exfat.stores_links && ntfs.stores_links && ext4.stores_links);
        assert!(fat.check_file_size("big.iso", FAT_MAX_FILE_SIZE).is_ok());
        let refused = fat.check_file_size("big.iso", FAT_MAX_FILE_SIZE + 1).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "big.iso is too large for the destination file system, which only supports files up to 4 GiB."
        );
        assert!(exfat.check_file_size("big.iso", u64::MAX).is_ok());
        assert_eq!(
            replace_forbidden_characters(OsStr::new("a:b*c?.txt")),
            OsStr::new("a_b_c_.txt")
        );
    }
}
