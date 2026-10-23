// SPDX-License-Identifier: AGPL-3.0-only
//! New windows open at the last window's size (TAB-054).
//!
//! The Python app always opened at 1320 × 810 (`set_default_size` in
//! `v2.0.0:desktop/winspace.py`); Dolphin and Nautilus remember the size and the
//! maximized state, and so does this. A window saves its size a moment
//! after the user stops resizing it, or maximizes or restores it, and when
//! it closes; the size saved is the one it has when not maximized, kept
//! within the 670 × 470 minimum the settings accept. Full screen is never
//! saved, from F11 or the desktop: the size and state the window had
//! before it are noted as they change and saved as usual. Every browser window
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
        self.imp().remembers_session.set(true);
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
            self.connect_notify_local(Some(property), |window, _| {
                // A save already waiting stays; full screen and back starts
                // none, as the window's own size and state did not change.
                if window.note_size() {
                    window.schedule_size_save();
                }
            });
        }
    }

    /// Notes the size and maximized state to save, unless the window is
    /// full screen: GTK calls a full-screen window not maximized, and it
    /// says so only once the window is full screen, so the note keeps the
    /// state from before full screen, however full screen began. Returns
    /// whether the note changed.
    fn note_size(&self) -> bool {
        if self.is_full_screen_now() {
            return false;
        }
        let (width, height) = self.default_size();
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return false;
        };
        // A window narrower than the minimum saves the minimum rather than
        // nothing, so the next window does not open at an older size.
        let size = Some(WindowSize {
            width: width.clamp(*WINDOW_WIDTHS.start(), *WINDOW_WIDTHS.end()),
            height: height.clamp(*WINDOW_HEIGHTS.start(), *WINDOW_HEIGHTS.end()),
            maximized: self.is_maximized(),
        });
        self.imp().size_to_save.replace(size) != size
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

    /// Saves the size last noted, unless it is the one saved already.
    fn save_size(&self) {
        let Some(size) = self.imp().size_to_save.get() else {
            return;
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
    use crate::window::window_action::WindowAction;

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

    /// Full screen never loses a size still waiting to be saved, whether
    /// F11 or the desktop (its own key or window menu) starts it: a window
    /// resized and then closed while full screen saves the size it had
    /// before full screen.
    ///
    /// parity: TAB-054, CMD-036
    #[gtk::test]
    fn a_size_still_waiting_survives_full_screen_from_f11_or_the_desktop() {
        for from_f11 in [true, false] {
            let fixture = Fixture::standard();
            let test = TestWindow::open(&fixture.uri());
            let window = BrowserWindow::new(&application(), &test.context);
            window.add_tab(&fixture.uri()).expect("valid folder");
            window.present_as_new_window();
            settle();

            window.set_default_size(900, 640);
            assert!(window.imp().size_save.borrow().is_some(), "a save waits");
            if from_f11 {
                WindowAction::FullScreen.activate_from(&window, None);
            }
            // The desktop's answer, which the private display cannot give.
            window.imp().full_screen_for_tests.set(Some(true));
            window.notify("fullscreened");
            window.close();

            let what = format!("the size before full screen to be saved (F11: {from_f11})");
            wait_until(&what, || {
                test.context
                    .settings_data()
                    .preferences
                    .window_size
                    .map(|size| (size.width, size.height))
                    == Some((900, 640))
            });
        }
    }

    /// Full screen and back saves nothing, also where the desktop calls a
    /// full-screen window not maximized: a second window's newer size
    /// stays saved, as only a change of a window's own size or maximized
    /// state outside full screen is saved.
    ///
    /// parity: TAB-054, CMD-036
    #[gtk::test]
    fn full_screen_and_back_keeps_another_windows_newer_size() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let saved_size = || {
            test.context
                .settings_data()
                .preferences
                .window_size
                .map(|size| (size.width, size.height))
        };
        let open_window = || {
            let window = BrowserWindow::new(&application(), &test.context);
            window.add_tab(&fixture.uri()).expect("valid folder");
            window.present_as_new_window();
            settle();
            window
        };
        let first = open_window();
        first.set_default_size(1000, 700);
        wait_until("the first window's size", || saved_size() == Some((1000, 700)));
        let second = open_window();
        second.set_default_size(900, 640);
        wait_until("the second window's size", || saved_size() == Some((900, 640)));

        // The desktop's answers to F11 and F11 again, which the private
        // display cannot give, with the maximized state it reports.
        WindowAction::FullScreen.activate_from(&first, None);
        first.imp().full_screen_for_tests.set(Some(true));
        first.notify("fullscreened");
        first.notify("maximized");
        WindowAction::FullScreen.activate_from(&first, None);
        first.imp().full_screen_for_tests.set(Some(false));
        first.notify("fullscreened");
        first.notify("maximized");
        let save_started = first.imp().size_save.borrow().is_some();
        first.close();
        second.close();
        settle();

        assert!(!save_started, "full screen and back starts no save");
        assert_eq!(saved_size(), Some((900, 640)), "the newer size stays saved");
    }
}
