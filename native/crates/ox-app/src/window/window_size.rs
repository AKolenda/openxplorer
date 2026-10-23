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
        let maximized = self.open_at_saved_size();
        self.present();
        self.remember_size(maximized);
        self.imp().remembers_session.set(true);
    }

    /// Gives the window the saved size, before it is shown. Returns
    /// whether it asked to be maximized.
    fn open_at_saved_size(&self) -> bool {
        let Some(size) = self.context().settings_data().preferences.window_size else {
            return false;
        };
        let width = i32::try_from(size.width).unwrap_or(i32::MAX);
        let height = i32::try_from(size.height).unwrap_or(i32::MAX);
        self.set_default_size(width, height);
        if size.maximized {
            self.maximize();
        }
        size.maximized
    }

    /// Saves the window's size whenever the user changes it, once it is
    /// shown. The size and state it opened at, `maximized` or not, are
    /// noted first, so the desktop granting them saves nothing, as Windows
    /// Explorer saves only the user's own changes.
    fn remember_size(&self, maximized: bool) {
        self.imp().size_to_save.set(self.size_now(maximized));
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
        let size = self.size_now(self.is_maximized_now());
        size.is_some() && self.imp().size_to_save.replace(size) != size
    }

    /// The window's size now, `maximized` or not, as it is saved.
    fn size_now(&self, maximized: bool) -> Option<WindowSize> {
        let (width, height) = self.default_size();
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return None;
        };
        // A window narrower than the minimum saves the minimum rather than
        // nothing, so the next window does not open at an older size.
        Some(WindowSize {
            width: width.clamp(*WINDOW_WIDTHS.start(), *WINDOW_WIDTHS.end()),
            height: height.clamp(*WINDOW_HEIGHTS.start(), *WINDOW_HEIGHTS.end()),
            maximized,
        })
    }

    /// Whether the window is maximized now.
    fn is_maximized_now(&self) -> bool {
        #[cfg(test)]
        if let Some(maximized) = self.imp().maximized_for_tests.get() {
            return maximized;
        }
        self.is_maximized()
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

    /// A new window for `test`'s folder, shown as the app shows one.
    fn open_new_window(test: &TestWindow, fixture: &Fixture) -> BrowserWindow {
        let window = BrowserWindow::new(&application(), &test.context);
        window.add_tab(&fixture.uri()).expect("valid folder");
        window.present_as_new_window();
        settle();
        window
    }

    /// The size the next window opens at.
    fn saved_size(test: &TestWindow) -> Option<WindowSize> {
        test.context.settings_data().preferences.window_size
    }

    /// Plays the desktop maximizing or restoring `window`, which the
    /// private display cannot do.
    fn set_maximized(window: &BrowserWindow, maximized: bool) {
        window.imp().maximized_for_tests.set(Some(maximized));
        window.notify("maximized");
    }

    /// A window that opens maximized from the saved state starts no save
    /// of its own, and neither does its F11 round trip, also where the
    /// desktop reports it maximized only later and not maximized while
    /// full screen: another window's newer size stays saved.
    ///
    /// parity: TAB-054, CMD-036
    #[gtk::test]
    fn a_window_restored_maximized_saves_nothing_for_its_own_starting_state() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let restored = WindowSize {
            width: 900,
            height: 640,
            maximized: true,
        };
        test.window.save_preference(Preference::WindowSize(restored));
        wait_until("the saved state", || saved_size(&test) == Some(restored));
        let first = open_new_window(&test, &fixture);
        // The desktop answers the maximize request after the window shows.
        set_maximized(&first, true);
        let opening_saves = first.imp().size_save.borrow().is_some();
        let second = open_new_window(&test, &fixture);
        second.set_default_size(1000, 700);
        let newer = WindowSize {
            width: 1000,
            height: 700,
            maximized: true,
        };
        wait_until("the second window's size", || saved_size(&test) == Some(newer));

        WindowAction::FullScreen.activate_from(&first, None);
        first.imp().full_screen_for_tests.set(Some(true));
        first.notify("fullscreened");
        set_maximized(&first, false);
        WindowAction::FullScreen.activate_from(&first, None);
        first.imp().full_screen_for_tests.set(Some(false));
        first.notify("fullscreened");
        set_maximized(&first, true);
        let round_trip_saves = first.imp().size_save.borrow().is_some();
        first.close();
        second.close();
        settle();

        assert!(!opening_saves, "opening at the saved state saves nothing");
        assert!(!round_trip_saves, "full screen and back saves nothing");
        assert_eq!(saved_size(&test), Some(newer), "the newer size stays saved");
    }

    /// A resize is saved once: the same size reported again starts no
    /// second save.
    ///
    /// parity: TAB-054
    #[gtk::test]
    fn a_resize_is_saved_once() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let window = open_new_window(&test, &fixture);

        window.set_default_size(1000, 700);
        let resized = WindowSize {
            width: 1000,
            height: 700,
            maximized: false,
        };
        wait_until("the new size", || saved_size(&test) == Some(resized));
        wait_until("the save to finish", || window.imp().size_save.borrow().is_none());
        window.notify("default-width");
        let saves_again = window.imp().size_save.borrow().is_some();
        window.close();
        settle();

        assert!(!saves_again, "an unchanged size is not saved again");
    }

    /// Maximizing is saved, and so is restoring.
    ///
    /// parity: TAB-054
    #[gtk::test]
    fn maximizing_and_restoring_are_saved() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let window = open_new_window(&test, &fixture);
        let maximized = || saved_size(&test).map(|size| size.maximized);

        set_maximized(&window, true);
        wait_until("the maximized state", || maximized() == Some(true));
        set_maximized(&window, false);
        wait_until("the restored state", || maximized() == Some(false));
        window.close();
        settle();
    }
}
