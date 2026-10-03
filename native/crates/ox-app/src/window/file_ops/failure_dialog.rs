// SPDX-License-Identifier: AGPL-3.0-only
//! The question about an item that failed for a reason other than a name
//! conflict (OPS-047): an unreadable source, a denied permission, a
//! device that is gone.
//!
//! Beyond the Python app, which recorded the error and went on. As
//! Dolphin's and Nautilus's error dialogs do, it names the item and the
//! error and offers Retry, Skip, "Skip all" (with several items) and
//! Cancel; what failed is still listed in the operation's result
//! (OPS-023). The engine asks from its worker thread and waits (see
//! `worker_question`); a question the window cannot show is answered
//! with Skip, the rule without a question.

use ox_core::ops::FailureAsker;
use ox_core::transfer::{FailedItem, FailureAnswer, TransferMode};

use super::worker_question::worker_question;
use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The dialog's title for `item`.
fn title(item: &FailedItem) -> String {
    let doing = match item.mode {
        TransferMode::Copy => "copying",
        TransferMode::Move => "moving",
        TransferMode::Trash => "moving to the Recycle Bin",
        TransferMode::Delete => "deleting",
    };
    ox_core::i18n::format_message(
        "Error while {doing} “{name}”",
        &[("doing", doing), ("name", &item.name)],
    )
}

impl BrowserWindow {
    /// An asker for the worker of one operation: each question opens the
    /// dialog over this window and waits for its answer.
    pub(in crate::window) fn failure_asker(&self) -> FailureAsker {
        FailureAsker::new(worker_question(
            self,
            FailureAnswer::Skip,
            |window, item: FailedItem| async move { window.ask_about_failure(&item).await },
        ))
    }

    /// Asks about `item`; Cancel, Escape and closing the dialog cancel the
    /// operation.
    async fn ask_about_failure(&self, item: &FailedItem) -> FailureAnswer {
        let dialog = Dialog::new(self, &title(item), &item.error);
        dialog.add_cancel_button();
        let skip_all = item
            .more_items
            .then(|| dialog.add_button(ox_core::i18n::gettext_static("Skip all"), ButtonStyle::Bordered));
        let skip = dialog.add_button(ox_core::i18n::gettext_static("Skip"), ButtonStyle::Bordered);
        let retry = dialog.add_button(ox_core::i18n::gettext_static("Retry"), ButtonStyle::Accent);
        dialog.open();
        let pressed = dialog.next_response().await;
        dialog.finish();
        match pressed {
            Some(button) if button == retry => FailureAnswer::Retry,
            Some(button) if button == skip => FailureAnswer::Skip,
            Some(button) if Some(button) == skip_all => FailureAnswer::SkipAll,
            _ => FailureAnswer::Cancel,
        }
    }
}
