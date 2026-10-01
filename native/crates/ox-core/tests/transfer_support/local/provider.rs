// SPDX-License-Identifier: AGPL-3.0-only
//! The overridable behaviour of the local test double: the Rust stand-in
//! for subclassing `LocalNode` in `v2.0.0:desktop/tests/local_provider.py`.

use std::path::PathBuf;
use std::sync::Arc;

use ox_core::transfer::{Cancellation, FilesystemInfo, Node, NodeInfo, TransferError};

use super::LocalNode;
use crate::transfer_support::file_uri;

/// Overridable behaviour of [`LocalNode`], like a Python subclass. Each
/// method answers the [`Node`] method of the same name for `node`.
pub trait Provider: Send + Sync + 'static {
    /// The provider this one extends; `None` means plain `LocalNode`.
    fn base(&self) -> Option<&dyn Provider> {
        None
    }

    /// [`Node::uri`]: a `file://` URI by default.
    fn uri(&self, node: &LocalNode) -> String {
        match self.base() {
            Some(base) => base.uri(node),
            None => file_uri(node.local_path()),
        }
    }

    /// [`Node::path`]: the local path by default.
    fn path(&self, node: &LocalNode) -> Option<PathBuf> {
        match self.base() {
            Some(base) => base.path(node),
            None => Some(node.local_path().to_path_buf()),
        }
    }

    /// [`Node::has_sibling_staging`]: `false` by default.
    fn has_sibling_staging(&self) -> bool {
        self.base().is_some_and(Provider::has_sibling_staging)
    }

    /// [`Node::native_copy_keeps_name`]: `false` by default.
    fn native_copy_keeps_name(&self, node: &LocalNode, target_folder: &dyn Node) -> bool {
        self.base()
            .is_some_and(|base| base.native_copy_keeps_name(node, target_folder))
    }

    /// [`Node::refresh_listing`]: nothing to refresh by default.
    fn refresh_listing(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.refresh_listing(node, cancel),
            None => Ok(()),
        }
    }

    /// [`Node::exists`]: [`LocalNode::local_exists`] by default.
    fn exists(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> bool {
        match self.base() {
            Some(base) => base.exists(node, cancel),
            None => node.local_exists(),
        }
    }

    /// [`Node::info`]: [`LocalNode::local_info`] by default.
    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        match self.base() {
            Some(base) => base.info(node, cancel),
            None => node.local_info(cancel),
        }
    }

    /// [`Node::create_directory`]: [`LocalNode::local_create_directory`] by
    /// default.
    fn create_directory(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.create_directory(node, cancel),
            None => node.local_create_directory(cancel),
        }
    }

    /// [`Node::copy_file`]: [`LocalNode::local_copy_file`] by default.
    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.copy_file(node, target, cancel, progress),
            None => node.local_copy_file(target, cancel, progress),
        }
    }

    /// [`Node::move_native`]: [`LocalNode::local_move_native`] by default.
    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.move_native(node, target, cancel),
            None => node.local_move_native(target, cancel),
        }
    }

    /// [`Node::replace_native`]: [`LocalNode::local_replace_native`] by
    /// default.
    fn replace_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.replace_native(node, target, cancel),
            None => node.local_replace_native(target, cancel),
        }
    }

    /// [`Node::delete`]: [`LocalNode::local_delete`] by default.
    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        match self.base() {
            Some(base) => base.delete(node),
            None => node.local_delete(),
        }
    }

    /// [`Node::filesystem`]: nothing reported by default.
    fn filesystem(&self, node: &LocalNode) -> Option<FilesystemInfo> {
        self.base().and_then(|base| base.filesystem(node))
    }
}

/// Plain `LocalNode` behaviour with no overrides.
pub struct Local;

impl Provider for Local {}

/// A shared plain local provider.
pub fn local() -> Arc<dyn Provider> {
    Arc::new(Local)
}
