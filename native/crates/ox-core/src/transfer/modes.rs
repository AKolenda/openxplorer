// SPDX-License-Identifier: AGPL-3.0-only
//! Unix permissions for real local staging.
//!
//! Ports `_secure_local_staging`, `_set_local_directory_mode` and
//! `_restore_directory_modes` in `v2.0.0:desktop/operations.py`.
//!
//! Rules enforced here:
//! - XFER-004: only `file:` items with a real path get Unix modes. MTP, AFC
//!   and many SMB backends expose a FUSE path but do not implement `chmod`;
//!   their random staging names stay private to the connected session
//!   instead. Modes are changed through a directory opened with
//!   `O_NOFOLLOW`, so a path swapped for a symbolic link cannot redirect
//!   the change.
//! - XFER-005: while a copy is being built, every staged folder is
//!   owner-only (`0700`); the source's exact modes are restored
//!   children-first, just before the completed subtree is published.
//!   A file system that keeps no Unix modes (a FAT or exFAT USB stick, an
//!   NTFS drive, many FUSE file systems) refuses the change on a folder of
//!   the user's own; the copy goes on there without it, as Windows copies
//!   to such a stick ([`keep_without_modes`]).
//! - OPS-047: an item written as administrator under `admin://` has no
//!   path the app may open, so its staged folders get their modes from
//!   `gvfsd-admin`, by path: [`crate::admin_copy`] let the copy in only
//!   where nobody but root can change the path. The staging folder and
//!   every folder built in it are owner-only too, and a published folder
//!   gets the source's mode without set-user-ID, set-group-ID, sticky,
//!   group write or other write, as a file copied there does.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, Permissions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use gio::prelude::*;
use rustix::fs::OFlags;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::node::{ItemIdentity, Node};
use crate::private_storage::KernelOpenFlags;

/// Owner-only access for staging folders.
pub(crate) const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

/// The permission bits an item written as administrator may keep: never
/// set-user-ID, set-group-ID or sticky on an item root now owns, and
/// nothing another account could change in a folder only root may
/// (OPS-047).
pub(crate) const ADMIN_MODE_MASK: u32 = 0o755;

/// Whether `node` is written as administrator, under `admin://`.
pub(crate) fn is_admin(node: &(impl Node + ?Sized)) -> bool {
    node.uri().starts_with("admin:")
}

/// OPS-047: gives the `admin://` item `uri` the permission bits `mode`
/// through `gvfsd-admin`, without following a link. It sets them by path,
/// which only root can change once [`crate::admin_copy`] let the item in.
///
/// # Errors
///
/// The backend's refusal.
pub(crate) fn set_admin_mode(uri: &str, mode: u32) -> Result<(), TransferError> {
    gio::File::for_uri(uri).set_attribute_uint32(
        "unix::mode",
        mode & ADMIN_MODE_MASK,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        None::<&gio::Cancellable>,
    )?;
    Ok(())
}

/// Opens a directory for `fchmod` without following a symbolic link.
///
/// # Errors
///
/// When `path` is not a folder, is a symbolic link, or cannot be opened.
pub(crate) fn open_directory_nofollow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::DIRECTORY | OFlags::NOFOLLOW)
        .open(path)
}

/// Sets `mode` on an already opened directory (`fchmod`). On a file
/// system that keeps no Unix modes, a folder of the user's own keeps the
/// mode it has ([`keep_without_modes`]).
///
/// # Errors
///
/// The kernel's refusal, other than that of a file system without Unix
/// modes on a folder of the user's own.
pub(crate) fn set_mode(directory: &File, mode: u32) -> std::io::Result<()> {
    keep_without_modes(directory.set_permissions(Permissions::from_mode(mode)), || {
        directory.metadata().map(|metadata| metadata.uid())
    })
}

/// The result of changing the mode of a folder whose owner `owner` reads,
/// where a refusal by a file system that keeps no Unix modes counts as
/// done. FAT (`EPERM` when the mode does not match its mount's masks),
/// exFAT and NTFS through FUSE, and other FUSE file systems (`ENOSYS`,
/// `ENOTSUP`) cannot store a mode at all, so stopping a copy there would
/// only keep the user from using a USB stick. Only a folder the user owns
/// qualifies: a folder of another account that refuses stops the copy.
///
/// # Errors
///
/// `changed`'s error when it is another error, or the folder is not the
/// user's own or its owner cannot be read.
pub(crate) fn keep_without_modes(
    changed: std::io::Result<()>,
    owner: impl FnOnce() -> std::io::Result<u32>,
) -> std::io::Result<()> {
    let Err(error) = changed else {
        return Ok(());
    };
    let without_modes = [
        rustix::io::Errno::PERM,
        rustix::io::Errno::NOSYS,
        rustix::io::Errno::NOTSUP,
    ]
    .iter()
    .any(|errno| error.raw_os_error() == Some(errno.raw_os_error()));
    let ours = || owner().is_ok_and(|uid| uid == rustix::process::geteuid().as_raw());
    if without_modes && ours() {
        Ok(())
    } else {
        Err(error)
    }
}

/// The path to apply Unix modes to. XFER-004: only real local items
/// qualify, never a `GVfs` FUSE path of a remote item.
pub(crate) fn path_for_unix_modes(node: &(impl Node + ?Sized)) -> Option<PathBuf> {
    if node.uri().starts_with("file:") {
        node.path()
    } else {
        None
    }
}

/// Makes an engine-created local staging folder owner-only (`0700`) and
/// returns the identity of the folder that got the mode. Does nothing for
/// `GVfs` backends (MTP, AFC, SMB), even when they expose a FUSE path, and
/// returns `None` for them. A folder under `admin://` is made owner-only
/// through `gvfsd-admin` before anything is written into it (OPS-047),
/// and has no identity the app can read. The ZIP extractor in [`crate::archive`]
/// secures its staging folder with this too, as
/// `v2.0.0:desktop/zip_extraction.py` calls `_secure_local_staging`.
///
/// # Errors
///
/// When the folder cannot be opened without following links, or its mode
/// cannot be changed.
pub(crate) fn secure_local_staging(
    node: &(impl Node + ?Sized),
) -> Result<Option<ItemIdentity>, TransferError> {
    let Some(path) = path_for_unix_modes(node) else {
        if is_admin(node) {
            set_admin_mode(&node.uri(), PRIVATE_DIRECTORY_MODE)?;
        }
        return Ok(None);
    };
    let directory = apply_mode(&path, PRIVATE_DIRECTORY_MODE)?;
    let metadata = directory.metadata()?;
    Ok(Some(ItemIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

/// Gives a local staging folder this engine made private `mode` just
/// before it is published, through a descriptor opened without following
/// links. When `created` is known, the folder must still be the one made
/// private, so a folder moved in under its name is never changed. Does
/// nothing for `GVfs` backends, as [`secure_local_staging`].
///
/// # Errors
///
/// When the folder cannot be opened that way, is another folder now, or
/// its mode cannot be changed.
pub(crate) fn open_local_staging(
    node: &(impl Node + ?Sized),
    created: Option<ItemIdentity>,
    mode: u32,
) -> Result<(), TransferError> {
    let Some(path) = path_for_unix_modes(node) else {
        return Ok(());
    };
    let directory = open_directory_nofollow(&path)?;
    if let Some(created) = created {
        let metadata = directory.metadata()?;
        if (metadata.dev(), metadata.ino()) != (created.device, created.inode) {
            return Err(TransferError::failed(format!(
                "{} was replaced while it was being extracted.",
                path.display()
            )));
        }
    }
    set_mode(&directory, mode)?;
    Ok(())
}

/// Sets `mode` on the folder at `path` and returns the folder, still open.
fn apply_mode(path: &Path, mode: u32) -> Result<File, TransferError> {
    let directory = open_directory_nofollow(path)?;
    set_mode(&directory, mode)?;
    Ok(directory)
}

/// Where a staged folder's final mode is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModeTarget {
    /// A local folder, at this path, through a descriptor.
    Local(PathBuf),
    /// A folder written as administrator, at this `admin://` URI, through
    /// `gvfsd-admin` (OPS-047).
    Admin(String),
}

impl ModeTarget {
    /// Gives the folder `mode`.
    fn apply(&self, mode: u32) -> Result<(), TransferError> {
        match self {
            ModeTarget::Local(path) => apply_mode(path, mode).map(drop),
            ModeTarget::Admin(uri) => set_admin_mode(uri, mode),
        }
    }
}

/// A staged folder's final mode, applied just before publishing.
#[derive(Debug)]
pub(crate) struct PendingMode {
    /// Where the staged folder is.
    pub(crate) target: ModeTarget,
    /// The exact mode to restore (the source's permission bits).
    pub(crate) mode: u32,
}

/// Final modes for staged local folders, keyed by the staged folder's URI.
#[derive(Debug, Default)]
pub(crate) struct DirectoryModes {
    pending: HashMap<String, PendingMode>,
}

impl DirectoryModes {
    /// Remembers the mode `uri` (at `target`) must get when published.
    pub(crate) fn record(&mut self, uri: String, target: ModeTarget, mode: u32) {
        self.pending.insert(uri, PendingMode { target, mode });
    }

    /// Removes and returns the pending mode for `uri`.
    pub(crate) fn take(&mut self, uri: &str) -> Option<PendingMode> {
        self.pending.remove(uri)
    }

    /// Forgets the modes of the staged folder `uri` and the folders inside
    /// it, which were removed.
    pub(crate) fn forget_below(&mut self, uri: &str) {
        let inside = format!("{}/", uri.trim_end_matches('/'));
        self.pending
            .retain(|pending, _| pending != uri && !pending.starts_with(&inside));
    }

    /// True when `uri` waits for its final mode.
    fn contains(&self, uri: &str) -> bool {
        self.pending.contains_key(uri)
    }

    /// True when no staged folder waits for its final mode.
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// XFER-005: restores the recorded modes of `source` and its staged
/// descendants, children before parents, so a restrictive parent mode
/// cannot block restoring its children. Called only right before a complete
/// subtree is published: restrictive source modes must not prevent
/// building, merging or cleaning a staging folder the engine exclusively
/// owns.
///
/// # Errors
///
/// The first folder that cannot be listed or given its mode, or
/// [`TransferError::Cancelled`].
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
        pending.target.apply(pending.mode)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{permission_bits, temporary_folder};

    /// parity: XFER-004
    #[test]
    fn modes_are_applied_through_a_no_follow_descriptor() {
        let root = temporary_folder();
        let folder = root.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");
        apply_mode(&folder, 0o750).expect("chmod a real folder");
        assert_eq!(permission_bits(&folder), 0o750);

        // A symbolic link swapped in for the folder is refused, and the
        // folder it points to keeps its mode.
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&folder, &link).expect("create link");
        assert!(apply_mode(&link, 0o700).is_err());
        assert_eq!(permission_bits(&folder), 0o750);
    }

    /// A folder of the user's own on a file system that keeps no Unix
    /// modes (FAT and exFAT USB sticks, NTFS drives, many FUSE file
    /// systems) refuses `chmod` with EPERM, ENOSYS or ENOTSUP. That is no
    /// reason to stop a copy there, as Windows copies to such a stick; a
    /// folder of another account, or any other error, still stops it.
    /// parity: XFER-004
    #[test]
    fn a_folder_of_ours_on_a_file_system_without_unix_modes_keeps_the_copy_going() {
        let ours = rustix::process::geteuid().as_raw();
        let refused = |errno: i32| std::io::Error::from_raw_os_error(errno);
        for errno in [
            rustix::io::Errno::PERM,
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::NOTSUP,
        ] {
            assert!(keep_without_modes(Err(refused(errno.raw_os_error())), || Ok(ours)).is_ok());
            assert!(keep_without_modes(Err(refused(errno.raw_os_error())), || Ok(ours + 1)).is_err());
        }
        let io_error = rustix::io::Errno::IO.raw_os_error();
        assert!(keep_without_modes(Err(refused(io_error)), || Ok(ours)).is_err());
        assert!(keep_without_modes(Ok(()), || Ok(ours + 1)).is_ok());
        let access = rustix::io::Errno::ACCESS.raw_os_error();
        assert!(
            keep_without_modes(Err(refused(rustix::io::Errno::PERM.raw_os_error())), || Err(
                refused(access)
            ))
            .is_err()
        );
    }

    #[test]
    fn pending_modes_are_taken_once() {
        let mut modes = DirectoryModes::default();
        assert!(modes.is_empty());
        modes.record("file:///a".into(), ModeTarget::Local(PathBuf::from("/a")), 0o755);
        assert!(!modes.is_empty());
        let pending = modes.take("file:///a").expect("recorded");
        assert_eq!(
            (pending.target, pending.mode),
            (ModeTarget::Local(PathBuf::from("/a")), 0o755)
        );
        assert!(modes.take("file:///a").is_none());
    }
}
