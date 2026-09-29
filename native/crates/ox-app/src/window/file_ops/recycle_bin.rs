// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin's commands: Restore, Delete permanently and Empty
//! Recycle Bin (OPS-040 to OPS-043).
//!
//! New in the native app, from the Dolphin baseline; the Python app could
//! only move items to the Trash. The window lists `trash:///` like any
//! folder, with each item's original location and deletion date; these
//! commands act on it through ox-core's Recycle Bin service, which never
//! overwrites when it restores. Each runs as the window's one operation,
//! with the transfer panel and Cancel, and deleting asks first.

use ox_core::ops::{
    delete_from_recycle_bin, empty_recycle_bin, move_out_of_recycle_bin, permanent_delete_confirmation,
    restore_from_recycle_bin, summarize, summarize_restore, DeleteConfirmation,
};
use ox_core::transfer::TransferMode;

use super::running::FinishedOperation;
use super::FileCommand;
use crate::window::BrowserWindow;

/// The panel's label while items are restored.
const RESTORING: &str = "Restoring items…";

/// The panel's label while items are deleted for good.
const DELETING: &str = "Deleting items…";

/// The question Empty Recycle Bin asks.
fn empty_confirmation() -> DeleteConfirmation {
    DeleteConfirmation {
        title: "Empty Recycle Bin?",
        body: "Every item in the Recycle Bin is deleted permanently and cannot be recovered.".to_owned(),
        confirm_label: "Empty Recycle Bin",
    }
}

impl BrowserWindow {
    /// Restore: puts the selected items back where they were deleted from,
    /// recreating missing folders; a taken name keeps its item in the
    /// Recycle Bin (OPS-041).
    pub(crate) async fn restore_selected_items(&self) {
        if !self.allows(FileCommand::Restore) {
            return;
        }
        let uris = self.folder_pane().model().selected_uris();
        let Some(context) = self.begin_operation(RESTORING) else {
            return;
        };
        let outcome = restore_from_recycle_bin(&uris, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_restore(&outcome.result),
            undo: outcome.undo,
            created: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Recycle Bin items dropped into `folder`: moved there under their
    /// original names, never overwriting (OPS-046).
    pub(crate) async fn move_out_of_recycle_bin(&self, uris: Vec<String>, folder: String) {
        let Some(context) = self.begin_operation(RESTORING) else {
            return;
        };
        let outcome = move_out_of_recycle_bin(&uris, &folder, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_restore(&outcome.result),
            undo: outcome.undo,
            created: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Delete in the Recycle Bin: asks, then deletes the selected items for
    /// good (OPS-043).
    pub(super) async fn delete_from_recycle_bin(&self) {
        let items = self.items_to_delete();
        if !self
            .confirm_deletion(&permanent_delete_confirmation(&items))
            .await
        {
            return;
        }
        let uris: Vec<String> = items.into_iter().map(|item| item.uri).collect();
        let Some(context) = self.begin_operation(DELETING) else {
            return;
        };
        let outcome = delete_from_recycle_bin(&uris, &context.cancel).await;
        self.end_operation();
        let finished = outcome.map(|result| FinishedOperation {
            summary: summarize(TransferMode::Delete, &result),
            undo: None,
            created: Vec::new(),
        });
        self.conclude_operation(finished).await;
    }

    /// Empty Recycle Bin: asks, then deletes everything in it for good
    /// (OPS-042).
    pub(crate) async fn empty_recycle_bin(&self) {
        if !self.allows(FileCommand::EmptyRecycleBin) {
            return;
        }
        if !self.confirm_deletion(&empty_confirmation()).await {
            return;
        }
        let Some(context) = self.begin_operation(DELETING) else {
            return;
        };
        let outcome = empty_recycle_bin(&context.cancel).await;
        self.end_operation();
        let finished = outcome.map(|result| FinishedOperation {
            summary: summarize(TransferMode::Delete, &result),
            undo: None,
            created: Vec::new(),
        });
        self.conclude_operation(finished).await;
    }
}
