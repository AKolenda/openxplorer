// SPDX-License-Identifier: AGPL-3.0-only
//! How tall a menu may be: as on Windows, a menu shows in full whenever it
//! fits on the monitor it opens on, and scrolls only when it is taller.
//!
//! A menu reads its monitor each time it opens, and again once its popup
//! shows or enters a monitor, so it fits the monitor it is on even in a
//! window spanning monitors of different heights. It follows the window
//! to another monitor, a monitor plugged in or out, and a change of scale.
//! GDK gives the monitor's size in the same logical pixels as widgets, so
//! the scale needs nothing more. On Wayland a menu cannot know where on
//! the monitor it is, so the limit is the whole monitor less a margin, and
//! GTK's placement keeps the menu on the screen.
//!
//! GTK flips a menu that opens below the pointer up when it does not fit
//! below, but never slides it, and cuts it short when it fits neither below
//! nor above (`create_popup_layout` in GTK 4.14's gtkpopover.c). Windows
//! moves such a menu up until it fits, so a menu at the pointer that may
//! fit on neither side opens beside it instead, its top at the pointer,
//! where GTK slides it up.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::MenuPopover;

/// The room kept between a menu as tall as the monitor and the monitor's
/// top and bottom edges.
const MONITOR_MARGIN: i32 = 8;

/// The limit when no monitor is known, which was every menu's limit before.
pub(super) const UNKNOWN_MONITOR_LIMIT: i32 = 560;

/// The least room the rows of a menu keep, however small the monitor.
const LEAST_ROWS_HEIGHT: i32 = 120;

/// The room a desktop panel or taskbar may take from the monitor.
const PANEL_ALLOWANCE: i32 = 48;

/// The tallest the rows of a menu may be on a monitor `monitor_height`
/// tall, when the rest of the menu (the strip, padding and border) is
/// `chrome` tall.
pub(super) fn rows_limit(monitor_height: i32, chrome: i32) -> i32 {
    (monitor_height - 2 * MONITOR_MARGIN - chrome.max(0)).max(LEAST_ROWS_HEIGHT)
}

/// Whether a menu `menu_height` tall that opens at an anchor
/// `anchor_height` tall on a monitor `monitor_height` tall surely fits
/// below or above it, wherever it is.
pub(super) fn fits_below_or_above(menu_height: i32, anchor_height: i32, monitor_height: i32) -> bool {
    2 * menu_height <= monitor_height - 2 * MONITOR_MARGIN - PANEL_ALLOWANCE - anchor_height.max(0)
}

/// The height of the monitor `surface` is on, or of the first monitor
/// without one, or `None` before `display` has a monitor.
pub(super) fn monitor_height(display: &gdk::Display, surface: Option<&gdk::Surface>) -> Option<i32> {
    let monitor = surface
        .and_then(|surface| display.monitor_at_surface(surface))
        .or_else(|| display.monitors().item(0).and_downcast::<gdk::Monitor>())?;
    let height = monitor.geometry().height();
    (height > 0).then_some(height)
}

impl MenuPopover {
    /// Lets the menu grow as tall as the monitor it opens on.
    pub(super) fn fit_to_monitor(&self) {
        let surface = self.monitor_surface();
        self.fit_to_monitor_height(monitor_height(&self.display(), surface.as_ref()));
    }

    /// The surface whose monitor the menu fits: its own popup once it
    /// shows, which GTK placed on the monitor of its anchor, even when the
    /// window spans monitors of different heights; before that, the
    /// surface it opens from (the window, or the menu a submenu hangs from).
    fn monitor_surface(&self) -> Option<gdk::Surface> {
        if self.is_mapped() {
            if let Some(surface) = self.surface() {
                return Some(surface);
            }
        }
        self.parent()?.native()?.surface()
    }

    /// Fits the menu again whenever its popup enters a monitor, as when
    /// GTK first places it, or places it again on another monitor.
    pub(super) fn follow_monitor_changes(&self) {
        let Some(surface) = self.surface() else { return };
        surface.connect_enter_monitor(glib::clone!(
            #[weak(rename_to = menu)]
            self,
            move |_, _| menu.refit_if_shown()
        ));
    }

    /// Fits the menu again when its rows change while it shows: a menu
    /// filled again and popped up without being hidden first, as "Show
    /// more options" and "New…" do, gets no new show from GTK.
    pub(super) fn refit_if_shown(&self) {
        if self.is_visible() {
            self.fit_to_monitor();
        }
    }

    /// Lets the menu grow as tall as a monitor `monitor_height` tall, or
    /// to [`UNKNOWN_MONITOR_LIMIT`] when that is not known.
    pub(super) fn fit_to_monitor_height(&self, monitor_height: Option<i32>) {
        let Some(rows) = self.row_scroller() else { return };
        let Some(monitor_height) = monitor_height else {
            rows.set_max_content_height(UNKNOWN_MONITOR_LIMIT);
            return;
        };
        // A popover not shown yet measures nothing, so the menu is measured
        // by its frame, GTK's `contents` widget, which also leaves out the
        // shadow GTK keeps out of the placement.
        let Some(frame) = self.first_child() else { return };
        let menu = frame.measure(gtk::Orientation::Vertical, -1).1;
        let chrome = menu - rows.measure(gtk::Orientation::Vertical, -1).1;
        rows.set_max_content_height(rows_limit(monitor_height, chrome));
        if let Some(anchor_height) = self.point_anchor_height() {
            let full_menu = chrome + self.list().measure(gtk::Orientation::Vertical, -1).1;
            self.set_position(if fits_below_or_above(full_menu, anchor_height, monitor_height) {
                gtk::PositionType::Bottom
            } else if self.direction() == gtk::TextDirection::Rtl {
                gtk::PositionType::Left
            } else {
                gtk::PositionType::Right
            });
            self.set_valign(gtk::Align::Start);
        }
    }

    /// The height of the point the menu opens at, as a context menu does:
    /// 1 at the pointer, or a row's height on the strip one pixel wide down
    /// the selected row that the keyboard opens it on (`context_menu.rs`,
    /// `keyboard_menu_anchor`). `None` below a button or beside a row.
    fn point_anchor_height(&self) -> Option<i32> {
        let (set, area) = self.pointing_to();
        (set && area.width() <= 1).then(|| area.height())
    }

    /// The scrolled window around the rows.
    fn row_scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.list()
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::Icon;
    use crate::test_support::harness::{settle, skin, wait_until};
    use crate::window::menu_popover::{MenuEntry, MenuItem, MenuStyle};
    use crate::window::window_action::WindowAction;

    /// The height of the test display's only monitor (check.py's Xvfb).
    const TEST_MONITOR_HEIGHT: i32 = 1000;

    /// A menu of `rows` items open below a button in a short window at the
    /// top of the monitor, so there is room for it below.
    fn open_menu(rows: usize) -> (gtk::Window, MenuPopover) {
        open_menu_in(rows, 40, None)
    }

    /// A menu of `rows` items open in a shown window `window_height` tall,
    /// below its button or, when there is one, at `anchor` in the button.
    fn open_menu_in(
        rows: usize,
        window_height: i32,
        anchor: Option<gdk::Rectangle>,
    ) -> (gtk::Window, MenuPopover) {
        let _ = skin();
        let button = gtk::Button::with_label("Open");
        let window = gtk::Window::builder()
            .default_width(400)
            .default_height(window_height)
            .child(&button)
            .build();
        window.present();
        wait_until("the window to be shown", || window.height() == window_height);
        let entries: Vec<MenuEntry> = (0..rows)
            .map(|row| MenuItem::new(&format!("Item {row}"), Icon::Copy, WindowAction::CopyAddress).into())
            .collect();
        let menu = MenuPopover::new(entries);
        menu.set_parent(&button);
        if let Some(anchor) = anchor {
            menu.set_pointing_to(Some(&anchor));
        }
        menu.popup();
        wait_until("the menu to open", || menu.is_mapped());
        settle();
        (window, menu)
    }

    /// Closes `menu` and `window`.
    fn close(window: &gtk::Window, menu: &MenuPopover) {
        menu.popdown();
        menu.unparent();
        window.destroy();
        settle();
    }

    /// The scrolled window around `menu`'s rows.
    fn rows_of(menu: &MenuPopover) -> gtk::ScrolledWindow {
        menu.row_scroller().expect("the rows are in a scrolled window")
    }

    /// The natural height of all of `menu`'s rows.
    fn all_rows_height(menu: &MenuPopover) -> i32 {
        menu.list().measure(gtk::Orientation::Vertical, -1).1
    }

    /// Whether `menu`'s rows scroll: some of them are out of sight.
    fn scrolls(menu: &MenuPopover) -> bool {
        let adjustment = rows_of(menu).vadjustment();
        adjustment.upper() - adjustment.page_size() > 0.5
    }

    /// Waits until `menu`'s rows are laid out at `height`.
    fn wait_for_rows_height(menu: &MenuPopover, what: &str, height: impl Fn(i32) -> bool) {
        wait_until(what, || height(rows_of(menu).height()));
    }

    #[gtk::test]
    fn the_test_monitor_is_the_one_the_menu_reads() {
        let (window, menu) = open_menu(3);
        let display = WidgetExt::display(&window);
        assert_eq!(
            monitor_height(&display, window.surface().as_ref()),
            Some(TEST_MONITOR_HEIGHT)
        );
        assert_eq!(monitor_height(&display, None), Some(TEST_MONITOR_HEIGHT));
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_shown_menu_fits_the_monitor_its_own_popup_is_on() {
        // A window can span monitors of different heights: the menu's own
        // popup is where GTK placed it, on the monitor of its anchor.
        let (window, menu) = open_menu(3);
        assert!(menu.surface().is_some());
        assert_eq!(menu.monitor_surface(), menu.surface());
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_opening_from_another_menu_fits_that_menus_monitor_until_it_shows() {
        let (window, menu) = open_menu(3);
        let row = menu.rows().into_iter().next().expect("the menu has rows");
        let submenu = MenuPopover::new(entries(3));
        submenu.set_parent(&row);
        assert_eq!(submenu.monitor_surface(), menu.surface());
        submenu.unparent();
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_fits_again_when_its_popup_enters_a_monitor() {
        let (window, menu) = open_menu(80);
        menu.fit_to_monitor_height(Some(700));
        let short = rows_of(&menu).max_content_height();
        let surface = menu.surface().expect("the menu shows");
        let monitor = WidgetExt::display(&window)
            .monitor_at_surface(&surface)
            .expect("the menu is on a monitor");
        surface.emit_by_name::<()>("enter-monitor", &[&monitor]);
        assert!(
            rows_of(&menu).max_content_height() > short,
            "the menu fits the {TEST_MONITOR_HEIGHT} pixel monitor it entered"
        );
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_taller_than_560_pixels_that_fits_on_the_monitor_shows_in_full() {
        let (window, menu) = open_menu(26);
        let natural = all_rows_height(&menu);
        assert!(
            natural > 560 && natural < TEST_MONITOR_HEIGHT - 100,
            "the rows are {natural} pixels: taller than 560 and shorter than the monitor"
        );
        wait_for_rows_height(&menu, "every row to show", |height| height >= natural);
        assert!(!scrolls(&menu), "a menu that fits does not scroll");
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_taller_than_the_monitor_scrolls_and_stays_on_it() {
        let (window, menu) = open_menu(80);
        let natural = all_rows_height(&menu);
        assert!(natural > TEST_MONITOR_HEIGHT, "the rows are {natural} pixels");
        let frame = menu.first_child().expect("the menu has its frame");
        wait_until("the menu to be laid out", || frame.height() > 560);
        assert!(scrolls(&menu), "a menu taller than the monitor scrolls");
        assert!(
            frame.height() <= TEST_MONITOR_HEIGHT - 2 * MONITOR_MARGIN,
            "the {} pixel menu fits on the monitor",
            frame.height()
        );
        assert!(
            frame.height() > TEST_MONITOR_HEIGHT - 120,
            "the {} pixel menu uses the monitor's height",
            frame.height()
        );
        close(&window, &menu);
    }

    #[gtk::test]
    fn the_limit_follows_the_monitor_height() {
        let (window, menu) = open_menu(80);
        let rows = rows_of(&menu);
        menu.fit_to_monitor_height(Some(700));
        let short = rows.max_content_height();
        assert!(
            short > 560 && short <= 700 - 2 * MONITOR_MARGIN,
            "the rows may be {short} pixels"
        );
        wait_for_rows_height(&menu, "the rows to fit a 700 pixel monitor", |height| {
            height == short
        });
        menu.fit_to_monitor_height(Some(900));
        assert_eq!(
            rows.max_content_height() - short,
            200,
            "the limit grows with the monitor"
        );
        wait_for_rows_height(&menu, "the rows to fit a 900 pixel monitor", |height| {
            height == short + 200
        });
        assert!(scrolls(&menu));
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_tall_menu_at_a_point_in_the_middle_of_the_monitor_slides_to_show_in_full() {
        let (window, menu) = open_menu_in(
            26,
            900,
            Some(gdk::Rectangle::new(20, TEST_MONITOR_HEIGHT / 2, 1, 1)),
        );
        let natural = all_rows_height(&menu);
        assert!(
            natural + 40 > TEST_MONITOR_HEIGHT / 2,
            "the {natural} pixel rows fit neither below nor above the point"
        );
        assert_eq!(
            menu.position(),
            gtk::PositionType::Right,
            "the menu opens beside the point"
        );
        assert_eq!(
            menu.valign(),
            gtk::Align::Start,
            "its top at the point when there is room"
        );
        wait_for_rows_height(&menu, "every row to show", |height| height >= natural);
        assert!(!scrolls(&menu), "a menu that fits on the monitor does not scroll");
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_tall_menu_opened_from_the_keyboard_on_a_row_in_the_middle_slides_to_show_in_full() {
        // The keyboard opens the context menu on a strip one pixel wide down
        // the selected row (context_menu.rs, keyboard_menu_anchor).
        let row = gdk::Rectangle::new(20, TEST_MONITOR_HEIGHT / 2 - 15, 1, 30);
        let (window, menu) = open_menu_in(26, 900, Some(row));
        let natural = all_rows_height(&menu);
        assert_eq!(
            menu.position(),
            gtk::PositionType::Right,
            "the menu opens beside the row"
        );
        wait_for_rows_height(&menu, "every row to show", |height| height >= natural);
        assert!(!scrolls(&menu), "a menu that fits on the monitor does not scroll");
        close(&window, &menu);
    }

    /// `rows` menu entries.
    fn entries(rows: usize) -> Vec<MenuEntry> {
        (0..rows)
            .map(|row| MenuItem::new(&format!("Item {row}"), Icon::Copy, WindowAction::CopyAddress).into())
            .collect()
    }

    #[gtk::test]
    fn a_menu_refilled_while_it_shows_fits_its_new_rows() {
        // "Show more options" and "New…" fill the open menu again and pop
        // it up without hiding it first, so GTK does not show it again.
        let (window, menu) = open_menu_in(
            5,
            900,
            Some(gdk::Rectangle::new(20, TEST_MONITOR_HEIGHT / 2, 1, 1)),
        );
        assert_eq!(menu.position(), gtk::PositionType::Bottom);
        menu.fit_to_monitor_height(Some(700));
        menu.set_entries(entries(26));
        menu.popup();
        let natural = all_rows_height(&menu);
        assert_eq!(
            menu.position(),
            gtk::PositionType::Right,
            "the taller menu opens beside the point"
        );
        wait_for_rows_height(&menu, "every row to show", |height| height >= natural);
        assert!(!scrolls(&menu), "the refilled menu fits on the monitor");
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_restyled_while_it_shows_fits_again() {
        let (window, menu) = open_menu_in(
            26,
            900,
            Some(gdk::Rectangle::new(20, TEST_MONITOR_HEIGHT / 2, 1, 1)),
        );
        assert_eq!(menu.position(), gtk::PositionType::Right);
        menu.set_entries(entries(5));
        menu.set_style_and_strip(MenuStyle::Compact, Vec::new());
        menu.popup();
        assert_eq!(
            menu.position(),
            gtk::PositionType::Bottom,
            "the short menu opens below the point again"
        );
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_short_menu_opened_from_the_keyboard_on_a_row_opens_below_it() {
        let (window, menu) = open_menu_in(5, 900, Some(gdk::Rectangle::new(20, 100, 1, 30)));
        assert_eq!(menu.position(), gtk::PositionType::Bottom);
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_short_menu_at_a_point_opens_below_it_and_flips_as_before() {
        let (window, menu) = open_menu_in(5, 900, Some(gdk::Rectangle::new(20, 100, 1, 1)));
        assert_eq!(menu.position(), gtk::PositionType::Bottom);
        assert!(!scrolls(&menu));
        close(&window, &menu);
    }

    #[gtk::test]
    fn a_menu_below_a_button_stays_below_it() {
        let (window, menu) = open_menu(26);
        assert_eq!(menu.position(), gtk::PositionType::Bottom);
        close(&window, &menu);
    }

    #[test]
    fn a_menu_surely_fits_below_or_above_a_point_up_to_half_the_monitor() {
        assert!(fits_below_or_above(400, 1, 1080));
        assert!(fits_below_or_above((1080 - 16 - 48 - 1) / 2, 1, 1080));
        assert!(!fits_below_or_above((1080 - 16 - 48 - 1) / 2 + 1, 1, 1080));
        assert!(!fits_below_or_above(774, 1, 1080));
        assert!(fits_below_or_above(600, 1, 1440));
        // A row the keyboard opens the menu on leaves less room around it.
        assert!(fits_below_or_above((1080 - 16 - 48 - 30) / 2, 30, 1080));
        assert!(!fits_below_or_above((1080 - 16 - 48 - 30) / 2 + 1, 30, 1080));
    }

    #[test]
    fn the_rows_keep_a_margin_and_the_rest_of_the_menu_off_the_monitor() {
        assert_eq!(rows_limit(1080, 10), 1080 - 16 - 10);
        assert_eq!(rows_limit(1440, 10), 1440 - 16 - 10);
        assert_eq!(rows_limit(1080, -4), 1080 - 16);
        assert_eq!(rows_limit(100, 40), LEAST_ROWS_HEIGHT);
    }
}
