// SPDX-License-Identifier: AGPL-3.0-only
//! Closing a window while it writes files (TAB-049).
//!
//! The Python app refused every close while a file operation ran (the
//! "A file operation is running" dialog and a toast for window-manager
//! closes), so the user had to find Cancel and try again. Here every
//! close, from the caption button, Alt+F4, the dock or Quit, asks
//! instead: "Keep open" leaves everything running, and "Cancel and close"
//! cancels the operation, as its Cancel button does, and closes the
//! window once it has stopped and its result has been read. Data safety
//! is unchanged: the window never goes away while a write runs, and what
//! was finished stays finished (OPS-022).

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::dialog::{ButtonStyle, Dialog};
use super::BrowserWindow;

/// The question's title (`A file operation is running` in app.js).
const RUNNING_TITLE: &str = "A file operation is running";

/// The question.
const RUNNING_QUESTION: &str =
    "Cancel it and close this window once it has stopped? Items already finished stay where they are.";

/// The answer that keeps the window and the operation.
const KEEP_OPEN: &str = "Keep open";

/// The answer that cancels the operation, then closes the window.
const CANCEL_AND_CLOSE: &str = "Cancel and close";

/// How often a window that should close looks whether it may.
const CLOSE_POLL: Duration = Duration::from_millis(100);

/// Where a window is in closing while it writes files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ClosingState {
    /// Nobody asked to close it.
    #[default]
    Open,
    /// The question is open.
    Asking,
    /// The user chose to cancel and close; it closes once idle.
    ClosingWhenIdle,
}

impl BrowserWindow {
    /// Whether a close may go ahead now; otherwise asks whether to cancel
    /// the running write and close afterwards, and the close waits.
    pub(super) fn may_close_now(&self) -> bool {
        if !self.is_writing_files() {
            return true;
        }
        if self.imp().closing.get() == ClosingState::Open {
            self.imp().closing.set(ClosingState::Asking);
            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move { window.ask_to_cancel_and_close().await }
            ));
        }
        false
    }

    /// Asks whether to cancel the running write and close the window.
    async fn ask_to_cancel_and_close(&self) {
        let dialog = Dialog::new(self, RUNNING_TITLE, RUNNING_QUESTION);
        // First, so it has the focus and Enter never cancels by accident.
        dialog.add_button(KEEP_OPEN, ButtonStyle::Standard);
        let close = dialog.add_button(CANCEL_AND_CLOSE, ButtonStyle::Danger);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer != Some(close) {
            self.imp().closing.set(ClosingState::Open);
            return;
        }
        self.imp().closing.set(ClosingState::ClosingWhenIdle);
        glib::timeout_add_local(
            CLOSE_POLL,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || window.close_once_idle()
            ),
        );
        self.close_once_idle();
    }

    /// One look of a window that should close: cancels whatever write
    /// runs, and closes once none does and no dialog, such as the
    /// operation's result, is open over it.
    fn close_once_idle(&self) -> glib::ControlFlow {
        if self.is_writing_files() {
            self.cancel_every_write();
            return glib::ControlFlow::Continue;
        }
        if self.has_open_dialog() {
            return glib::ControlFlow::Continue;
        }
        self.imp().closing.set(ClosingState::Open);
        self.close();
        glib::ControlFlow::Break
    }

    /// Cancels the running file operation and archive operation.
    fn cancel_every_write(&self) {
        self.cancel_operation();
        self.operation_panel().cancel();
    }

    /// Whether a dialog shows over this window.
    pub(super) fn has_open_dialog(&self) -> bool {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|window| window.downcast::<gtk::Window>().ok())
            .any(|window| {
                window.is_visible()
                    && window.transient_for().as_ref() == Some(self.upcast_ref::<gtk::Window>())
            })
    }
}
