// SPDX-License-Identifier: AGPL-3.0-only
//! When a window may close (TAB-049, TAB-052).
//!
//! Ports `askClose` in `desktop/ui/app.js` and `on_delete` and
//! `quit_safely` in `desktop/winspace.py`. Every close, from the caption's
//! Close button, closing the only tab, or the window manager (Alt+F4, the
//! dock, the shell), is refused with a toast while an update installs
//! (UPD-005).
//!
//! The Python app refused every close while a file operation ran (the
//! "A file operation is running" dialog and a toast for window-manager
//! closes), so the user had to find Cancel and try again. Here closing a
//! window asks instead (Quit is still refused with its toast while any
//! window writes): "Keep open" leaves everything running, and "Cancel and
//! close" cancels the operation, as its Cancel button does, and closes the
//! window once it has stopped and its result has been read. Data safety is
//! unchanged: the window never goes away while a write runs, and what was
//! finished stays finished (OPS-022). The Python app's third refusal, "A
//! tab is moving", does not apply: a tab moves between windows of this
//! process in one step ([`super::tab_moves`]), so no window ever waits for
//! a handoff.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::actions::plain_action;
use super::dialog::Dialog;
use super::window_action::WindowAction;
use super::BrowserWindow;
use super::ButtonStyle;

/// The caption's refusal while an update installs (`askClose`).
const WAIT_FOR_UPDATE: &str = "Wait for the update to finish before closing OpenXplorer.";

/// The refusal of Quit while any window writes (`quit_safely`).
pub(crate) const QUIT_WHILE_WRITING: &str =
    "Finish or cancel active file operations before quitting OpenXplorer.";

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
    /// Adds `win.close-window`, the caption's Close.
    pub(super) fn install_closing_actions(&self) {
        self.add_action_entries([plain_action(
            WindowAction::CloseWindow,
            BrowserWindow::request_close,
        )]);
    }

    /// The caption's Close, and closing the only tab (`askClose`): says
    /// why the window stays open while an update installs, or closes it,
    /// which asks first while it writes files ([`Self::may_close_now`]).
    pub(super) fn request_close(&self) {
        if self.context().updates().close_refusal().is_some() {
            self.show_message(WAIT_FOR_UPDATE);
            return;
        }
        self.close();
    }

    /// Why the window may not close now, if it may not: an update
    /// installs (UPD-005).
    pub(super) fn close_refusal(&self) -> Option<String> {
        self.context().updates().close_refusal()
    }

    /// Whether this window writes files now, which refuses Quit
    /// (`quit_safely`).
    pub(crate) fn has_running_write(&self) -> bool {
        self.is_writing_files()
    }

    /// Starts a stand-in write that runs until [`Self::end_test_write`],
    /// for tests outside the window's modules; false when one runs.
    #[cfg(test)]
    pub(crate) fn begin_test_write(&self) -> bool {
        self.begin_operation("Preparing copy…").is_some()
    }

    /// Ends the stand-in write of [`Self::begin_test_write`].
    #[cfg(test)]
    pub(crate) fn end_test_write(&self) {
        self.end_operation();
    }

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
        dialog.add_button(KEEP_OPEN, ButtonStyle::Bordered);
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

    /// Cancels every write and closes the dialogs over the window, which
    /// cancels a question such as the conflict dialog: a test that ends
    /// while its window writes lets it stop before the window closes.
    #[cfg(test)]
    pub(crate) fn stop_writing_for_test(&self) {
        self.cancel_every_write();
        for window in gtk::Window::list_toplevels() {
            let Ok(window) = window.downcast::<gtk::Window>() else {
                continue;
            };
            if window.transient_for().as_ref() == Some(self.upcast_ref::<gtk::Window>()) {
                window.close();
            }
        }
    }

    /// Cancels the running write: a file operation, an extraction, a
    /// compression or a restored copy.
    fn cancel_every_write(&self) {
        self.transfer_panel().cancel();
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
