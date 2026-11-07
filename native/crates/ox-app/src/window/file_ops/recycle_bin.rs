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
//! name taken there (`ox_core::ops::scan_restore_conflicts`), also one
//! taken while the restore runs, and deleting asks first unless the
//! settings say not to (SET-010). Every other failure is asked about in
//! the progress window, as Windows asks (OPS-047), and nothing is said
//! once it ends. Empty Recycle Bin asks Windows'
//! question: "Are you sure you want to permanently delete these N
//! items?", with Yes and No.

use ox_core::location::TRASH_URI;
use ox_core::ops::{
    delete_from_recycle_bin, empty_recycle_bin, list_recycle_bin, permanent_delete_confirmation,
    restore_answered_items, restore_free_items, restore_questions, restores_into_one_folder,
    scan_drag_out_conflicts, scan_restore_conflicts, starting_label, ConflictScan, DeleteItem,
    OperationContext, OpsError, Resolution, RestoreSoFar, TransferOutcome,
};
use ox_core::transfer::{Cancellation, TransferMode};

use super::running::FinishedOperation;
use super::FileCommand;
use crate::window::transfer_panel::{Action, Verb};
use crate::window::BrowserWindow;

/// The panel's label while items are restored.
const RESTORING: &str = crate::i18n::message_id("Restoring items…");

/// How many times a restore asks again about items whose names were
/// taken while it ran, as by an item restored before them from the same
/// place; beyond that, another program keeps taking the names, and such
/// an item is asked about as a failure.
const TAKEN_ROUNDS: usize = 4;

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
        let outcome = self.restore_asking(&uris, None, &context).await;
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
                self.restore_asking(&uris, None, &context).await
            }
            Err(error) => Err(error),
        };
        self.end_restore(outcome).await;
    }

    /// The plan of restoring `uris`, into their original folders, or into
    /// `into` for items dragged out of the Recycle Bin.
    async fn restore_scan(
        uris: &[String],
        into: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<ConflictScan, OpsError> {
        match into {
            Some(folder) => scan_drag_out_conflicts(uris, folder, cancel).await,
            None => scan_restore_conflicts(uris, cancel).await,
        }
    }

    /// Restores `uris` as the operation of `context`, as Windows moves
    /// them back into their folders, or into `into` for items dragged out
    /// of the Recycle Bin: the action reads "Moving 2 items to Desktop",
    /// the items whose names are free go first, then a taken name asks
    /// "Replace or Skip Files" in the progress window, whose answers
    /// decide the rest. A name taken while the restore runs, as by an
    /// item restored before it from the same place, is asked about the
    /// same way, as Windows asks as it meets it. Every other failure is
    /// asked about in the progress window too (OPS-047). Cancel at a
    /// question keeps what was restored, and says nothing.
    async fn restore_asking(
        &self,
        uris: &[String],
        into: Option<&str>,
        context: &OperationContext,
    ) -> Result<TransferOutcome, OpsError> {
        let mut scan = Self::restore_scan(uris, into, &context.cancel).await?;
        let destination = restores_into_one_folder(&scan).then(|| scan.destination_folder.clone());
        let action = Action {
            verb: Verb::Restoring,
            count: Some(u64::try_from(uris.len()).unwrap_or(u64::MAX)),
            source: TRASH_URI.to_owned(),
            destination,
        };
        self.describe_operation(&context.cancel, Some(action.clone()));
        let mut so_far = restore_free_items(&scan, RestoreSoFar::new(), context).await?;
        for _ in 0..TAKEN_ROUNDS {
            if so_far.cancelled() {
                return Ok(so_far.into_outcome());
            }
            let Some(answers) = self.ask_restore_conflicts(&scan, &action, context).await else {
                return Ok(so_far.into_outcome());
            };
            so_far = restore_answered_items(&scan, &answers, so_far, context).await?;
            let taken = so_far.take_taken();
            if taken.is_empty() || so_far.cancelled() {
                return Ok(so_far.finish());
            }
            scan = Self::restore_scan(&taken, into, &context.cancel).await?;
            so_far = restore_free_items(&scan, so_far, context).await?;
        }
        Ok(so_far.finish())
    }

    /// Asks "Replace or Skip Files" about the conflicts of the restore
    /// `scan`, one question per folder, each naming its folder: an answer
    /// per conflict, or `None` when the user cancelled.
    async fn ask_restore_conflicts(
        &self,
        scan: &ConflictScan,
        action: &Action,
        context: &OperationContext,
    ) -> Option<Vec<Resolution>> {
        let mut answers = vec![Resolution::Skip; scan.conflicts.len()];
        if !scan.has_conflicts() {
            return Some(answers);
        }
        let panel = self.panel_for_operation(&context.cancel);
        let several = action.destination.is_none();
        for (question, indices) in restore_questions(scan) {
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
            let answered = panel.ask_about_conflicts(&question).await?;
            for (index, answer) in indices.into_iter().zip(answered) {
                answers[index] = answer;
            }
        }
        if several {
            self.describe_operation(&context.cancel, Some(action.clone()));
        }
        Some(answers)
    }

    /// Ends a restore: Undo remembers it and the restored items are
    /// selected.
    async fn end_restore(&self, outcome: Result<TransferOutcome, OpsError>) {
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Recycle Bin items dropped into `folder`: moved there under their
    /// original names, never overwriting, a taken name asking "Replace or
    /// Skip Files" as a restore does (OPS-046).
    pub(crate) async fn move_out_of_recycle_bin(&self, uris: Vec<String>, folder: String) {
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RESTORING)) else {
            return;
        };
        let outcome = self.restore_asking(&uris, Some(&folder), &context).await;
        self.end_restore(outcome).await;
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
        // "Deleting 2 items from Recycle Bin", as Windows reads it.
        self.describe_operation(&context.cancel, Action::of(Verb::Deleting, &uris, None));
        let outcome = delete_from_recycle_bin(&uris, &context).await;
        self.end_operation();
        let finished = outcome.map(|_| FinishedOperation {
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
            Some(uris) => {
                self.describe_operation(&context.cancel, Action::of(Verb::Deleting, &uris, None));
                delete_from_recycle_bin(&uris, &context).await
            }
            None => empty_recycle_bin(&context).await,
        };
        self.end_operation();
        let finished = outcome.map(|_| FinishedOperation {
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
