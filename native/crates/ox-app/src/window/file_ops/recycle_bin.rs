// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin's commands: Restore, Restore all items, Delete
//! permanently and Empty Recycle Bin (OPS-040 to OPS-043).
//!
//! New in the native app, from the Dolphin baseline; the Python app could
//! only move items to the Trash. The window lists `trash:///` like any
//! folder, with each item's original location and deletion date; these
//! commands act on it through ox-core's Recycle Bin service, which never
//! overwrites unless the user chose Replace. Each runs as the window's one operation,
//! with the transfer panel and Cancel. Restore moves each item back into
//! its folder as Windows does, asking "Replace or Skip Files" about a
//! name taken there (`ox_core::ops::scan_restore_conflicts`), and deleting asks first unless the
//! settings say not to (SET-010). Empty Recycle Bin asks Windows'
//! question: "Are you sure you want to permanently delete these N
//! items?", with Yes and No.

use ox_core::location::TRASH_URI;
use ox_core::ops::{
    delete_from_recycle_bin, empty_recycle_bin, list_recycle_bin, move_out_of_recycle_bin,
    permanent_delete_confirmation, restore_answered_items, restore_free_items, restore_questions,
    restores_into_one_folder, scan_restore_conflicts, starting_label, summarize, summarize_restore,
    DeleteItem, OperationContext, OpsError, Resolution, TransferOutcome,
};
use ox_core::transfer::{Cancellation, TransferMode};

use super::running::FinishedOperation;
use super::FileCommand;
use crate::window::transfer_panel::{Action, Verb};
use crate::window::BrowserWindow;

/// The panel's label while items are restored.
const RESTORING: &str = crate::i18n::message_id("Restoring items…");

impl BrowserWindow {
    /// Restore: puts the selected items back where they were deleted from,
    /// recreating missing folders (OPS-041); a taken name asks Windows'
    /// "Replace or Skip Files" question (OPS-026).
    pub(crate) async fn restore_selected_items(&self) {
        if !self.allows(FileCommand::Restore) {
            return;
        }
        let uris = self.folder_pane().model().selected_uris();
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RESTORING)) else {
            return;
        };
        let outcome = self.restore_asking(&uris, &context).await;
        self.end_restore(outcome).await;
    }

    /// Restore all items: puts every item in the Recycle Bin back where it
    /// was deleted from, as Restore does (OPS-041).
    pub(crate) async fn restore_all_items(&self) {
        if !self.allows(FileCommand::RestoreAll) {
            return;
        }
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RESTORING)) else {
            return;
        };
        let outcome = match list_recycle_bin(&context.cancel).await {
            Ok(items) => {
                let uris: Vec<String> = items.into_iter().map(|item| item.uri).collect();
                self.restore_asking(&uris, &context).await
            }
            Err(error) => Err(error),
        };
        self.end_restore(outcome).await;
    }

    /// Restores `uris` as the operation of `context`, as Windows moves
    /// them back into their folders: the action reads "Moving 2 items to
    /// Desktop", the items whose names are free go first, then a taken
    /// name asks "Replace or Skip Files" in the progress window, whose
    /// answers decide the rest. Cancel at the question keeps what was
    /// restored, and says nothing.
    async fn restore_asking(
        &self,
        uris: &[String],
        context: &OperationContext,
    ) -> Result<TransferOutcome, OpsError> {
        let scan = scan_restore_conflicts(uris, &context.cancel).await?;
        let destination = restores_into_one_folder(&scan).then(|| scan.destination_folder.clone());
        let action = Action {
            verb: Verb::Restoring,
            count: Some(u64::try_from(uris.len()).unwrap_or(u64::MAX)),
            source: TRASH_URI.to_owned(),
            destination,
        };
        self.describe_operation(&context.cancel, Some(action.clone()));
        let so_far = restore_free_items(&scan, context).await?;
        if so_far.cancelled() {
            return Ok(so_far.into_outcome());
        }
        if !scan.has_conflicts() {
            return restore_answered_items(&scan, &[], so_far, context).await;
        }
        let panel = self.panel_for_operation(&context.cancel);
        // One question per original folder, each naming its folder.
        let mut answers = vec![Resolution::Skip; scan.conflicts.len()];
        let several = action.destination.is_none();
        for (question, indices) in restore_questions(&scan) {
            if several {
                let going = scan
                    .items
                    .iter()
                    .filter(|item| item.destination_folder == question.destination_folder)
                    .count();
                // "Moving 2 items to Documents": the folder asked about.
                let asked = Action {
                    count: Some(u64::try_from(going).unwrap_or(u64::MAX)),
                    destination: Some(question.destination_folder.clone()),
                    ..action.clone()
                };
                self.describe_operation(&context.cancel, Some(asked));
            }
            let Some(answered) = panel.ask_about_conflicts(&question).await else {
                return Ok(so_far.into_outcome());
            };
            for (index, answer) in indices.into_iter().zip(answered) {
                answers[index] = answer;
            }
        }
        if several {
            self.describe_operation(&context.cancel, Some(action));
        }
        restore_answered_items(&scan, &answers, so_far, context).await
    }

    /// Ends a restore: Undo remembers it, the restored items are selected
    /// and a failure is reported.
    async fn end_restore(&self, outcome: Result<TransferOutcome, OpsError>) {
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_restore(&outcome.result),
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Recycle Bin items dropped into `folder`: moved there under their
    /// original names, never overwriting (OPS-046).
    pub(crate) async fn move_out_of_recycle_bin(&self, uris: Vec<String>, folder: String) {
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RESTORING)) else {
            return;
        };
        let outcome = move_out_of_recycle_bin(&uris, &folder, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_restore(&outcome.result),
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Delete in the Recycle Bin: asks, then deletes the selected items for
    /// good (OPS-043).
    pub(super) async fn delete_from_recycle_bin(&self) {
        let items = self.items_to_delete();
        if !self.confirms_permanent_delete(&items).await {
            return;
        }
        let uris: Vec<String> = items.into_iter().map(|item| item.uri).collect();
        let Some(context) = self.begin_operation(starting_label(TransferMode::Delete)) else {
            return;
        };
        let outcome = delete_from_recycle_bin(&uris, &context.cancel).await;
        self.end_operation();
        let finished = outcome.map(|result| FinishedOperation {
            summary: summarize(TransferMode::Delete, &result),
            undo: None,
            select_after: Vec::new(),
        });
        self.conclude_operation(finished).await;
    }

    /// Empty Recycle Bin: asks, then deletes everything in it for good
    /// (OPS-042).
    pub(crate) async fn empty_recycle_bin(&self) {
        if !self.allows(FileCommand::EmptyRecycleBin) {
            return;
        }
        self.empty_trash().await;
    }

    /// Empty Recycle Bin from the sidebar's Recycle Bin (SIDE-025), from
    /// any folder: asks, then deletes for good the items it asked about,
    /// or, when the settings say not to ask, everything in it.
    pub(crate) async fn empty_trash(&self) {
        let asks = self.context().settings_data().preferences.confirm_empty_trash;
        let confirmed = if asks {
            match self.confirm_emptying().await {
                Ok(Some(uris)) => Some(uris),
                Ok(None) => return,
                Err(error) => {
                    self.conclude_operation(Err(error)).await;
                    return;
                }
            }
        } else {
            None
        };
        let Some(context) = self.begin_operation(starting_label(TransferMode::Delete)) else {
            return;
        };
        let outcome = match confirmed {
            Some(uris) => delete_from_recycle_bin(&uris, &context.cancel).await,
            None => empty_recycle_bin(&context.cancel).await,
        };
        self.end_operation();
        let finished = outcome.map(|result| FinishedOperation {
            summary: summarize(TransferMode::Delete, &result),
            undo: None,
            select_after: Vec::new(),
        });
        self.conclude_operation(finished).await;
    }

    /// Asks about the items in the Recycle Bin as Windows Explorer does
    /// when it empties it: "Delete Multiple Items", "Are you sure you want
    /// to permanently delete these N items?" (one item is asked about as a
    /// file or a folder, with its details), Yes and No. The URIs of the
    /// items asked about for Yes, so that an item deleted while the
    /// question shows is kept; `None` for No, and when there is nothing to
    /// empty.
    ///
    /// # Errors
    ///
    /// The Recycle Bin could not be listed, so nothing was asked.
    async fn confirm_emptying(&self) -> Result<Option<Vec<String>>, OpsError> {
        let items = list_recycle_bin(&Cancellation::new()).await?;
        if items.is_empty() {
            return Ok(None);
        }
        let items: Vec<DeleteItem> = items
            .into_iter()
            .map(|item| DeleteItem {
                uri: item.uri,
                name: item.name,
                can_trash: None,
            })
            .collect();
        let confirmed = self.confirm_deletion(&items, permanent_delete_confirmation).await;
        Ok(confirmed.then(|| items.into_iter().map(|item| item.uri).collect()))
    }
}
