// SPDX-License-Identifier: AGPL-3.0-only
//! New windows open at the last window's size (TAB-054).
//!
//! The Python app always opened at 1320 × 810 (`set_default_size` in
//! `desktop/winspace.py`); Dolphin and Nautilus remember the size and the
//! maximized state, and so does this. A window saves its size a moment
//! after the user stops resizing it, or maximizes or restores it, and when
//! it closes; the size saved is the one it has when not maximized, and
//! within the window's minimum of 670 × 470.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::WindowSize;

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
    /// Gives the window the saved size, before it is shown.
    pub(crate) fn open_at_saved_size(&self) {
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
    pub(crate) fn remember_size(&self) {
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
        let size = WindowSize {
            width,
            height,
            maximized: self.is_maximized(),
        };
        let saved = self.context().settings_data().preferences.window_size;
        if saved.unwrap_or(DEFAULT_SIZE) != size {
            self.save_preference(Preference::WindowSize(size));
        }
    }
}
