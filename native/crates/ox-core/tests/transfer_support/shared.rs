// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the `transfer` and `gio_node` test binaries: an engine
//! over the production GIO adapter, Unix permission bits and the guard that
//! gives read-only test folders back to their owner. `gio_node.rs`
//! includes this file by path, so it holds only what both binaries use.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

/// Gives a folder and every folder below it owner access again when
/// dropped, so the temporary folder can be removed even after a test that
/// made folders read-only failed midway.
pub struct RestoreOwnerAccess(PathBuf);

impl RestoreOwnerAccess {
    /// Restores owner access to `folder` and below when dropped.
    pub fn new(folder: &Path) -> Self {
        Self(folder.to_path_buf())
    }
}

impl Drop for RestoreOwnerAccess {
    fn drop(&mut self) {
        restore_owner_access(&self.0);
    }
}

/// Makes `folder`, if it is a folder, and every folder below it owner-only
/// and writable.
fn restore_owner_access(folder: &Path) {
    let is_folder = fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir());
    if !is_folder {
        return;
    }
    set_mode(folder, 0o700);
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        restore_owner_access(&entry.path());
    }
}
