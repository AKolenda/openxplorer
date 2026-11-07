// SPDX-License-Identifier: AGPL-3.0-only
//! Duplicate: a copy of each selected item next to itself.
//!
//! New in the native app, from the Dolphin baseline ("Duplicate Here").
//! ox-core copies each item into its own folder with the Keep both
//! policy, so the copy is staged privately and never overwrites, and is
//! named as Keep both names copies (`report - Copy.pdf`). It runs as
//! the window's one operation with the transfer panel; the duplicates are
//! selected afterwards and Undo moves them to the Trash.

use ox_core::ops::{duplicate_items, starting_label};
use ox_core::transfer::TransferMode;

use super::running::FinishedOperation;
use super::FileCommand;
use crate::window::BrowserWindow;

impl BrowserWindow {
    /// Duplicates the selected items.
    pub(crate) async fn duplicate_selection(&self) {
        if !self.allows(FileCommand::Duplicate) {
            return;
        }
        let uris = self.folder_pane().model().selected_uris();
        self.duplicate_uris(&uris).await;
    }

    /// Copies each of `uris` beside itself, as `report - Copy.pdf`: a
    /// Duplicate, or a copy dropped back in the items' own folder, as
    /// Windows Explorer makes then (DND-027).
    pub(crate) async fn duplicate_uris(&self, uris: &[String]) {
        let Some(context) = self.begin_operation(starting_label(TransferMode::Copy)) else {
            return;
        };
        let progress = self.progress_reporter(&context.cancel);
        let folder = self.current_uri();
        let verb = crate::window::transfer_panel::Verb::Copying;
        let action = crate::window::transfer_panel::Action::of(verb, uris, folder.as_deref());
        self.describe_operation(&context.cancel, action);
        let outcome = duplicate_items(uris, &context, progress).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }
}
