// SPDX-License-Identifier: AGPL-3.0-only
//! The question about an item the destination cannot store (XFER-028):
//! a name with characters FAT, exFAT and NTFS forbid, or a symbolic link
//! on FAT or exFAT.
//!
//! Beyond the Python app, which showed the raw GIO error. Dolphin asks
//! "Replace invalid characters", "Replace all", "Skip", "Skip all" or
//! "Cancel"; this dialog offers the same answers the way the name-conflict
//! dialog does, with a check box that makes the answer apply to every such
//! item of the operation. The engine asks from its worker thread and waits
//! (see `worker_question`).

use gtk::prelude::*;
use ox_core::ops::UnstorableAsker;
use ox_core::transfer::{UnstorableAnswer, UnstorableItem, UnstorableReason};

use super::worker_question::worker_question;
use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The dialog's title and message for `item`.
fn question_text(item: &UnstorableItem) -> (&'static str, String) {
    let name = &item.name;
    match item.reason {
        UnstorableReason::InvalidCharacters => (
            "Name not supported",
            ox_core::i18n::format_message("“{name}” has characters the destination file system does not allow (\" * : < > ? \\ | and control characters).", &[("name", &(name).to_string())]),
        ),
        UnstorableReason::SymbolicLink => (
            "Link not supported",
            ox_core::i18n::format_message("“{name}” is a symbolic link, which the destination file system cannot store.", &[("name", &(name).to_string())]),
        ),
    }
}

/// The answer of the button pressed, when the check box to apply it to
/// every such item is `for_all`.
fn answer(replace: bool, for_all: bool) -> UnstorableAnswer {
    match (replace, for_all) {
        (true, true) => UnstorableAnswer::ReplaceAll,
        (true, false) => UnstorableAnswer::Replace,
        (false, true) => UnstorableAnswer::SkipAll,
        (false, false) => UnstorableAnswer::Skip,
    }
}

impl BrowserWindow {
    /// An asker for the worker of one operation: each question opens the
    /// dialog over this window and waits for its answer. A question the
    /// window can no longer show is answered with Cancel.
    pub(in crate::window) fn unstorable_asker(&self) -> UnstorableAsker {
        UnstorableAsker::new(worker_question(
            self,
            UnstorableAnswer::Cancel,
            |window, item: UnstorableItem| async move { window.ask_about_unstorable(&item).await },
        ))
    }

    /// Asks about `item`; Cancel, Escape and closing the dialog cancel.
    async fn ask_about_unstorable(&self, item: &UnstorableItem) -> UnstorableAnswer {
        let (title, message) = question_text(item);
        let dialog = Dialog::new(self, title, &message);
        let for_all = dialog.add_check_button("Do this for all such items", false);
        dialog.add_cancel_button();
        let skip = dialog.add_button("Skip", ButtonStyle::Bordered);
        let replace = (item.reason == UnstorableReason::InvalidCharacters)
            .then(|| dialog.add_button("Replace invalid characters", ButtonStyle::Accent));
        dialog.open();
        let pressed = dialog.next_response().await;
        let for_all = for_all.is_active();
        dialog.finish();
        match pressed {
            Some(button) if button == skip => answer(false, for_all),
            Some(button) if Some(button) == replace => answer(true, for_all),
            _ => UnstorableAnswer::Cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: XFER-028
    #[test]
    fn each_button_and_the_check_box_give_dolphins_answers() {
        assert_eq!(answer(true, false), UnstorableAnswer::Replace);
        assert_eq!(answer(true, true), UnstorableAnswer::ReplaceAll);
        assert_eq!(answer(false, false), UnstorableAnswer::Skip);
        assert_eq!(answer(false, true), UnstorableAnswer::SkipAll);
        let link = UnstorableItem {
            name: "latest".into(),
            reason: UnstorableReason::SymbolicLink,
        };
        let (title, message) = question_text(&link);
        assert_eq!(title, "Link not supported");
        assert!(message.starts_with("“latest” is a symbolic link"), "{message}");
    }
}
