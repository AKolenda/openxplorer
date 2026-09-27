// SPDX-License-Identifier: AGPL-3.0-only
//! Unix permissions for real local staging.
//!
//! Ports `_secure_local_staging`, `_set_local_directory_mode` and
//! `_restore_directory_modes` in `desktop/operations.py`.
//!
//! Rules enforced here:
//! - Only `file:` items with a real path get Unix modes. MTP, AFC and many
//!   SMB backends expose a FUSE path but do not implement `chmod`; their
//!   random staging names stay private to the connected session instead.
//! - Modes are changed through a directory opened with `O_NOFOLLOW`, so a
//!   path swapped for a symbolic link cannot redirect the change.
//! - While a copy is being built, every staged folder is owner-only
//!   (`0700`); the source's exact modes are restored children-first, just
//!   before the completed subtree is published.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, Permissions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::error::TransferError;
use super::node::{Cancellation, ItemIdentity, Node};

/// Owner-only access for staging folders.
pub(crate) const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

/// Opens a directory for `fchmod` without following a symbolic link.
pub(crate) fn open_directory_nofollow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// Sets `mode` on an already opened directory (`fchmod`).
pub(crate) fn set_mode(directory: &File, mode: u32) -> std::io::Result<()> {
    directory.set_permissions(Permissions::from_mode(mode))
}

/// The path to apply Unix modes to: only real local items qualify.
pub(crate) fn local_directory_path(node: &(impl Node + ?Sized)) -> Option<PathBuf> {
    if node.uri().starts_with("file:") {
        node.path()
    } else {
        None
    }
}

/// Makes an engine-created local staging folder owner-only (`0700`) and
/// returns the identity of the folder that got the mode. Does nothing for
/// `GVfs` backends (MTP, AFC, SMB), even when they expose a FUSE path, and
/// returns `None` for them. The Python ZIP extractor
/// (`desktop/zip_extraction.py`) secures its staging folder the same way;
/// its port will use this too.
///
/// # Errors
///
/// When the folder cannot be opened without following links, or its mode
/// cannot be changed.
pub fn secure_local_staging(node: &(impl Node + ?Sized)) -> Result<Option<ItemIdentity>, TransferError> {
    let Some(path) = local_directory_path(node) else {
        return Ok(None);
    };
    let directory = apply_mode(&path, PRIVATE_DIRECTORY_MODE)?;
    let metadata = directory.metadata()?;
    Ok(Some(ItemIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

/// Sets `mode` on the folder at `path` and returns the folder, still open.
fn apply_mode(path: &Path, mode: u32) -> Result<File, TransferError> {
    let directory = open_directory_nofollow(path)?;
    set_mode(&directory, mode)?;
    Ok(directory)
}

/// A staged local folder's final mode, applied just before publishing.
#[derive(Debug)]
pub(crate) struct PendingMode {
    /// The staged folder's local path.
    pub(crate) path: PathBuf,
    /// The exact mode to restore (the source's permission bits).
    pub(crate) mode: u32,
}

/// Final modes for staged local folders, keyed by the staged folder's URI.
#[derive(Debug, Default)]
pub(crate) struct DirectoryModes {
    pending: HashMap<String, PendingMode>,
}

impl DirectoryModes {
    /// Remembers the mode `uri` (at `path`) must get when published.
    pub(crate) fn record(&mut self, uri: String, path: PathBuf, mode: u32) {
        self.pending.insert(uri, PendingMode { path, mode });
    }

    /// Removes and returns the pending mode for `uri`.
    pub(crate) fn take(&mut self, uri: &str) -> Option<PendingMode> {
        self.pending.remove(uri)
    }

    fn contains(&self, uri: &str) -> bool {
        self.pending.contains_key(uri)
    }

    /// True when no staged folder waits for its final mode.
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Restores the recorded modes of `source` and its staged descendants,
/// children before parents, so a restrictive parent mode cannot block
/// restoring its children. Called only right before a complete subtree is
/// published: restrictive source modes must not prevent building, merging
/// or cleaning a staging folder the engine exclusively owns.
pub(crate) fn restore_directory_modes(
    source: &dyn Node,
    modes: &mut DirectoryModes,
    cancel: &Cancellation,
) -> Result<(), TransferError> {
    cancel.check()?;
    for child in source.children(Some(cancel))? {
        if modes.contains(&child.uri()) {
            restore_directory_modes(child.as_ref(), modes, cancel)?;
        }
    }
    if let Some(pending) = modes.take(&source.uri()) {
        apply_mode(&pending.path, pending.mode)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(path: &Path) -> u32 {
        let metadata = std::fs::symlink_metadata(path).expect("the test path exists");
        metadata.permissions().mode() & 0o7777
    }

    #[test]
    fn modes_are_applied_through_a_no_follow_descriptor() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let folder = temp.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");
        apply_mode(&folder, 0o750).expect("chmod a real folder");
        assert_eq!(mode_of(&folder), 0o750);

        // A symbolic link swapped in for the folder is refused, and the
        // folder it points to keeps its mode.
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&folder, &link).expect("create link");
        assert!(apply_mode(&link, 0o700).is_err());
        assert_eq!(mode_of(&folder), 0o750);
    }

    #[test]
    fn pending_modes_are_taken_once() {
        let mut modes = DirectoryModes::default();
        assert!(modes.is_empty());
        modes.record("file:///a".into(), PathBuf::from("/a"), 0o755);
        assert!(!modes.is_empty());
        let pending = modes.take("file:///a").expect("recorded");
        assert_eq!((pending.path, pending.mode), (PathBuf::from("/a"), 0o755));
        assert!(modes.take("file:///a").is_none());
    }
}
