// SPDX-License-Identifier: AGPL-3.0-only
//! The question before a move the location cannot do natively, to another
//! drive, share or device, is finished by copying (XFER-011, XFER-013).
//!
//! The Python app refused such moves and kept the source. Dolphin and
//! Windows copy and then delete without asking; this dialog keeps the old
//! refusal as the safe answer and asks once per operation. Cancel, Escape
//! and closing the dialog keep the originals. The engine asks from its
//! worker thread and waits (see `worker_question`).

use ox_core::ops::MoveByCopyingAsker;
use ox_core::transfer::MoveByCopyingItem;

use super::worker_question::worker_question;
use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The dialog's title.
const TITLE: &str = "Move by copying?";

/// The label of the button that agrees.
const COPY_AND_REMOVE: &str = "Copy, then remove originals";

/// The dialog's message for `item`.
fn question_text(item: &MoveByCopyingItem) -> String {
    format!(
        "“{}” cannot be moved directly to “{}”, which is on another drive, share or device. \
         The items can be copied there and the originals removed once their copies are complete. \
         An original that changes during the move is kept.",
        item.name, item.destination
    )
}

impl BrowserWindow {
    /// An asker for the worker of one operation: the question opens the
    /// dialog over this window and waits for its answer. A question the
    /// window can no longer show keeps the originals.
    pub(in crate::window) fn move_by_copying_asker(&self) -> MoveByCopyingAsker {
        MoveByCopyingAsker::new(worker_question(
            self,
            false,
            |window, item: MoveByCopyingItem| async move { window.ask_about_move_by_copying(&item).await },
        ))
    }

    /// Asks about `item`; only the agreeing button agrees.
    async fn ask_about_move_by_copying(&self, item: &MoveByCopyingItem) -> bool {
        let dialog = Dialog::new(self, TITLE, &question_text(item));
        dialog.add_cancel_button();
        let copy = dialog.add_button(COPY_AND_REMOVE, ButtonStyle::Accent);
        dialog.open();
        let pressed = dialog.next_response().await;
        dialog.finish();
        pressed == Some(copy)
    }
}
