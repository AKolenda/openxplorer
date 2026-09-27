// SPDX-License-Identifier: AGPL-3.0-only
//! The storage abstraction the transfer engine works on.
//!
//! Production uses [`crate::gio_node::GioNode`]; tests use a local-disk fake
//! with the same contract (the Rust counterpart of
//! `desktop/tests/local_provider.py`, in `tests/transfer_support/`).

use std::sync::Arc;

use gio::prelude::*;

/// A transfer failure. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    /// The user cancelled: returned by [`Cancellation::check`]. The engine
    /// reports a cancelled item as `cancelled`, not as an error.
    #[error("Operation cancelled.")]
    Cancelled,
    /// A definite "does not exist" (`G_IO_ERROR_NOT_FOUND`, `ENOENT`).
    #[error("{0}")]
    NotFound(String),
    /// The name is taken; nothing was overwritten.
    #[error("{0}")]
    Exists(String),
    /// The backend cannot do this here (for example a cross-filesystem move).
    #[error("{0}")]
    NotSupported(String),
    /// The backend cannot replace in one step; the engine then uses
    /// reversible renames instead.
    #[error("{0}")]
    ReplaceUnsupported(String),
    /// A recovery location or permission problem the user must see even
    /// when cancellation arrived during the commit.
    #[error("{0}")]
    RecoveryRequired(String),
    /// Any other backend or validation failure.
    #[error("{0}")]
    Failed(String),
}

impl TransferError {
    /// A definite "does not exist", as opposed to "could not check".
    pub fn is_not_found(&self) -> bool {
        matches!(self, TransferError::NotFound(_))
    }

    /// True for a user cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, TransferError::Cancelled)
    }

    /// A validation or safety failure with a user-facing message.
    pub fn failed(message: impl Into<String>) -> Self {
        TransferError::Failed(message.into())
    }
}

/// Maps GLib errors the way `desktop/gio_backend.py` and
/// `desktop/operations.py` interpret them: `NOT_FOUND` is a definite absence
/// (see `is_not_found` in `operations.py`), `EXISTS` a taken name and
/// `NOT_SUPPORTED` an unsupported operation. Operation-specific meanings
/// (for example `WOULD_RECURSE` on a move) are mapped by the caller before
/// falling back to this conversion.
///
/// `G_IO_ERROR_CANCELLED` deliberately stays an ordinary failure with the
/// backend's message. A device can report "Operation was cancelled" for a
/// call the user never cancelled; like the Python engine, the transfer
/// engine counts an item as cancelled only when the user's
/// [`Cancellation`] is cancelled.
impl From<glib::Error> for TransferError {
    fn from(error: glib::Error) -> Self {
        let message = error.message().to_string();
        match error.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotFound) => TransferError::NotFound(message),
            Some(gio::IOErrorEnum::Exists) => TransferError::Exists(message),
            Some(gio::IOErrorEnum::NotSupported) => TransferError::NotSupported(message),
            _ => TransferError::Failed(message),
        }
    }
}

/// Maps local I/O errors: `ENOENT` is a definite absence and `EEXIST` a
/// taken name, exactly like `FileNotFoundError`/`FileExistsError` in Python.
impl From<std::io::Error> for TransferError {
    fn from(error: std::io::Error) -> Self {
        let message = error.to_string();
        match error.kind() {
            std::io::ErrorKind::NotFound => TransferError::NotFound(message),
            std::io::ErrorKind::AlreadyExists => TransferError::Exists(message),
            _ => TransferError::Failed(message),
        }
    }
}

/// Preserves missing-file and name-conflict errors from descriptor-relative I/O.
impl From<rustix::io::Errno> for TransferError {
    fn from(error: rustix::io::Errno) -> Self {
        std::io::Error::from(error).into()
    }
}

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

/// Cooperative cancellation shared with in-flight GIO calls.
#[derive(Debug, Clone, Default)]
pub struct Cancellation {
    cancellable: gio::Cancellable,
}

impl Cancellation {
    /// A fresh, not yet cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. In-flight GIO calls using this token abort.
    pub fn cancel(&self) {
        self.cancellable.cancel();
    }

    /// True once [`Cancellation::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancellable.is_cancelled()
    }

    /// Returns [`TransferError::Cancelled`] once cancelled.
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
///
/// Every query that inspects an item (`info`, `children`) must not follow
/// symbolic links: the engine copies links as links and never traverses them.
pub trait Node: Send + Sync {
    /// The canonical URI.
    fn uri(&self) -> String;
    /// The last path component.
    fn name(&self) -> String;
    /// A local filesystem path, when the item has one.
    fn path(&self) -> Option<std::path::PathBuf>;
    /// The item `name` inside this folder. `name` must be one path component;
    /// the engine validates generated and listed names before calling this.
    fn child(&self, name: &str) -> Box<dyn Node>;
    /// The containing folder, or `None` at a root.
    fn parent(&self) -> Option<Box<dyn Node>>;
    /// `false` also when the backend cannot answer; use [`Node::info`] to
    /// tell "missing" from "unreachable".
    fn exists(&self, cancel: Option<&Cancellation>) -> bool;
    /// Kind, size and mode without following a symbolic link.
    fn info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError>;
    /// True for a folder (a link to a folder counts, like the Python app).
    fn is_directory(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError>;
    /// The folder's items, including hidden ones, without following links.
    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError>;
    /// Exclusive: fails if the name exists.
    fn mkdir(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Copies one file, or one symbolic link as a link, to the new name
    /// `target`. Never overwrites. `progress(current, total)` may be called
    /// often. The callback cannot abort the copy, so implementations must
    /// stop with an error soon after `cancel` is cancelled (GIO does this
    /// through the cancellable; the engine stops reporting progress then).
    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError>;
    /// A native move or rename with no copy/delete fallback. Never
    /// overwrites an existing target.
    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Move over an existing file after the user chose Replace. Returns
    /// [`TransferError::ReplaceUnsupported`] when the backend cannot do this
    /// in one step.
    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError>;
    /// Deletes one file or one empty folder. Only ever called on staging the
    /// engine created, or a replacement backup it owns.
    fn delete(&self) -> Result<(), TransferError>;
    /// Moves the item to the Trash. Must never fall back to a permanent
    /// delete.
    fn trash(&self, cancel: &Cancellation) -> Result<(), TransferError>;
    /// Whether this location has a usable Trash.
    fn can_trash(&self, cancel: Option<&Cancellation>) -> bool;
    /// Explicit, user-confirmed permanent delete of a whole tree. Symbolic
    /// links are removed as links; their targets are never traversed.
    fn delete_tree(
        &self,
        cancel: &Cancellation,
        assert_writable: Option<&WriteGuard>,
    ) -> Result<(), TransferError>;
    /// Stage directory copies beside the final name (MTP). Asked of the
    /// destination folder: its native move cannot rename across folders.
    /// Directory copies use a hidden sibling plus same-folder rename; file
    /// copies keep their final name inside an exclusively owned folder.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glib_errors_keep_their_meaning() {
        let cases = [
            (gio::IOErrorEnum::Cancelled, TransferError::Failed("m".into())),
            (gio::IOErrorEnum::NotFound, TransferError::NotFound("m".into())),
            (gio::IOErrorEnum::Exists, TransferError::Exists("m".into())),
            (
                gio::IOErrorEnum::NotSupported,
                TransferError::NotSupported("m".into()),
            ),
            (
                gio::IOErrorEnum::PermissionDenied,
                TransferError::Failed("m".into()),
            ),
        ];
        for (code, expected) in cases {
            let error = glib::Error::new(code, "m");
            assert_eq!(TransferError::from(error), expected);
        }
    }

    #[test]
    fn io_errors_keep_their_meaning() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(TransferError::from(missing).is_not_found());
        let taken = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert!(matches!(TransferError::from(taken), TransferError::Exists(_)));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(TransferError::from(denied), TransferError::Failed(_)));
    }

    #[test]
    fn cancellation_is_reported_by_check() {
        let cancel = Cancellation::new();
        assert_eq!(cancel.check(), Ok(()));
        cancel.cancel();
        assert!(cancel.is_cancelled());
        assert_eq!(cancel.check(), Err(TransferError::Cancelled));
    }
}
