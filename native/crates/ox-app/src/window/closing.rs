// SPDX-License-Identifier: AGPL-3.0-only
//! When a window may close (TAB-049, TAB-052).
//!
//! Ports `askClose` in `desktop/ui/app.js` and `on_delete` and
//! `quit_safely` in `desktop/winspace.py`. Two paths lead here:
//!
//! - The caption's Close button and closing the only tab ask first, as
//!   `askClose` does: a toast while an update installs, and the dialog "A
//!   file operation is running" while a write runs.
//! - Every other close, from the window manager (Alt+F4, the dock, the
//!   shell) or from Quit, reaches [`BrowserWindow::close_refusal`], which
//!   refuses with the toast of `on_delete`.
//!
//! Data safety rule "a window never closes under a running write": both
//! paths refuse while [`BrowserWindow::is_writing_files`] holds, so a copy
//! or move is never cut off by closing its window. The Python app's third
//! refusal, "A tab is moving", does not apply: a tab moves between windows
//! of this process in one step ([`super::tab_moves`]), so no window ever
//! waits for a handoff.

use gtk::prelude::*;

use super::actions::plain_action;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The caption's refusal while an update installs (`askClose`).
const WAIT_FOR_UPDATE: &str = "Wait for the update to finish before closing OpenXplorer.";

/// The title of the caption's dialog while a write runs (`askClose`).
const OPERATION_RUNNING_TITLE: &str = "A file operation is running";

/// The text of the caption's dialog while a write runs (`askClose`).
const OPERATION_RUNNING_TEXT: &str =
    "Cancel the operation and wait for its result before closing OpenXplorer.";

/// The refusal of any other close while a write runs (`on_delete`).
const WRITES_STILL_FINISHING: &str = "A file operation is still finishing. Wait or cancel it before closing.";

/// The refusal of Quit while any window writes (`quit_safely`).
pub(crate) const QUIT_WHILE_WRITING: &str =
    "Finish or cancel active file operations before quitting OpenXplorer.";

impl BrowserWindow {
    /// Adds `win.close-window`, the caption's Close.
    pub(super) fn install_closing_actions(&self) {
        self.add_action_entries([plain_action(
            WindowAction::CloseWindow,
            BrowserWindow::request_close,
        )]);
    }

    /// The caption's Close, and closing the only tab (`askClose`): says
    /// why the window stays open, or closes it.
    pub(super) fn request_close(&self) {
        if self.context().updates().close_refusal().is_some() {
            self.show_message(WAIT_FOR_UPDATE);
            return;
        }
        if self.is_writing_files() {
            self.show_result_dialog(OPERATION_RUNNING_TITLE, OPERATION_RUNNING_TEXT);
            return;
        }
        self.close();
    }

    /// Why the window may not close now, if it may not: an update
    /// installs (UPD-005) or a write runs (`on_delete`).
    pub(super) fn close_refusal(&self) -> Option<String> {
        if let Some(refusal) = self.context().updates().close_refusal() {
            return Some(refusal);
        }
        self.is_writing_files().then(|| WRITES_STILL_FINISHING.to_owned())
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
}
