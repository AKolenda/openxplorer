// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the `transfer` and `gio_node` test binaries: an engine
//! over the production GIO adapter and Unix permission bits. `gio_node.rs`
//! includes this file by path, so it holds only what both binaries use.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{Node, TransferEngine};

/// An engine resolving every URI with the production [`GioNode`], without
/// a write guard.
pub fn gio_engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri: &str| {
        Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>)
    }))
}

/// The permission bits of `path`, without following a link.
pub fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("stat").permissions().mode() & 0o7777
}

/// Sets the permission bits of `path`.
pub fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}
