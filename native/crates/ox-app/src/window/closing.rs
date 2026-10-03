// SPDX-License-Identifier: AGPL-3.0-only
//! When a window may close (TAB-049, TAB-052).
//!
//! Ports `askClose` in `v2.0.0:desktop/ui/app.js` and `on_delete` and
//! `quit_safely` in `v2.0.0:desktop/winspace.py`. Every close, from the caption's
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
use super::window_action::WindowAction;
use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;

/// The caption's refusal while an update installs (`askClose`).
const WAIT_FOR_UPDATE: &str =
    crate::i18n::message_id("Wait for the update to finish before closing OpenXplorer.");

/// The refusal of Quit while any window writes (`quit_safely`).
pub(crate) const QUIT_WHILE_WRITING: &str =
    crate::i18n::message_id("Finish or cancel active file operations before quitting OpenXplorer.");

/// The question's title (`A file operation is running` in app.js).
const RUNNING_TITLE: &str = crate::i18n::message_id("A file operation is running");

/// The question.
const RUNNING_QUESTION: &str = crate::i18n::message_id(
    "Cancel it and close this window once it has stopped? Items already finished stay where they are.",
);

/// The answer that keeps the window and the operation.
const KEEP_OPEN: &str = crate::i18n::message_id("Keep open");

/// The answer that cancels the operation, then closes the window.
const CANCEL_AND_CLOSE: &str = crate::i18n::message_id("Cancel and close");

/// The question before a window with several tabs closes, when the
/// settings ask for it (Dolphin's `ConfirmClosingMultipleTabs`, SET-010).
const CLOSE_TABS_TITLE: &str = crate::i18n::message_id("Close all tabs?");

/// The answer that closes the window and its tabs.
const CLOSE_TABS: &str = crate::i18n::message_id("Close all tabs");

/// The question before Quit closes windows with several tabs.
const QUIT_TABS_TITLE: &str = crate::i18n::message_id("Quit OpenXplorer?");

/// The answer that closes every window and quits.
const QUIT_TABS: &str = crate::i18n::message_id("Quit");

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
            self.show_message(ox_core::i18n::gettext_static(WAIT_FOR_UPDATE));
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
        self.begin_operation(ox_core::i18n::gettext_static("Preparing copy…"))
            .is_some()
    }

    /// Ends the stand-in write of [`Self::begin_test_write`].
    #[cfg(test)]
    pub(crate) fn end_test_write(&self) {
        self.end_operation();
    }

    /// Whether a close may go ahead now; otherwise asks whether to cancel
    /// the running write and close afterwards, and the close waits.
    pub(super) fn may_close_now(&self) -> bool {
        if self.asks_before_closing_tabs() {
            // One question at a time, however often the shell asks.
            if self.imp().closing.get() == ClosingState::Open {
                self.imp().closing.set(ClosingState::Asking);
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    async move { window.ask_to_close_tabs().await }
                ));
            }
            return false;
        }
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

    /// Whether closing must first ask about the window's tabs: the
    /// settings ask for it, it has several, and nobody agreed yet.
    pub(crate) fn asks_before_closing_tabs(&self) -> bool {
        let asks = self.context().settings_data().preferences.confirm_close_tabs;
        asks && !self.imp().closing_tabs_confirmed.get() && self.tab_count() > 1
    }

    /// Asks once, for Quit, whether to close `windows`, the windows with
    /// several tabs, as Dolphin asks on Quit; when the user agrees, none of
    /// them asks again as it closes. False, without a second question,
    /// while a question is already open over this window.
    pub(crate) async fn confirm_quit_with_tabs(&self, windows: &[BrowserWindow]) -> bool {
        if self.imp().closing.get() != ClosingState::Open {
            return false;
        }
        self.imp().closing.set(ClosingState::Asking);
        let tabs: usize = windows.iter().map(BrowserWindow::tab_count).sum();
        let question = if windows.len() == 1 {
            ox_core::i18n::format_message(
                "This window has {tabs} tabs open. Close them all and quit?",
                &[("tabs", &tabs.to_string())],
            )
        } else {
            ox_core::i18n::format_message(
                "{len} windows have {tabs} tabs open. Close them all and quit?",
                &[("len", &windows.len().to_string()), ("tabs", &tabs.to_string())],
            )
        };
        let dialog = Dialog::new(self, ox_core::i18n::gettext_static(QUIT_TABS_TITLE), &question);
        dialog.add_cancel_button();
        let quit = dialog.add_button(ox_core::i18n::gettext_static(QUIT_TABS), ButtonStyle::Accent);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        self.imp().closing.set(ClosingState::Open);
        let confirmed = answer == Some(quit);
        if confirmed {
            for window in windows {
                window.imp().closing_tabs_confirmed.set(true);
            }
        }
        confirmed
    }

    /// Asks whether to close the window with its tabs, and closes it.
    async fn ask_to_close_tabs(&self) {
        let count = self.tab_count();
        let question = ox_core::i18n::format_message(
            "This window has {count} tabs open. Close them all?",
            &[("count", &count.to_string())],
        );
        let dialog = Dialog::new(self, ox_core::i18n::gettext_static(CLOSE_TABS_TITLE), &question);
        dialog.add_cancel_button();
        let close = dialog.add_button(ox_core::i18n::gettext_static(CLOSE_TABS), ButtonStyle::Accent);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        self.imp().closing.set(ClosingState::Open);
        if answer == Some(close) {
            self.imp().closing_tabs_confirmed.set(true);
            self.close();
        }
    }

    /// Asks whether to cancel the running write and close the window.
    async fn ask_to_cancel_and_close(&self) {
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(RUNNING_TITLE),
            ox_core::i18n::gettext_static(RUNNING_QUESTION),
        );
        // First, so it has the focus and Enter never cancels by accident.
        dialog.add_button(ox_core::i18n::gettext_static(KEEP_OPEN), ButtonStyle::Bordered);
        let close = dialog.add_button(
            ox_core::i18n::gettext_static(CANCEL_AND_CLOSE),
            ButtonStyle::Danger,
        );
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer != Some(close) {
            self.imp().closing.set(ClosingState::Open);
            self.imp().closing_tabs_confirmed.set(false);
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
        self.cancel_operation();
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
