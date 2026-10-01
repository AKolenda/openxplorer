// SPDX-License-Identifier: AGPL-3.0-only
//! New windows open at the last window's size (TAB-054).
//!
//! The Python app always opened at 1320 × 810 (`set_default_size` in
//! `v2.0.0:desktop/winspace.py`); Dolphin and Nautilus remember the size and the
//! maximized state, and so does this. A window saves its size a moment
//! after the user stops resizing it, or maximizes or restores it, and when
//! it closes; the size saved is the one it has when not maximized, kept
//! within the 670 × 470 minimum the settings accept. Every browser window
//! opens through [`BrowserWindow::present_as_new_window`]; snapshot
//! windows set their own size and do not save it.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{WindowSize, WINDOW_HEIGHTS, WINDOW_WIDTHS};

use super::preferences::Preference;
use super::BrowserWindow;

/// How long after the last change of size the window saves it.
const SAVE_DELAY: Duration = Duration::from_millis(800);

/// The size of a window when nothing was saved (`window.ui`).
const DEFAULT_SIZE: WindowSize = WindowSize {
    width: 1320,
    height: 810,
    maximized: false,
};

impl BrowserWindow {
    /// Shows a new window at the saved size and saves its size from now
    /// on.
    pub(crate) fn present_as_new_window(&self) {
        self.open_at_saved_size();
        self.present();
        self.remember_size();
    }

    /// Gives the window the saved size, before it is shown.
    fn open_at_saved_size(&self) {
        let Some(size) = self.context().settings_data().preferences.window_size else {
            return;
        };
        let width = i32::try_from(size.width).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height).unwrap_or(i32::MAX);
        self.set_default_size(width, height);
        if size.maximized {
            self.maximize();
        }
    }

    /// Saves the window's size whenever the user changes it, once it is
    /// shown.
    fn remember_size(&self) {
        for property in ["default-width", "default-height", "maximized"] {
            self.connect_notify_local(Some(property), |window, _| window.schedule_size_save());
        }
    }

    /// Saves the size [`SAVE_DELAY`] after the last change.
    fn schedule_size_save(&self) {
        if let Some(timer) = self.imp().size_save.take() {
            timer.remove();
        }
        let timer = glib::timeout_add_local_once(
            SAVE_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().size_save.replace(None);
                    window.save_size();
                }
            ),
        );
        self.imp().size_save.replace(Some(timer));
    }

    /// Saves a size change that is still waiting, as the window closes.
    pub(super) fn save_pending_size(&self) {
        if let Some(timer) = self.imp().size_save.take() {
            timer.remove();
            self.save_size();
        }
    }

    /// Saves the window's size, unless it is the one saved already.
    fn save_size(&self) {
        let (width, height) = self.default_size();
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return;
        };
        // A window narrower than the minimum saves the minimum rather than
        // nothing, so the next window does not open at an older size.
        let size = WindowSize {
            width: width.clamp(*WINDOW_WIDTHS.start(), *WINDOW_WIDTHS.end()),
            height: height.clamp(*WINDOW_HEIGHTS.start(), *WINDOW_HEIGHTS.end()),
            maximized: self.is_maximized(),
        };
        let saved = self.context().settings_data().preferences.window_size;
        if saved.unwrap_or(DEFAULT_SIZE) != size {
            self.save_preference(Preference::WindowSize(size));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{application, settle, wait_until, Fixture, TestWindow};

    /// A new window opens maximized when the last one was.
    ///
    /// parity: TAB-054
    #[gtk::test]
    fn a_new_window_opens_maximized_when_the_last_one_was() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let saved = WindowSize {
            width: 900,
            height: 640,
            maximized: true,
        };
        test.window.save_preference(Preference::WindowSize(saved));
        wait_until("the size to be saved", || {
            test.context.settings_data().preferences.window_size == Some(saved)
        });

        let window = BrowserWindow::new(&application(), &test.context);
        window.add_tab(&fixture.uri()).expect("valid folder");
        window.present_as_new_window();
        let maximized = window.is_maximized();
        let size = window.default_size();
        window.close();
        settle();

        assert!(maximized);
        assert_eq!(size, (900, 640), "the size it restores to");
    }
}
