// SPDX-License-Identifier: AGPL-3.0-only
//! The storage abstraction the transfer engine works on.
//!
//! Ports the `Node` protocol and `Info` of `desktop/operations.py`.
//! Production uses [`crate::gio_node::GioNode`]; tests use a local-disk fake
//! with the same contract (the Rust counterpart of
//! `desktop/tests/local_provider.py`, in `tests/transfer_support/`).

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use super::cancellation::Cancellation;
use super::error::TransferError;
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
/// `desktop/operations.py`, plus the device capabilities added for MTP.
///
/// Every query that inspects an item (`info`, `children`) must not follow
/// symbolic links: the engine copies links as links and never traverses them.
/// Every method that returns a [`TransferError`] reports the backend's own
/// error; the variants the engine acts on are named in each method.
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
    fn mkdir(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError>;

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

    /// Deletes one file or one empty folder. Only ever called on staging the
    /// engine created, or a replacement backup it owns.
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
    /// `desktop/gio_backend.py`.
    fn can_trash(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError>;

    /// Explicit, user-confirmed permanent delete of a whole tree. Symbolic
    /// links are removed as links; their targets are never traversed.
    /// `assert_writable` is asked about every item before it is removed.
    ///
    /// # Errors
    ///
    /// The first failure, the guard's refusal or [`TransferError::Cancelled`];
    /// items not yet reached are left in place.
    fn delete_tree(
        &self,
        cancel: &Cancellation,
        assert_writable: Option<&WriteGuard>,
    ) -> Result<(), TransferError>;

    /// Removes this staging tree, which the engine created. `created` is
    /// the identity recorded right after the engine made it, when the
    /// backend has one.
    ///
    /// The default walks the tree by path ([`clean_staging`]). Local
    /// backends override it to walk relative to pinned folders and to refuse
    /// a folder other than `created`, so a folder moved in under the staging
    /// name before or during cleanup is never emptied.
    ///
    /// # Errors
    ///
    /// The first item that cannot be removed, or a staging name that now
    /// leads to another item; the caller reports the leftover.
    ///
    /// [`clean_staging`]: super::clean_staging
    fn delete_staging(&self, _created: Option<ItemIdentity>) -> Result<(), TransferError> {
        clean_staging(self)
    }

    /// True when copies into this folder are staged beside their final name
    /// (MTP). Asked of the destination folder: its native move cannot
    /// rename across folders, and some devices cannot move across folders
    /// at all. Files and folders are then built under a hidden sibling name
    /// and published by a same-folder rename.
    fn stage_as_sibling(&self) -> bool {
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
}

/// Rejects writes into protected locations such as snapshot folders.
pub type WriteGuard = dyn Fn(&str) -> Result<(), TransferError> + Send + Sync;

/// Resolves a URI to a node.
pub type NodeFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Node>, TransferError> + Send + Sync>;
