// SPDX-License-Identifier: AGPL-3.0-only
//! Explicit Trash and permanent deletion, kept as separate operations.
//!
//! Ports `GioNode.trash` and `GioNode.delete_tree` in
//! `desktop/gio_backend.py`.

use gio::prelude::*;

use super::{local_delete, remote_delete, GioNode};
use crate::transfer::{Cancellation, TransferError, WriteGuard};

impl GioNode {
    /// Moves the item to the Trash. XFER-014: never falls back to a
    /// permanent delete.
    ///
    /// # Errors
    ///
    /// [`TransferError::Cancelled`] when `cancel` was cancelled, a refusal
    /// for a filesystem root or share, [`TransferError::NotSupported`] with
    /// the Python app's explanation where there is no Trash, and the
    /// backend's error otherwise.
    pub(super) fn trash_item(&self, cancel: &Cancellation) -> Result<(), TransferError> {
        cancel.check()?;
        self.require_item()?;
        self.file.trash(Some(cancel.cancellable())).map_err(trash_error)
    }

    /// XFER-015: permanently deletes the item and everything inside it.
    /// Local items are deleted relative to pinned folder descriptors; every
    /// other location is deleted by path, as the Python app does.
    ///
    /// # Errors
    ///
    /// A refusal for a filesystem root or share, a refusal from `guard`,
    /// [`TransferError::Cancelled`], and the first error that stopped the
    /// deletion; what was deleted before it stays deleted.
    pub(super) fn delete_item_tree(
        &self,
        cancel: &Cancellation,
        guard: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        self.require_item()?;
        match self.local_path() {
            Some(path) => local_delete::delete_tree(&path, cancel, guard),
            None => remote_delete::delete_tree(&self.file, cancel, guard),
        }
    }
}

/// Maps a failed move to the Trash. A location without a Trash gets the
/// Python app's explanation, which says the item was kept.
fn trash_error(error: glib::Error) -> TransferError {
    if !error.matches(gio::IOErrorEnum::NotSupported) {
        return error.into();
    }
    TransferError::NotSupported(
        "Trash is not supported at this location. The original item was not \
         permanently deleted. Delete it permanently instead, or use the server’s \
         recycle-bin policy."
            .into(),
    )
}
