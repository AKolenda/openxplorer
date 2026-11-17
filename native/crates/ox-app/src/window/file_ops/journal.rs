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
//! they reverse ("Undo Rename"). Before Undo moves copies to the Trash
//! that were changed after the copy, it asks, as Dolphin does (OPS-030).
//! A Replace that overwrote files, as Windows 11 does by default, can't
//! bring them back: Undo says so plainly first (OPS-028).
//! Items that cannot go to the Recycle Bin, as on a share, are deleted
//! permanently only after Delete's own question there (OPS-016).

use ox_core::ops::{
    changed_copies_except, plan_delete, removed_by_undo, reverse_deleting, JournalDirection, JournalEntry,
    STOPPED_TITLE,
};
use ox_core::transfer::Cancellation;

use super::FileCommand;
use crate::dialog::{self, Dialog};
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The title of the question before Undo moves changed copies to the
/// Trash (Dolphin's "Undo File Copy Confirmation").
const UNDO_COPY_TITLE: &str = crate::i18n::message_id("Undo copy?");

/// What the user said when Undo named the files a Replace overwrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverwrittenAnswer {
    /// The step overwrote nothing: nothing was said.
    Nothing,
    /// The step does nothing more, and the user was told with OK.
    Told,
    /// The user chose to undo the rest of the step.
    UndoTheRest,
    /// The user cancelled: the step stays to undo.
    Cancelled,
}

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
        let mut delete_permanently = Vec::new();
        if direction == JournalDirection::Undo {
            match self.tell_what_undo_cannot_bring_back(&step).await {
                OverwrittenAnswer::Nothing | OverwrittenAnswer::UndoTheRest => {}
                OverwrittenAnswer::Cancelled => {
                    self.context().put_back_journal_step(direction, step);
                    return;
                }
                // Nothing else to undo: the step is used up, so the next
                // Undo reverses the operation before it.
                OverwrittenAnswer::Told => return,
            }
            let Some(without_recycle_bin) = self.confirm_undo_without_recycle_bin(&step).await else {
                self.context().put_back_journal_step(direction, step);
                return;
            };
            // The permanent delete was just confirmed with the item's
            // details; the Recycle Bin question is for the others.
            if !self
                .confirm_undoing_changed_copies(&step, &without_recycle_bin)
                .await
            {
                self.context().put_back_journal_step(direction, step);
                return;
            }
            delete_permanently = without_recycle_bin;
        }
        let label = format!("{}…", step.label(direction));
        let Some(context) = self.begin_operation(&label) else {
            self.context().put_back_journal_step(direction, step);
            return;
        };
        let progress = self.progress_reporter(&context.cancel);
        let outcome = reverse_deleting(&step.record, &delete_permanently, &context, progress).await;
        self.end_operation();
        match outcome {
            Ok(reversal) => {
                self.context()
                    .record_reversal(direction, step.operation, reversal.inverse);
                // Each failure was asked about while the reversal ran:
                // nothing more is said, as in Windows Explorer.
                self.reload_selecting(reversal.result.done.clone());
            }
            Err(error) => {
                self.context().put_back_journal_step(direction, step);
                self.reload_selecting(Vec::new());
                if error.is_cancelled() {
                    return;
                }
                dialog::show_message(
                    self,
                    ox_core::i18n::gettext_static(STOPPED_TITLE),
                    &error.to_string(),
                )
                .await;
            }
        }
    }

    /// Says that Undo of `step` can't bring back the files its Replace
    /// overwrote, as Windows 11 overwrites them: with OK alone when there
    /// is nothing else to undo, else asking whether to undo the rest.
    async fn tell_what_undo_cannot_bring_back(&self, step: &JournalEntry) -> OverwrittenAnswer {
        let overwritten = step.record.overwritten();
        let Some(first) = overwritten.first() else {
            return OverwrittenAnswer::Nothing;
        };
        let said = if overwritten.len() == 1 {
            let name = gtk::prelude::FileExt::basename(&gtk::gio::File::for_uri(first))
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            ox_core::i18n::format_message(
                "“{name}” was replaced, so Undo can't bring back the old file.",
                &[("name", &name)],
            )
        } else {
            ox_core::i18n::format_message(
                "{count} files were replaced, so Undo can't bring back the old files.",
                &[("count", &overwritten.len().to_string())],
            )
        };
        let title = step.label(JournalDirection::Undo);
        // A Windows message box, as the app's other file questions are:
        // the caption names the step, the icon stands beside the words.
        if !step.record.undoes_more_than_overwrites() {
            let dialog = Dialog::message_box(self, &title, &said);
            dialog.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
            dialog.open();
            dialog.next_response().await;
            dialog.finish();
            return OverwrittenAnswer::Told;
        }
        let message = format!("{said} {}", ox_core::i18n::gettext("Undo the rest anyway?"));
        let dialog = Dialog::message_box(self, &title, &message);
        let undo = dialog.add_button(ox_core::i18n::gettext_static("Undo"), ButtonStyle::Accent);
        dialog.add_cancel_button();
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer == Some(undo) {
            OverwrittenAnswer::UndoTheRest
        } else {
            OverwrittenAnswer::Cancelled
        }
    }

    /// The items Undo of `step` removes that cannot go to the Recycle
    /// Bin, as on a share, once the user agreed to delete them
    /// permanently with Delete's own question there ("Are you sure you
    /// want to permanently delete this file?"); empty when every item can
    /// go to the Recycle Bin. `None` when the user answered No.
    async fn confirm_undo_without_recycle_bin(&self, step: &JournalEntry) -> Option<Vec<String>> {
        let removed = removed_by_undo(&step.record);
        if removed.is_empty() {
            return Some(Vec::new());
        }
        // Only a cancellation fails the plan, and nothing cancels it here.
        let plan = plan_delete(&super::delete::named_items(removed), &Cancellation::new())
            .await
            .ok()?;
        if plan.to_delete.is_empty() {
            return Some(Vec::new());
        }
        let items = super::delete::named_items(plan.to_delete.clone());
        self.confirms_permanent_delete(&items)
            .await
            .then_some(plan.to_delete)
    }

    /// True when Undo may move the copies of `step` to the Trash: none
    /// was modified after the copy, or the user confirmed (OPS-030).
    async fn confirm_undoing_changed_copies(&self, step: &JournalEntry, deleted: &[String]) -> bool {
        let changed = changed_copies_except(&step.record, step.recorded_at, deleted).await;
        let Some(first) = changed.first() else {
            return true;
        };
        let message = if changed.len() == 1 {
            ox_core::i18n::format_message(
                "“{first}” may have changed after it was copied. Undo moves it to the Recycle Bin anyway?",
                &[("first", first)],
            )
        } else {
            ox_core::i18n::format_message("{len} copies, such as “{first}”, may have changed after they were copied. Undo moves them to the Recycle Bin anyway?", &[("len", &changed.len().to_string()), ("first", first)])
        };
        let dialog = Dialog::new(self, ox_core::i18n::gettext_static(UNDO_COPY_TITLE), &message);
        dialog.add_cancel_button();
        dialog.add_button(ox_core::i18n::gettext_static("Undo copy"), ButtonStyle::Accent);
        dialog.open();
        let confirmed = dialog.next_response().await.is_some();
        if confirmed {
            dialog.finish();
        }
        confirmed
    }

    /// The label of the Undo or Redo command now, such as "Undo Rename",
    /// or its plain name while it has nothing to do.
    pub(crate) fn journal_label(&self, direction: JournalDirection) -> String {
        self.context()
            .journal_label(direction)
            .unwrap_or_else(|| direction.command_name().to_owned())
    }
}
