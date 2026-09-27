// SPDX-License-Identifier: AGPL-3.0-only
//! The storage abstraction the transfer engine works on.
//!
//! Production uses [`crate::gio_node::GioNode`]; tests use a local-disk fake
//! with the same contract (the Rust counterpart of
//! `desktop/tests/local_provider.py`).

use std::sync::Arc;

/// A transfer failure. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("Operation cancelled.")]
    Cancelled,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Exists(String),
    #[error("{0}")]
    NotSupported(String),
    /// The backend cannot replace in one step; the engine then uses
    /// reversible renames instead.
    #[error("{0}")]
    ReplaceUnsupported(String),
    /// Any other backend or validation failure.
    #[error("{0}")]
    Failed(String),
}

impl TransferError {
    /// A definite "does not exist", as opposed to "could not check".
    pub fn is_not_found(&self) -> bool {
        matches!(self, TransferError::NotFound(_))
    }
}

/// What an item is, queried without following symbolic links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Directory,
    File,
    Symlink,
    Special,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    pub kind: NodeKind,
    pub size: u64,
    /// Unix permission bits when the backend reports them (not on MTP).
    pub mode: Option<u32>,
}

/// Cooperative cancellation shared with in-flight GIO calls.
#[derive(Debug, Clone, Default)]
pub struct Cancellation {
    cancellable: gio::Cancellable,
}

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        use gio::prelude::CancellableExt;
        self.cancellable.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        use gio::prelude::CancellableExt;
        self.cancellable.is_cancelled()
    }

    pub fn check(&self) -> Result<(), TransferError> {
        if self.is_cancelled() {
            Err(TransferError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// The underlying cancellable for GIO calls.
    pub fn cancellable(&self) -> &gio::Cancellable {
        &self.cancellable
    }
}

/// One file or folder in some backend. Mirrors the Python `Node` protocol in
/// `desktop/operations.py`, plus the device capabilities added for MTP.
pub trait Node: Send + Sync {
    fn uri(&self) -> String;
    fn name(&self) -> String;
    /// A local filesystem path, when the item has one.
    fn path(&self) -> Option<std::path::PathBuf>;
    fn child(&self, name: &str) -> Box<dyn Node>;
    fn parent(&self) -> Option<Box<dyn Node>>;
    /// `false` also when the backend cannot answer; use [`Node::info`] to
    /// tell "missing" from "unreachable".
    fn exists(&self, cancel: Option<&Cancellation>) -> bool;
    fn info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError>;
    fn is_directory(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError>;
    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError>;
    /// Exclusive: fails if the name exists.
    fn mkdir(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Never overwrites. `progress(current, total)` may be called often.
    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError>;
    /// A native move or rename with no copy/delete fallback.
    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Move over an existing file after the user chose Replace.
    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Deletes one file or one empty folder. Only ever called on staging the
    /// engine created, or a replacement backup it owns.
    fn delete(&self) -> Result<(), TransferError>;
    fn trash(&self, cancel: &Cancellation) -> Result<(), TransferError>;
    fn can_trash(&self, cancel: Option<&Cancellation>) -> bool;
    /// Explicit, user-confirmed permanent delete of a whole tree.
    fn delete_tree(
        &self,
        cancel: &Cancellation,
        assert_writable: Option<&WriteGuard>,
    ) -> Result<(), TransferError>;
    /// Stage copies beside the final name (MTP).
    fn stage_as_sibling(&self) -> bool {
        false
    }
    /// A native copy into `target_dir` keeps this item's own name (MTP
    /// CopyObject within one device).
    fn native_copy_keeps_name(&self, _target_dir: &dyn Node) -> bool {
        false
    }
    /// Re-list this folder so a backend's stale path cache is rebuilt (MTP).
    fn refresh_listing(&self, _cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        Ok(())
    }
}

/// Rejects writes into protected locations such as snapshot folders.
pub type WriteGuard = dyn Fn(&str) -> Result<(), TransferError> + Send + Sync;

/// Resolves a URI to a node.
pub type NodeFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Node>, TransferError> + Send + Sync>;
