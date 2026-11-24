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
//!
//! Another application's Open or Save dialog (INT-032) is resized like
//! any window and keeps a size of its own, as Windows' common file dialog
//! does: it opens at the size the last dialog was left at, within the same
//! minimum, and saves it the same way, never maximized and never over the
//! windows' size ([`BrowserWindow::open_dialog_at_saved_size`]).
//!
//! As in Windows, a window never opens larger than the screen's work area,
//! whatever size was saved or the default asks: on a 1366 × 768 laptop, or
//! a screen at 150 % or 200 %, it opens fitted to the screen, in logical
//! pixels, and keeps the saved size for a larger screen. GTK fits a window
//! to the work area on X11, but on Wayland only to the whole monitor, if at
//! all: the panels are the compositor's, so there the window leaves them
//! room ([`PANEL_ROOM`]). A saved size that fits is kept as it is.

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

/// The size of another application's Open or Save dialog when nothing was
/// saved: about Windows 11's Save As dialog at 100 %.
const DIALOG_SIZE: WindowSize = WindowSize {
    width: 980,
    height: 640,
    maximized: false,
};

/// The room a window leaves for the desktop's panels and its own
/// placement on Wayland, in logical pixels, where GTK knows no work area:
/// a top or bottom panel of about 32 pixels and the compositor's margin.
const PANEL_ROOM: (i32, i32) = (32, 64);

/// The window's least size, as `window.ui` asks for it
/// (`set_size_request(670, 470)` in `v2.0.0:desktop/winspace.py`).
const LEAST_SIZE: (i32, i32) = (670, 470);

/// The room a window has on a monitor `geometry` wide and high, in
/// logical pixels: all of it where GTK fits windows to the work area
/// itself, else less [`PANEL_ROOM`].
fn room_on(geometry: (i32, i32), knows_work_area: bool) -> (i32, i32) {
    if knows_work_area {
        geometry
    } else {
        (geometry.0 - PANEL_ROOM.0, geometry.1 - PANEL_ROOM.1)
    }
}

/// `size` fitted into `room`: each side kept when it fits, else as long
/// as the room.
fn fit_into(size: (i32, i32), room: (i32, i32)) -> (i32, i32) {
    (size.0.min(room.0), size.1.min(room.1))
}

impl BrowserWindow {
    /// The room a new window has on the screen, in logical pixels: on the
    /// monitor of the window in front, else on the first one. `None`
    /// without a monitor.
    fn room_on_screen(&self) -> Option<(i32, i32)> {
        let display = WidgetExt::display(self);
        let front = self
            .application()
            .and_then(|application| application.active_window())
            .and_then(|window| window.surface());
        let monitor = front
            .and_then(|surface| display.monitor_at_surface(&surface))
            .or_else(|| display.monitors().item(0).and_downcast::<gtk::gdk::Monitor>())?;
        let geometry = monitor.geometry();
        let knows_work_area = !display.is::<gdk_wayland::WaylandDisplay>();
        Some(room_on((geometry.width(), geometry.height()), knows_work_area))
    }

    /// Gives the window `size`, fitted to the screen ([`fit_into`]), and
    /// lowers its least size on a screen smaller than it.
    fn set_fitted_default_size(&self, size: (i32, i32)) {
        let Some(room) = self.room_on_screen() else {
            self.set_default_size(size.0, size.1);
            return;
        };
        let (least_width, least_height) = fit_into(LEAST_SIZE, room);
        self.set_size_request(least_width, least_height);
        let (width, height) = fit_into(size, room);
        self.set_default_size(width, height);
    }

    /// Shows a new window at the saved size and saves its size from now
    /// on.
    pub(crate) fn present_as_new_window(&self) {
        let maximized = self.open_at_saved_size();
        self.present();
        self.remember_size(maximized);
        self.imp().remembers_session.set(true);
        crate::startup_trace::mark("presented");
    }

    /// Gives the window the saved size, else the default one, fitted to
    /// the screen, before it is shown. Returns whether it asked to be
    /// maximized.
    fn open_at_saved_size(&self) -> bool {
        let size = self
            .context()
            .settings_data()
            .preferences
            .window_size
            .unwrap_or(DEFAULT_SIZE);
        self.set_fitted_default_size((pixels(size.width), pixels(size.height)));
        if size.maximized {
            self.maximize();
        }
        size.maximized
    }

    /// Gives another application's Open or Save dialog the size the last
    /// one was left at, else [`DIALOG_SIZE`], before it is shown.
    pub(super) fn open_dialog_at_saved_size(&self) {
        let size = self
            .context()
            .settings_data()
            .preferences
            .dialog_size
            .unwrap_or(DIALOG_SIZE);
        self.set_fitted_default_size((pixels(size.width), pixels(size.height)));
    }

    /// Saves the dialog's size whenever the user changes it, once it is
    /// shown, as [`Self::remember_size`] does for a window.
    pub(super) fn remember_dialog_size(&self) {
        self.remember_size(false);
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
        // A dialog's size is saved as it is when not maximized, and it
        // never opens maximized.
        let size = self.size_now(!self.is_picking() && self.is_maximized_now());
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

    /// Saves the size last noted, unless it is the one saved already: a
    /// dialog's as the dialogs' size, a window's as the windows'.
    fn save_size(&self) {
        let Some(size) = self.imp().size_to_save.get() else {
            return;
        };
        let preferences = self.context().settings_data().preferences;
        let (saved, preference) = if self.is_picking() {
            (
                preferences.dialog_size.unwrap_or(DIALOG_SIZE),
                Preference::DialogSize(size),
            )
        } else {
            (
                preferences.window_size.unwrap_or(DEFAULT_SIZE),
                Preference::WindowSize(size),
            )
        };
        if saved != size {
            self.save_preference(preference);
        }
    }
}

/// A saved size in pixels, as GTK takes it.
fn pixels(size: u32) -> i32 {
    i32::try_from(size).unwrap_or(i32::MAX)
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

    /// A new window fits a 1366 × 768 laptop at 100 % and a 1650 × 1050
    /// screen at 200 % (825 × 525 logical pixels), also on Wayland, where
    /// it leaves the panels room; a size that fits is kept.
    ///
    /// parity: TAB-054
    #[test]
    fn a_window_fits_small_and_scaled_screens() {
        let default = (1320, 810);
        assert_eq!(fit_into(default, room_on((1366, 768), true)), (1320, 768));
        assert_eq!(fit_into(default, room_on((1366, 768), false)), (1320, 704));
        assert_eq!(fit_into(default, room_on((825, 525), false)), (793, 461));
        assert_eq!(fit_into(LEAST_SIZE, room_on((825, 525), false)), (670, 461));
        assert_eq!(fit_into((900, 640), room_on((1920, 1080), false)), (900, 640));
        assert_eq!(fit_into(default, room_on((2560, 1440), true)), default);
    }

    /// The room a window has on the private display's only monitor.
    fn screen_room() -> (i32, i32) {
        let display = gtk::gdk::Display::default().expect("the private display");
        let (width, height) = screen_size();
        room_on((width, height), !display.is::<gdk_wayland::WaylandDisplay>())
    }

    /// The logical size of the private display's only monitor.
    fn screen_size() -> (i32, i32) {
        let display = gtk::gdk::Display::default().expect("the private display");
        let monitor = display
            .monitors()
            .item(0)
            .and_downcast::<gtk::gdk::Monitor>()
            .expect("the private display has a monitor");
        let geometry = monitor.geometry();
        (geometry.width(), geometry.height())
    }

    /// A saved size larger than the screen opens fitted to it, never off
    /// its edges, and that fitted size is not saved over the user's; a
    /// saved size that fits is kept as it is.
    ///
    /// parity: TAB-054
    #[gtk::test]
    fn a_saved_size_larger_than_the_screen_opens_fitted_to_it() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let (screen_width, screen_height) = screen_room();
        let fits_the_screen = |(width, height): (i32, i32)| width <= screen_width && height <= screen_height;
        assert_eq!(saved_size(&test), None, "nothing saved yet");
        let window = open_new_window(&test, &fixture);
        wait_until("the first window to be drawn", || window.width() > 0);
        let first_size = window.default_size();
        let drawn = (window.width(), window.height());
        window.close();
        assert!(fits_the_screen(drawn), "the first window is drawn at {drawn:?}");
        settle();
        assert!(
            fits_the_screen(first_size),
            "the first window, {first_size:?}, in {screen_width} × {screen_height} of room"
        );

        let too_large = WindowSize {
            width: 4000,
            height: 3000,
            maximized: false,
        };
        test.window.save_preference(Preference::WindowSize(too_large));
        wait_until("the size to be saved", || saved_size(&test) == Some(too_large));

        let window = open_new_window(&test, &fixture);
        wait_until("the window to be drawn", || window.width() > 0);
        let drawn = (window.width(), window.height());
        let (width, height) = window.default_size();
        assert!(
            fits_the_screen(drawn),
            "the restored window is drawn at {drawn:?}"
        );
        let starts_a_save = window.imp().size_save.borrow().is_some();
        window.close();
        settle();
        assert!(
            width <= screen_width && height <= screen_height,
            "{width} × {height} in {screen_width} × {screen_height} of room"
        );
        let least = fit_into(LEAST_SIZE, (screen_width, screen_height));
        assert!(
            width >= least.0 && height >= least.1,
            "never under the least size"
        );
        assert!(!starts_a_save, "fitting it saves nothing");
        assert_eq!(saved_size(&test), Some(too_large));

        // 900 × 640, or less on a screen with less room, as long as
        // the settings keep it.
        let fits = WindowSize {
            width: u32::try_from(screen_width.min(900)).unwrap_or(900).max(670),
            height: u32::try_from(screen_height.min(640)).unwrap_or(640).max(470),
            maximized: false,
        };
        test.window.save_preference(Preference::WindowSize(fits));
        wait_until("the size that fits", || saved_size(&test) == Some(fits));
        let window = open_new_window(&test, &fixture);
        let size = window.default_size();
        window.close();
        settle();
        let kept = (pixels(fits.width), pixels(fits.height));
        assert_eq!(
            size,
            fit_into(kept, (screen_width, screen_height)),
            "a size that fits is kept"
        );
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
