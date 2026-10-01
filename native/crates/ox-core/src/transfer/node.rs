// SPDX-License-Identifier: AGPL-3.0-only
//! The storage abstraction the transfer engine works on.
//!
//! Ports the `Node` protocol and `Info` of `v2.0.0:desktop/operations.py`.
//! Production uses [`crate::gio_node::GioNode`]; tests use a local-disk fake
//! with the same contract (the Rust counterpart of
//! `v2.0.0:desktop/tests/local_provider.py`, in `tests/transfer_support/`).

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::limits::FilesystemInfo;
use super::staging::clean_staging;

/// What an item is, queried without following symbolic links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A folder.
    Directory,
    /// A regular file.
    File,
    /// A symbolic link, copied as a link and never traversed.
    Symlink,
    /// Sockets, devices, FIFOs and anything else the engine never copies.
    Special,
}

/// The metadata the engine needs about one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    /// What the item is, without following a symbolic link.
    pub kind: NodeKind,
    /// Size in bytes as the backend reports it.
    pub size: u64,
    /// Unix permission bits when the backend reports them (not on MTP).
    pub mode: Option<u32>,
    /// The last modification time, when the backend reports it.
    pub modified: Option<SystemTime>,
    /// Which local object the item is, when the backend reports it.
    pub identity: Option<ItemIdentity>,
}

impl NodeInfo {
    /// True when `current`, queried later, still describes the item this
    /// info describes: the same object and kind, and for anything but a
    /// folder the same size and modification time. A value either query
    /// lacks is not compared. A folder's own size and time change with its
    /// entries, which are checked one by one instead (XFER-013).
    pub(crate) fn still_describes(&self, current: &NodeInfo) -> bool {
        if self.kind != current.kind || !agree(self.identity, current.identity) {
            return false;
        }
        self.kind == NodeKind::Directory
            || (self.size == current.size && agree(self.modified, current.modified))
    }
}

/// True when two optional values are equal or either is unknown.
fn agree<T: PartialEq>(earlier: Option<T>, later: Option<T>) -> bool {
    match (earlier, later) {
        (Some(earlier), Some(later)) => earlier == later,
        _ => true,
    }
}

/// Which local filesystem object an item is (`st_dev`, `st_ino`). The
/// engine records it for a staging folder right after creating it, so
/// cleanup can refuse a different folder that was moved in under the same
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemIdentity {
    /// The device holding the item.
    pub device: u64,
    /// The item's inode on that device.
    pub inode: u64,
}

/// One file or folder in some backend. Mirrors the Python `Node` protocol in
/// `v2.0.0:desktop/operations.py`, plus the device capabilities added for MTP.
///
/// Every query that inspects an item (`info`, `children`) must not follow
/// symbolic links: the engine copies links as links and never traverses them.
/// Every method that returns a [`TransferError`] reports the backend's own
/// error; the variants the engine acts on are named in each method.
///
/// Cancellation takes three forms, by what a method is used for:
///
/// - `Option<&Cancellation>` for the short steps the engine also takes
///   after the user cancelled: checking, publishing, replacing and
///   relisting. Cleanup and the recovery of a replacement pass `None`,
///   because stopping halfway would leave staging or a moved-aside
///   original behind; the user's steps pass `Some`.
/// - `&Cancellation` for the long operations the user starts and may stop
///   at any time: copying a file, moving to the Trash and deleting a tree.
/// - No token for [`Node::delete`], which removes only the engine's own
///   staging or backups, or one source item a move has already copied:
///   that cleanup must finish even after the user
///   cancelled (`_clean_staging` and `_discard_stage` in
///   `v2.0.0:desktop/operations.py` take no cancellation either).
pub trait Node: Send + Sync {
    /// The canonical URI.
    fn uri(&self) -> String;

    /// The last path component, byte for byte. Linux file names need not be
    /// UTF-8, and a copy must publish exactly the name it read.
    fn name(&self) -> OsString;

    /// The name for labels and messages, with invalid UTF-8 replaced. Never
    /// use it to address an item.
    fn display_name(&self) -> String {
        self.name().to_string_lossy().into_owned()
    }

    /// A local filesystem path, when the item has one.
    fn path(&self) -> Option<PathBuf>;

    /// The item `name` inside this folder. `name` must be one path component;
    /// the engine validates generated and listed names before calling this.
    fn child(&self, name: &OsStr) -> Box<dyn Node>;

    /// The containing folder, or `None` at a root.
    fn parent(&self) -> Option<Box<dyn Node>>;

    /// True when the item exists. Also `false` when the backend cannot
    /// answer; use [`Node::info`] to tell "missing" from "unreachable".
    /// A dangling symbolic link exists: its name is taken.
    fn exists(&self, cancel: Option<&Cancellation>) -> bool;

    /// Kind, size and mode without following a symbolic link.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotFound`] only when the item definitely does not
    /// exist; any other error means its state is unknown.
    fn info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError>;

    /// True for a folder (a link to a folder counts, like the Python app).
    ///
    /// # Errors
    ///
    /// The backend's error when the item cannot be inspected.
    fn is_directory(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError>;

    /// The folder's items, including hidden ones, without following links.
    ///
    /// # Errors
    ///
    /// The backend's error when the folder cannot be listed completely.
    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError>;

    /// Creates this folder. Exclusive: it never reuses an existing item.
    ///
    /// # Errors
    ///
    /// [`TransferError::Exists`] when the name is taken; the engine then has
    /// no right to clean up that name.
    fn create_directory(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError>;

    /// Copies one file, or one symbolic link as a link, to the new name
    /// `target`. `progress(current, total)` may be called often. The
    /// callback cannot abort the copy, so implementations must stop with an
    /// error soon after `cancel` is cancelled (GIO does this through the
    /// cancellable; the engine stops reporting progress then).
    ///
    /// # Errors
    ///
    /// [`TransferError::Exists`] when `target` exists: the copy never
    /// overwrites.
    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError>;

    /// A native move or rename with no copy/delete fallback.
    ///
    /// # Errors
    ///
    /// [`TransferError::Exists`] when `target` exists: the move never
    /// overwrites. [`TransferError::NotSupported`] when the backend cannot
    /// move natively, for example across filesystems.
    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;

    /// Installs this completed staged copy under `target` without ever
    /// overwriting: the step that makes a copy visible under its final name.
    ///
    /// Staged copies are the engine's own items, so a backend may use a
    /// stricter rename than [`Node::move_native`], which also has to carry
    /// the user's item along with its desktop metadata. The default is
    /// [`Node::move_native`].
    ///
    /// # Errors
    ///
    /// [`TransferError::Exists`] when `target` exists, including when
    /// another program created it a moment before; nothing is overwritten.
    fn publish(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.move_native(target, cancel)
    }

    /// Move over an existing file after the user chose Replace.
    ///
    /// # Errors
    ///
    /// [`TransferError::ReplaceUnsupported`] when the backend cannot do this
    /// in one step; the engine then replaces through reversible renames.
    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;

    /// Deletes one file, one link or one empty folder. Only ever called on
    /// staging the engine created, a replacement backup it owns, or a
    /// source item a move has just copied (XFER-013).
    ///
    /// # Errors
    ///
    /// The backend's error, for example for a folder that is not empty.
    fn delete(&self) -> Result<(), TransferError>;

    /// Moves the item to the Trash.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotSupported`] where there is no Trash. It never
    /// falls back to a permanent delete.
    fn trash(&self, cancel: &Cancellation) -> Result<(), TransferError>;

    /// Whether this location has a usable Trash, which decides whether the
    /// app offers "Move to Trash" or an explicit permanent delete.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotMounted`] when the share or device must be
    /// mounted first, so the caller can mount it and ask again. Every other
    /// failure answers `Ok(false)`, like `can_trash` in
    /// `v2.0.0:desktop/gio_backend.py`.
    fn can_trash(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError>;

    /// Explicit, user-confirmed permanent delete of a whole tree. Symbolic
    /// links are removed as links; their targets are never traversed.
    /// `guard` is asked about every item before it is removed.
    ///
    /// # Errors
    ///
    /// The first failure, the guard's refusal or [`TransferError::Cancelled`];
    /// items not yet reached are left in place.
    fn delete_tree(&self, cancel: &Cancellation, guard: Option<&WriteGuard>) -> Result<(), TransferError>;

    /// Removes this staging tree, which the engine created. `created` is
    /// the identity recorded right after the engine made it, when the
    /// backend has one.
    ///
    /// The default walks the tree by path (`clean_staging` in the engine's
    /// `staging` module). Local backends override it to walk relative to
    /// pinned folders and to refuse a folder other than `created`, so a
    /// folder moved in under the staging name before or during cleanup is
    /// never emptied.
    ///
    /// # Errors
    ///
    /// The first item that cannot be removed, or a staging name that now
    /// leads to another item; the caller reports the leftover.
    fn delete_staging(&self, _created: Option<ItemIdentity>) -> Result<(), TransferError> {
        clean_staging(self)
    }

    /// True when copies into this folder are staged beside their final name
    /// (MTP). Asked of the destination folder: its native move cannot
    /// rename across folders, and some devices cannot move across folders
    /// at all. Files and folders are then built under a hidden sibling name
    /// and published by a same-folder rename.
    fn has_sibling_staging(&self) -> bool {
        false
    }

    /// True when a native copy into `target_folder` keeps this item's own
    /// name whatever target name is asked for (MTP `CopyObject` within one
    /// device).
    fn native_copy_keeps_name(&self, _target_folder: &dyn Node) -> bool {
        false
    }

    /// Re-lists this folder so a backend's stale path cache is rebuilt
    /// (MTP). Other backends have no such cache and do nothing.
    ///
    /// # Errors
    ///
    /// The backend's error when the folder cannot be listed.
    fn refresh_listing(&self, _cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        Ok(())
    }

    /// The file system this item is on (XFER-028): its type, free space
    /// and id, as far as the backend reports them; `None` when it reports
    /// nothing.
    fn filesystem(&self, _cancel: Option<&Cancellation>) -> Option<FilesystemInfo> {
        None
    }
}

/// Rejects writes into protected locations such as snapshot folders.
pub type WriteGuard = dyn Fn(&str) -> Result<(), TransferError> + Send + Sync;

/// Resolves a URI to a node.
pub type NodeFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Node>, TransferError> + Send + Sync>;
