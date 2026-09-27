// SPDX-License-Identifier: AGPL-3.0-only
//! Relisting the folders that moves took items from. Ports the
//! `moved_from` handling of `TransferEngine.run` in `desktop/operations.py`.
//!
//! XFER-025: `GVfs` MTP keeps resolving a moved item's old path to the
//! moved object until that folder is listed again, so a later delete of
//! the old path would delete the moved file. Every source folder is
//! relisted once at the end of a run.

use super::node::Node;

/// The folders moves took items from, each once, in first-use order.
#[derive(Default)]
pub(crate) struct SourceFolders {
    folders: Vec<Box<dyn Node>>,
}

impl SourceFolders {
    /// Remembers `source`'s folder. Called before the move, so a failed or
    /// partial device move is relisted too.
    pub(crate) fn remember(&mut self, source: &dyn Node) {
        let Some(parent) = source.parent() else {
            return;
        };
        let uri = parent.uri();
        if !self.folders.iter().any(|folder| folder.uri() == uri) {
            self.folders.push(parent);
        }
    }

    /// Relists every remembered folder. Best effort: an unmounted device
    /// drops its cache anyway, so a failure is ignored.
    pub(crate) fn refresh_all(&self) {
        for folder in &self.folders {
            let _ = folder.refresh_listing(None);
        }
    }
}
