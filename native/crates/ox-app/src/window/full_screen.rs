// SPDX-License-Identifier: AGPL-3.0-only
//! Full screen with F11, as in Windows 11 File Explorer (CMD-036).
//!
//! F11 asks the desktop to show the window full screen, and F11 again
//! gives it back the size it had, maximized or not: GTK keeps the size of
//! a full-screen window as it keeps a maximized one's, so the size saved
//! for the next window (TAB-054) is never the screen's, and a size still
//! waiting to be saved is saved before full screen begins. The window
//! remembers which way F11 last asked and follows the desktop when full
//! screen ends another way, such as the desktop's own key, so the next
//! F11 asks the right way. A file dialog keeps its size: the picker turns
//! the action off.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::actions::plain_action;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds `win.full-screen` (F11).
    pub(super) fn install_full_screen(&self) {
        self.add_action_entries([plain_action(
            WindowAction::FullScreen,
            BrowserWindow::toggle_full_screen,
        )]);
        self.connect_fullscreened_notify(|window| {
            window.imp().wants_full_screen.set(window.is_fullscreen());
        });
    }

    /// Makes the window full screen, or gives it back its size.
    fn toggle_full_screen(&self) {
        let full_screen = !self.imp().wants_full_screen.get();
        self.imp().wants_full_screen.set(full_screen);
        if full_screen {
            // Full screen saves no size, so a size still waiting to be
            // saved is saved now, before the window loses it.
            self.save_pending_size();
            self.fullscreen();
        } else {
            self.unfullscreen();
        }
    }
}
