// SPDX-License-Identifier: AGPL-3.0-only
//! Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z, Ctrl+Y) (OPS-029, OPS-031).
//!
//! New in the native app; the Python app had neither. The journal belongs
//! to the application ([`crate::app_context::AppContext`]), as Dolphin's
//! undo manager does, so Undo in any window reverses the newest operation
//! of any window. A step is taken from the journal before its reversal
//! runs, so two windows can never take the same one; a reversal refused
//! before it changed anything goes back. Each reversal runs as the
//! window's one operation, with the transfer panel and Cancel, and reuses
//! the safety rules of the operation that reverses it (see
//! [`ox_core::ops::undo`]). The commands are labelled with the operation
//! they reverse ("Undo: Rename"). Before Undo moves copies to the Trash
//! that were changed after the copy, it asks, as Dolphin does (OPS-030).

use ox_core::ops::{
    changed_copies, reverse, summarize_journal_step, JournalDirection, JournalEntry, STOPPED_TITLE,
};

use super::FileCommand;
use crate::window::background_notice::Destination;
use crate::window::dialog::{self, Dialog};
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The title of the question before Undo moves changed copies to the
/// Trash (Dolphin's "Undo File Copy Confirmation").
const UNDO_COPY_TITLE: &str = "Undo copy?";

/// The command that walks the journal in `direction`.
const fn command_for(direction: JournalDirection) -> FileCommand {
    match direction {
        JournalDirection::Undo => FileCommand::Undo,
        JournalDirection::Redo => FileCommand::Redo,
    }
}

impl BrowserWindow {
    /// Undo or Redo: reverses the newest step in `direction`.
    pub(crate) async fn walk_journal(&self, direction: JournalDirection) {
        if !self.allows(command_for(direction)) {
            return;
        }
        let Some(step) = self.context().take_journal_step(direction) else {
            return;
        };
        if direction == JournalDirection::Undo && !self.confirm_undoing_changed_copies(&step).await {
            self.context().put_back_journal_step(direction, step);
            return;
        }
        let label = format!("{}…", step.label(direction));
        let Some(context) = self.begin_operation(&label) else {
            self.context().put_back_journal_step(direction, step);
            return;
        };
        let progress = self.progress_reporter(&context.cancel);
        let outcome = reverse(&step.record, &context, progress).await;
        self.end_operation();
        match outcome {
            Ok(reversal) => {
                self.context()
                    .record_reversal(direction, step.title, reversal.inverse);
                let destination = Destination::items(reversal.result.done.clone());
                self.reload_selecting(reversal.result.done.clone());
                let summary = summarize_journal_step(direction, step.title, &reversal.result);
                self.report(summary, destination).await;
            }
            Err(error) => {
                self.context().put_back_journal_step(direction, step);
                self.reload_selecting(Vec::new());
                dialog::show_message(self, STOPPED_TITLE, &error.to_string()).await;
            }
        }
    }

    /// True when Undo may move the copies of `step` to the Trash: none
    /// was modified after the copy, or the user confirmed (OPS-030).
    async fn confirm_undoing_changed_copies(&self, step: &JournalEntry) -> bool {
        let changed = changed_copies(&step.record, step.recorded_at).await;
        let Some(first) = changed.first() else {
            return true;
        };
        let message = if changed.len() == 1 {
            format!("“{first}” was changed after it was copied. Undo moves it to the Recycle Bin anyway?")
        } else {
            format!(
                "{} copies, such as “{first}”, were changed after they were copied. Undo moves them to \
                 the Recycle Bin anyway?",
                changed.len()
            )
        };
        let dialog = Dialog::new(self, UNDO_COPY_TITLE, &message);
        dialog.add_cancel_button();
        dialog.add_button("Undo copy", ButtonStyle::Accent);
        dialog.open();
        let confirmed = dialog.next_response().await.is_some();
        if confirmed {
            dialog.finish();
        }
        confirmed
    }

    /// The label of the Undo or Redo command now, such as "Undo: Rename",
    /// or its plain name while it has nothing to do.
    pub(crate) fn journal_label(&self, direction: JournalDirection) -> String {
        self.context()
            .journal_label(direction)
            .unwrap_or_else(|| direction.command_name().to_owned())
    }
}
