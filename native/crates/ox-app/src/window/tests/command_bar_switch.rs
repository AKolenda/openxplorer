// SPDX-License-Identifier: AGPL-3.0-only
//! Moving between the command bar's menus with the real pointer on
//! check.py's private display, as in Windows 11 File Explorer: while one
//! menu is open, one click on another menu button closes it and opens
//! that one, a click on the open menu's own button only closes it, and a
//! click anywhere else closes it and still does what it does there. GTK
//! decides where a press goes while a menu is open, so only real presses
//! show it.

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::command_bar::{command_bar_controls, control_name};
use super::narrow_windows::resize;
use crate::test_support::harness::{
    descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::test_support::pointer::{window_origin, Button, Pointer};
use crate::window::menu_popover::{row_label, MenuPopover};

/// Where the pointer first looks for the window.
const PROBE: (i32, i32) = (8, 8);

/// How long a test watches for something that must not happen.
const QUIET: Duration = Duration::from_millis(300);

/// The menu buttons of the bar every folder shows, left to right.
const MENUS: [&str; 4] = ["New", "Sort", "View", "See more"];

/// The window under test with the real pointer, and where the window is
/// on the screen.
struct Session {
    pointer: Pointer,
    test: TestWindow,
    origin: (i32, i32),
    fixture: Fixture,
}

impl Session {
    /// A window on a folder with a few files, wide or `width` pixels.
    fn open(width: Option<i32>) -> Self {
        Self::open_on(Fixture::standard(), width)
    }

    /// A window on `fixture`'s folder, wide or `width` pixels.
    fn open_on(fixture: Fixture, width: Option<i32>) -> Self {
        let mut pointer = Pointer::connect();
        let test = TestWindow::open(&fixture.uri());
        if let Some(width) = width {
            resize(&test, width);
        }
        wait_for_frames(&test.window, 4);
        let origin = window_origin(&mut pointer, &test.window, PROBE);
        Self {
            pointer,
            test,
            origin,
            fixture,
        }
    }

    /// The screen point of (`x`, `y`) in `widget`, in the window.
    fn on_screen(&self, widget: &impl IsA<gtk::Widget>, (x, y): (f64, f64)) -> (i32, i32) {
        let native = widget.native().expect("a shown widget is in a window");
        #[expect(clippy::cast_possible_truncation, reason = "window positions are small")]
        let point = widget
            .compute_point(&native, &gtk::graphene::Point::new(x as f32, y as f32))
            .expect("the widget is laid out in its window");
        let (shadow_x, shadow_y) = native.surface_transform();
        let (popup_x, popup_y) = popup_offset(&native);
        let scale = f64::from(native.scale_factor());
        #[expect(clippy::cast_possible_truncation, reason = "window positions are small")]
        let to_device = |logical: f64| (logical * scale).round() as i32;
        (
            self.origin.0 + to_device(f64::from(point.x()) + shadow_x + popup_x),
            self.origin.1 + to_device(f64::from(point.y()) + shadow_y + popup_y),
        )
    }

    /// The screen point of the middle of `widget`.
    fn middle_of(&self, widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
        let half = |size: i32| f64::from(size) / 2.0;
        self.on_screen(widget, (half(widget.width()), half(widget.height())))
    }

    /// A real primary click at `point` on the screen.
    fn click_at(&mut self, (x, y): (i32, i32)) {
        self.pointer.move_to(x, y);
        self.pointer.press(Button::Primary);
        self.pointer.release(Button::Primary);
    }

    /// A real primary click on the middle of the menu button `name`.
    fn click_button(&mut self, name: &str) {
        let point = self.middle_of(&menu_button(&self.test, name));
        self.click_at(point);
    }

    /// Opens `name`'s menu with a real click and waits for it.
    fn open_by_click(&mut self, name: &str) {
        self.click_button(name);
        let menu = menu_of(&self.test, name);
        wait_until(&format!("{name}'s menu to open"), || menu.is_mapped());
        wait_for_frames(&menu, 2);
    }

    /// The names of the bar's open menus.
    fn open_menus(&self) -> Vec<String> {
        open_menus(&self.test)
    }

    /// Waits until the only open menu of the bar is `name`'s, or none for
    /// `None`, then watches a moment that no other opens.
    fn expect_only_open(&self, name: Option<&str>, after: &str) {
        let expected: Vec<String> = name.map(str::to_owned).into_iter().collect();
        wait_until(&format!("{after}: open menus {expected:?}"), || {
            self.open_menus() == expected
        });
        wait_for(QUIET);
        assert_eq!(self.open_menus(), expected, "{after}");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for button in bar_menu_buttons(&self.test) {
            button.popdown();
        }
        self.pointer.move_to(720, 990);
    }
}

/// Where the surface of `native` is in its window's surface: a menu's
/// popup is placed relative to the surface it opens from.
fn popup_offset(native: &gtk::Native) -> (f64, f64) {
    let Some(popup) = native.surface().and_downcast::<gdk::Popup>() else {
        return (0.0, 0.0);
    };
    let parent = native.parent().and_then(|parent| parent.native());
    let (x, y) = parent.as_ref().map_or((0.0, 0.0), popup_offset);
    (
        x + f64::from(popup.position_x()),
        y + f64::from(popup.position_y()),
    )
}

/// The bar's menu buttons that show now, with their names.
fn bar_menu_buttons(test: &TestWindow) -> Vec<gtk::MenuButton> {
    command_bar_controls(test)
        .into_iter()
        .filter_map(|control| control.downcast::<gtk::MenuButton>().ok())
        .collect()
}

/// The command bar's menu button named `name`.
fn menu_button(test: &TestWindow, name: &str) -> gtk::MenuButton {
    bar_menu_buttons(test)
        .into_iter()
        .find(|button| control_name(button.upcast_ref()).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the bar shows the menu button {name}"))
}

/// The menu of the bar's menu button `name`.
fn menu_of(test: &TestWindow, name: &str) -> MenuPopover {
    menu_button(test, name)
        .popover()
        .and_downcast::<MenuPopover>()
        .unwrap_or_else(|| panic!("{name} opens an app menu"))
}

/// The names of the bar's menus that are open (shown on screen).
fn open_menus(test: &TestWindow) -> Vec<String> {
    bar_menu_buttons(test)
        .into_iter()
        .filter(|button| button.popover().is_some_and(|menu| menu.is_mapped()))
        .filter_map(|button| control_name(button.upcast_ref()))
        .collect()
}

/// The row of `menu` labeled `label`.
fn row_of(menu: &MenuPopover, label: &str) -> gtk::ListBoxRow {
    let list = menu.row_list();
    std::iter::successors(list.first_child(), WidgetExt::next_sibling)
        .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
        .find(|row| row_label(row).as_deref() == Some(label))
        .unwrap_or_else(|| panic!("the menu has {label}"))
}

/// Whether the keyboard is on `widget` or inside it.
fn has_keyboard(test: &TestWindow, widget: &impl IsA<gtk::Widget>) -> bool {
    gtk::prelude::GtkWindowExt::focus(&test.window)
        .is_some_and(|focus| focus.is_ancestor(widget) || focus == *widget.as_ref())
}

/// As in Windows 11 File Explorer, while a menu of the command bar is
/// open, one click on another menu button closes it and opens that one,
/// for every pair of New, Sort, View and See more; a click on the button
/// of the menu that is open then closes it and opens nothing.
///
/// parity: CMD-005
#[gtk::test]
fn one_click_on_another_menu_button_opens_its_menu() {
    let mut session = Session::open(None);
    for first in MENUS {
        for second in MENUS.into_iter().filter(|name| *name != first) {
            session.open_by_click(first);
            session.click_button(second);
            session.expect_only_open(Some(second), &format!("{first} open, one click on {second}"));
            session.click_button(second);
            session.expect_only_open(None, &format!("{second} open, a click on {second}"));
        }
    }
}

/// While a search shows, its Search options menu takes part too.
///
/// parity: CMD-005
#[gtk::test]
fn search_options_and_view_open_from_each_other_with_one_click() {
    let mut session = Session::open(None);
    session.test.search_for("notes");
    wait_until("Search options to show", || {
        bar_menu_buttons(&session.test)
            .iter()
            .any(|button| control_name(button.upcast_ref()).as_deref() == Some("Search options"))
    });
    wait_for_frames(&session.test.window, 3);
    session.open_by_click("View");
    session.click_button("Search options");
    session.expect_only_open(Some("Search options"), "View open, a click on Search options");
    session.click_button("View");
    session.expect_only_open(Some("View"), "Search options open, a click on View");
    session.click_button("View");
    session.expect_only_open(None, "View open, a click on View");
}

/// In a narrow window, where the bar keeps fewer commands, See more and
/// the other menus open from each other the same way.
///
/// parity: CMD-005
#[gtk::test]
fn the_menus_of_a_narrow_window_open_from_each_other_with_one_click() {
    let mut session = Session::open(Some(672));
    assert!(session.test.window.width() <= 680, "a compact window");
    session.open_by_click("See more");
    let mut open = "See more";
    for next in ["New", "See more", "View", "See more", "Sort", "See more"] {
        session.click_button(next);
        session.expect_only_open(Some(next), &format!("{open} open, one click on {next}"));
        open = next;
    }
    session.click_button(open);
    session.expect_only_open(None, &format!("{open} open, a click on {open}"));
}

/// Clicks on Sort, View and See more in quick succession while New is
/// open leave See more's menu open, and nothing else; the pointer is free
/// afterwards, as a click on a file then selects it.
///
/// parity: CMD-005
#[gtk::test]
fn quick_clicks_on_several_menu_buttons_leave_the_last_menu_open() {
    let mut session = Session::open(None);
    session.open_by_click("New");
    let points: Vec<(i32, i32)> = ["Sort", "View", "See more"]
        .into_iter()
        .map(|name| session.middle_of(&menu_button(&session.test, name)))
        .collect();
    for point in points {
        session.click_at(point);
    }
    session.expect_only_open(Some("See more"), "quick clicks on Sort, View and See more");
    session.pointer.tap_key(gdk::Key::Escape);
    session.expect_only_open(None, "Escape");
    let file = super::right_drag::item_on_screen(&mut session.pointer, &session.test, "Notes 2.txt");
    session.click_at(file);
    wait_until("the click to select the file", || {
        session.test.selected_names() == ["Notes 2.txt"]
    });
}

/// As in Windows 11 File Explorer, a click on a file while a menu of the
/// bar is open closes the menu and selects the file; a click on the
/// bar's empty part closes it and opens no other.
///
/// parity: CMD-005
#[gtk::test]
fn a_click_elsewhere_closes_the_menu_and_still_acts_there() {
    let mut session = Session::open(None);
    assert!(session.test.selected_names().is_empty());
    session.open_by_click("View");
    let file = super::right_drag::item_on_screen(&mut session.pointer, &session.test, "Notes 2.txt");
    session.click_at(file);
    session.expect_only_open(None, "View open, a click on a file");
    wait_until("the click to select the file", || {
        session.test.selected_names() == ["Notes 2.txt"]
    });

    session.open_by_click("Sort");
    let bar = session.test.window.command_bar().clone();
    // Just past See more, before the commands at the right.
    let more = menu_button(&session.test, "See more")
        .compute_bounds(&bar)
        .expect("See more is laid out in the bar");
    let gap = (
        f64::from(more.x() + more.width()) + 30.0,
        f64::from(bar.height()) / 2.0,
    );
    let picked = bar.pick(gap.0, gap.1, gtk::PickFlags::DEFAULT);
    assert!(
        picked.is_none_or(|widget| widget.ancestor(gtk::Button::static_type()).is_none()),
        "the point is on no command"
    );
    let empty = session.on_screen(&bar, gap);
    session.click_at(empty);
    session.expect_only_open(None, "Sort open, a click on the bar's empty part");
    assert_eq!(
        session.test.selected_names(),
        ["Notes 2.txt"],
        "the selection stays"
    );

    // The address bar starts editing the address, as a click on it does.
    session.open_by_click("New");
    let address_bar = session.test.window.address_bar().clone();
    let on_address = session.on_screen(
        &address_bar,
        (
            f64::from(address_bar.width()) * 0.9,
            f64::from(address_bar.height()) / 2.0,
        ),
    );
    session.click_at(on_address);
    session.expect_only_open(None, "New open, a click on the address bar");
    let entry = address_bar.entry();
    wait_until("the address to be edited", || {
        entry.is_mapped() && has_keyboard(&session.test, &entry)
    });
    session.pointer.tap_key(gdk::Key::Escape);
    wait_until("the address edit to end", || !entry.is_mapped());

    // The navigation pane goes where it is clicked.
    session.open_by_click("See more");
    let sidebar = session.test.window.sidebar().clone();
    let index = sidebar
        .labels()
        .iter()
        .position(|label| label == "This PC")
        .expect("the navigation pane lists This PC");
    let row = sidebar
        .list()
        .row_at_index(i32::try_from(index).expect("a few rows"))
        .expect("This PC's row");
    let on_row = session.middle_of(&row);
    session.click_at(on_row);
    session.expect_only_open(None, "See more open, a click on This PC");
    wait_until("This PC to show", || {
        session.test.window.current_uri().as_deref() == Some(crate::locations::Page::ThisPc.uri())
    });
}

/// A click on an item runs it and closes the menu, opening no other.
///
/// parity: CMD-005
#[gtk::test]
fn a_click_on_an_item_runs_it_and_opens_no_other_menu() {
    let mut session = Session::open(None);
    let window = session.test.window.clone();
    let details_pane = move || -> bool {
        let action = window
            .lookup_action("details-pane")
            .expect("the window has Details pane");
        action
            .state()
            .and_then(|state| state.get::<bool>())
            .expect("Details pane is a toggle")
    };
    assert!(!details_pane(), "a new window has the details pane closed");
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    let row = row_of(&menu, "Details pane");
    let point = session.on_screen(&row, (24.0, f64::from(row.height()) / 2.0));
    session.click_at(point);
    session.expect_only_open(None, "a click on Details pane");
    wait_until("the details pane to open", details_pane.clone());

    // The pointer resting on a row while the window lays itself out
    // again, as when the folder is listed again: the click still goes to
    // the row, not to the file under the menu.
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    let row = row_of(&menu, "Details pane");
    let point = session.on_screen(&row, (24.0, f64::from(row.height()) / 2.0));
    session.pointer.move_to(point.0, point.1);
    wait_for(QUIET);
    session.test.window.folder_pane().queue_resize();
    wait_for_frames(&session.test.window, 3);
    wait_for(QUIET);
    session.pointer.press(Button::Primary);
    session.pointer.release(Button::Primary);
    session.expect_only_open(None, "a click on Details pane after a layout");
    wait_until("the details pane to close", || !details_pane());
    assert!(
        session.test.selected_names().is_empty(),
        "no file under the menu selected"
    );
}

/// Escape closes the menu a click opened and gives the keyboard back to
/// its button; the keyboard opens the menus with Enter and Space, moves
/// in them with the arrows and closes them with Escape.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn escape_and_the_keyboard_still_work_the_menus() {
    let mut session = Session::open(None);
    for name in MENUS {
        session.open_by_click(name);
        session.pointer.tap_key(gdk::Key::Escape);
        session.expect_only_open(None, &format!("Escape in {name}"));
        let button = menu_button(&session.test, name);
        wait_until(&format!("the keyboard back on {name}"), || {
            has_keyboard(&session.test, &button)
        });
    }
    for key in [gdk::Key::Return, gdk::Key::space] {
        let button = menu_button(&session.test, "Sort");
        button.grab_focus();
        session.pointer.tap_key(key);
        session.expect_only_open(Some("Sort"), &format!("{key:?} on Sort"));
        let menu = menu_of(&session.test, "Sort");
        wait_until("the keyboard in the menu", || has_keyboard(&session.test, &menu));
        let before = gtk::prelude::GtkWindowExt::focus(&session.test.window);
        session.pointer.tap_key(gdk::Key::Down);
        wait_until("Down to move in the menu", || {
            let now = gtk::prelude::GtkWindowExt::focus(&session.test.window);
            now != before && has_keyboard(&session.test, &menu)
        });
        session.pointer.tap_key(gdk::Key::Escape);
        session.expect_only_open(None, "Escape");
        wait_until("the keyboard back on Sort", || {
            has_keyboard(&session.test, &button)
        });
    }
}

/// In the dark theme and Compact view the menus open from each other the
/// same way, and a click on a plain command of the bar while a menu is
/// open closes the menu and runs the command, as every click reaches
/// what it is on.
///
/// parity: CMD-005
#[gtk::test]
fn the_menus_switch_the_same_in_the_dark_theme_and_compact_view() {
    let _theme = ThemeGuard::keep();
    let mut session = Session::open(None);
    session.test.activate("theme", Some("dark"));
    session.test.activate("compact-density", None);
    wait_for_frames(&session.test.window, 4);
    session.open_by_click("View");
    for next in ["Sort", "New", "See more", "View"] {
        session.click_button(next);
        session.expect_only_open(Some(next), &format!("one click on {next}"));
    }

    let window = session.test.window.clone();
    let details_pane = move || -> bool {
        let action = window
            .lookup_action("details-pane")
            .expect("the window has Details pane");
        action
            .state()
            .and_then(|state| state.get::<bool>())
            .expect("Details pane is a toggle")
    };
    assert!(!details_pane());
    let details = super::geometry::button_for(&session.test, "win.details-pane");
    let point = session.middle_of(&details);
    session.click_at(point);
    session.expect_only_open(None, "View open, a click on Details");
    wait_until("Details to open the details pane", details_pane);
    session.test.activate("compact-density", None);
}

/// Rests the pointer at `point` on an open menu while the window lays
/// itself out again, as when the folder is listed again. On X11, GTK's
/// pointer target is then the widget of the window under the menu, not
/// the menu, until the pointer moves.
fn rest_on_menu_through_a_layout(session: &mut Session, point: (i32, i32)) {
    session.pointer.move_to(point.0, point.1);
    wait_for(QUIET);
    session.test.window.folder_pane().queue_resize();
    wait_for_frames(&session.test.window, 3);
    wait_for(QUIET);
}

/// A window on a folder whose files fill the list, so that every point of
/// the list under a menu is on a file.
fn session_over_many_files() -> Session {
    let session = Session::open_on(Fixture::with_files(60), None);
    assert!(session.test.selected_names().is_empty());
    session
}

/// Whether the window toggle `name` (`details-pane`, `preview-pane`) is
/// on.
fn toggle_is_on(test: &TestWindow, name: &str) -> bool {
    let action = test
        .window
        .lookup_action(name)
        .expect("the window has the toggle");
    action
        .state()
        .and_then(|state| state.get::<bool>())
        .expect("a toggle")
}

/// A press and release on a part of the open menu that is no item, such
/// as the gap a divider leaves, never reaches the file under the menu,
/// also right after the window laid itself out with the pointer resting
/// there; the menu stays open, as such a click does nothing in a menu.
///
/// parity: CMD-005
#[gtk::test]
fn a_click_between_a_menus_items_never_reaches_the_file_under_it() {
    let mut session = session_over_many_files();
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    let row = row_of(&menu, "Details pane");
    let bounds = row
        .compute_bounds(&menu)
        .expect("the row is laid out in the menu");
    // In the divider's gap just above the row.
    let gap = (f64::from(bounds.x()) + 24.0, f64::from(bounds.y()) - 2.0);
    let picked = menu
        .pick(gap.0, gap.1, gtk::PickFlags::DEFAULT)
        .expect("the gap is in the menu");
    assert!(
        picked.ancestor(gtk::ListBoxRow::static_type()).is_none()
            && picked.ancestor(gtk::Button::static_type()).is_none(),
        "the gap is on no item: {:?}",
        picked.type_().name()
    );
    let point = session.on_screen(&menu, gap);

    rest_on_menu_through_a_layout(&mut session, point);
    session.pointer.press(Button::Primary);
    session.pointer.release(Button::Primary);
    wait_for(QUIET);

    assert!(
        session.test.selected_names().is_empty(),
        "no file under the menu selected"
    );
    assert_eq!(
        session.open_menus(),
        ["View"],
        "a click on no item leaves the menu open"
    );
    assert!(!toggle_is_on(&session.test, "details-pane"), "nothing ran");
}

/// After the window laid itself out with the pointer resting on an item,
/// a press there runs the item only when the primary button is released
/// on that same item: a press dragged to another item or out of the menu
/// before the release, or a right or middle click, runs nothing, as in
/// any menu, and reaches no file under the menu.
///
/// parity: CMD-005
#[gtk::test]
fn a_press_on_an_item_runs_it_only_if_released_there_with_the_primary_button() {
    let mut session = session_over_many_files();
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    let middle_of_row = |session: &Session, label: &str| {
        let row = row_of(&menu, label);
        session.on_screen(&row, (24.0, f64::from(row.height()) / 2.0))
    };
    let details = middle_of_row(&session, "Details pane");
    let preview = middle_of_row(&session, "Preview pane");

    // Dragged to Preview pane before the release.
    rest_on_menu_through_a_layout(&mut session, details);
    session.pointer.press(Button::Primary);
    session.pointer.glide(details, preview, 6);
    session.pointer.release(Button::Primary);
    wait_for(QUIET);
    assert!(
        !toggle_is_on(&session.test, "details-pane"),
        "the item pressed did not run"
    );
    assert!(
        !toggle_is_on(&session.test, "preview-pane"),
        "nor the item released on"
    );

    // Dragged out of the menu, onto the files, before the release.
    let row = row_of(&menu, "Details pane");
    let row_bounds = row.compute_bounds(&menu).expect("the row is in the menu");
    let row_y = f64::from(row_bounds.y() + row_bounds.height() / 2.0);
    let outside = session.on_screen(&menu, (f64::from(menu.width()) + 80.0, row_y));
    rest_on_menu_through_a_layout(&mut session, details);
    session.pointer.press(Button::Primary);
    session.pointer.glide(details, outside, 8);
    session.pointer.release(Button::Primary);
    wait_for(QUIET);
    assert!(
        !toggle_is_on(&session.test, "details-pane"),
        "a press dragged off the menu ran nothing"
    );

    // A right click and a middle click.
    for button in [Button::Secondary, Button::Middle] {
        if session.open_menus().is_empty() {
            session.open_by_click("View");
        }
        rest_on_menu_through_a_layout(&mut session, details);
        session.pointer.press(button);
        session.pointer.release(button);
        wait_for(QUIET);
        assert!(
            !toggle_is_on(&session.test, "details-pane"),
            "a {button:?} click ran nothing"
        );
    }
    assert!(
        session.test.selected_names().is_empty(),
        "no file under the menu selected"
    );

    // A primary click released on the item pressed still runs it.
    if session.open_menus().is_empty() {
        session.open_by_click("View");
    }
    rest_on_menu_through_a_layout(&mut session, details);
    session.pointer.press(Button::Primary);
    session.pointer.release(Button::Primary);
    session.expect_only_open(None, "a click on Details pane");
    wait_until("the details pane to open", || {
        toggle_is_on(&session.test, "details-pane")
    });
}

/// The file list's scrolled window: the one in the folder pane that can
/// scroll down.
fn file_list_scroller(test: &TestWindow) -> gtk::ScrolledWindow {
    descendants::<gtk::ScrolledWindow>(test.window.folder_pane())
        .into_iter()
        .find(|scroller| {
            let adjustment = scroller.vadjustment();
            scroller.is_mapped() && adjustment.upper() > adjustment.page_size() + 1.0
        })
        .expect("the file list scrolls")
}

/// The wheel turned over an open menu, also right after the window laid
/// itself out with the pointer resting on the menu, never scrolls the
/// file list under the menu.
///
/// parity: CMD-005
#[gtk::test]
fn the_wheel_over_an_open_menu_never_scrolls_the_files_under_it() {
    let mut session = session_over_many_files();
    let scroller = file_list_scroller(&session.test);
    let top = scroller.vadjustment().value();
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    let row = row_of(&menu, "Details pane");
    let point = session.on_screen(&row, (24.0, f64::from(row.height()) / 2.0));

    rest_on_menu_through_a_layout(&mut session, point);
    for _ in 0..3 {
        session.pointer.press(Button::WheelDown);
        session.pointer.release(Button::WheelDown);
    }
    wait_for(QUIET);

    assert!(
        (scroller.vadjustment().value() - top).abs() < f64::EPSILON,
        "the file list did not scroll"
    );
    assert_eq!(session.open_menus(), ["View"], "the wheel leaves the menu open");
}

/// A right or middle click on the open menu's own button closes it, as a
/// click elsewhere does: only a primary click toggles the button.
///
/// parity: CMD-005
#[gtk::test]
fn a_right_or_middle_click_on_the_open_menus_button_closes_it() {
    let mut session = Session::open(None);
    for button in [Button::Secondary, Button::Middle] {
        session.open_by_click("View");
        let point = session.middle_of(&menu_button(&session.test, "View"));
        session.pointer.move_to(point.0, point.1);
        session.pointer.press(button);
        session.pointer.release(button);
        session.expect_only_open(None, &format!("View open, a {button:?} click on View"));
    }
}

/// Tab, Shift+Tab, F6 and Shift+F6 never leave an open menu on screen
/// while the keyboard is outside it: the menu keeps the keyboard, or it
/// closes.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn the_keyboard_never_leaves_an_open_menu_behind() {
    let mut session = Session::open(None);
    for (shift, key) in [
        (false, gdk::Key::Tab),
        (true, gdk::Key::Tab),
        (false, gdk::Key::F6),
        (true, gdk::Key::F6),
    ] {
        session.open_by_click("View");
        let menu = menu_of(&session.test, "View");
        wait_until("the keyboard in the menu", || has_keyboard(&session.test, &menu));
        if shift {
            session.pointer.hold_key(gdk::Key::Shift_L);
        }
        session.pointer.tap_key(key);
        if shift {
            session.pointer.let_go_key(gdk::Key::Shift_L);
        }
        wait_for(QUIET);
        let open = !session.open_menus().is_empty();
        assert!(
            !open || has_keyboard(&session.test, &menu),
            "{key:?} (Shift {shift}): View is open while the keyboard is on {:?}",
            gtk::prelude::GtkWindowExt::focus(&session.test.window).map(|focus| focus.type_().name())
        );
        for name in MENUS {
            menu_button(&session.test, name).popdown();
        }
        wait_for(QUIET);
    }
}

/// With real keys: Right on Show opens its submenu with the keyboard in
/// it, Escape closes only the submenu and gives the keyboard back to the
/// View menu, and a second Escape closes View and gives it back to its
/// button.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn escape_closes_a_submenu_then_its_menu() {
    let mut session = Session::open(None);
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    wait_until("the keyboard in the menu", || has_keyboard(&session.test, &menu));
    let show = row_of(&menu, "Show");
    for _ in 0..30 {
        if has_keyboard(&session.test, &show) {
            break;
        }
        session.pointer.tap_key(gdk::Key::Down);
    }
    assert!(has_keyboard(&session.test, &show), "Down reaches Show");
    session.pointer.tap_key(gdk::Key::Right);
    wait_until("Show's submenu", || {
        menu.open_submenu_menu()
            .is_some_and(|submenu| submenu.is_mapped())
    });
    let submenu = menu.open_submenu_menu().expect("Show's submenu");
    wait_until("the keyboard in the submenu", || {
        has_keyboard(&session.test, &submenu)
    });

    session.pointer.tap_key(gdk::Key::Escape);
    wait_until("the submenu to close", || !submenu.is_mapped());
    wait_for(QUIET);
    assert_eq!(session.open_menus(), ["View"], "View stays open");
    assert!(has_keyboard(&session.test, &menu), "the keyboard is back in View");

    session.pointer.tap_key(gdk::Key::Escape);
    session.expect_only_open(None, "a second Escape");
    let button = menu_button(&session.test, "View");
    wait_until("the keyboard back on View", || {
        has_keyboard(&session.test, &button)
    });
}

/// The open menu closes when the window stops being the active one, as
/// when another app's window comes in front.
///
/// parity: CMD-005
#[gtk::test]
fn the_open_menu_closes_when_another_window_comes_in_front() {
    let mut session = Session::open(None);
    session.open_by_click("Sort");
    let other = gtk::Window::builder()
        .title("Another app")
        .default_width(300)
        .default_height(200)
        .build();
    other.present();
    wait_until("the window to stop being active", || {
        !session.test.window.is_active()
    });
    session.expect_only_open(None, "another window in front");
    crate::test_support::harness::put_away(&other);
}

/// The open menu closes when the window shows another tab or location,
/// as when a key goes back, up or to the next tab while it is open.
///
/// parity: CMD-005
#[gtk::test]
fn the_open_menu_closes_when_the_tab_or_location_changes() {
    let mut session = Session::open(None);
    session.test.activate("new-tab", None);
    wait_until("two tabs", || {
        session.test.window.imp().session.borrow().tabs().len() == 2
    });
    wait_for_frames(&session.test.window, 3);
    session.open_by_click("View");
    session.test.activate("next-tab", None);
    session.expect_only_open(None, "View open, the window shows the next tab");

    // A subfolder of the test's own folder, so nothing outside it is
    // listed.
    session.test.activate("previous-tab", None);
    wait_for_frames(&session.test.window, 3);
    let documents = session.fixture.uri_of("Documents");
    session.open_by_click("View");
    session.test.activate("go-to", Some(&documents));
    session.expect_only_open(None, "View open, the window shows another folder");
}

/// A primary press on the open menu's own button dragged off the button
/// before the release cancels the button's click: the menu is then not
/// left on screen while the keyboard is outside it, as the press moved
/// the keyboard to the button.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn a_press_on_the_open_menus_button_dragged_off_never_strands_the_menu() {
    let mut session = Session::open(None);
    session.open_by_click("View");
    let menu = menu_of(&session.test, "View");
    wait_until("the keyboard in the menu", || has_keyboard(&session.test, &menu));
    let on_button = session.middle_of(&menu_button(&session.test, "View"));
    let bar = session.test.window.command_bar().clone();
    let off_button = session.on_screen(
        &bar,
        (f64::from(bar.width()) - 300.0, f64::from(bar.height()) / 2.0),
    );

    session.pointer.move_to(on_button.0, on_button.1);
    session.pointer.press(Button::Primary);
    session.pointer.glide(on_button, off_button, 10);
    session.pointer.release(Button::Primary);
    wait_for(QUIET);

    let open = !session.open_menus().is_empty();
    assert!(
        !open || has_keyboard(&session.test, &menu),
        "View is open while the keyboard is on {:?}",
        gtk::prelude::GtkWindowExt::focus(&session.test.window).map(|focus| focus.type_().name())
    );
}

/// Entries for the New menu: `count` items, as many templates give.
fn many_new_entries(count: usize) -> Vec<crate::window::menu_popover::MenuEntry> {
    use crate::window::menu_popover::MenuItem;
    (0..count)
        .map(|number| {
            MenuItem::new(
                &format!("Template {number:02}"),
                crate::icons::Icon::Document,
                crate::window::window_action::WindowAction::NewFolder,
            )
            .into()
        })
        .collect()
}

/// The New menu rebuilt while it is open, as when the templates are read
/// again just after it opened, keeps the keyboard in the menu: the arrows
/// still move in it.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn the_new_menu_keeps_the_keyboard_when_its_items_change_while_open() {
    let mut session = Session::open(None);
    session.open_by_click("New");
    let menu = menu_of(&session.test, "New");
    wait_until("the keyboard in the menu", || has_keyboard(&session.test, &menu));

    menu.set_entries(many_new_entries(4));
    assert!(
        has_keyboard(&session.test, &menu),
        "the keyboard is in the rebuilt menu at once, not on a row it removed"
    );
    wait_for_frames(&menu, 3);

    assert_eq!(session.open_menus(), ["New"]);
    assert!(has_keyboard(&session.test, &menu), "and stays there");
    let before = gtk::prelude::GtkWindowExt::focus(&session.test.window);
    session.pointer.tap_key(gdk::Key::Down);
    wait_until("Down to move in the menu", || {
        let now = gtk::prelude::GtkWindowExt::focus(&session.test.window);
        now != before && has_keyboard(&session.test, &menu)
    });
}

/// The wheel over a menu too tall for the screen scrolls the menu, also
/// right after the window laid itself out with the pointer resting on
/// it, and never the file list under it.
///
/// parity: CMD-005
#[gtk::test]
fn the_wheel_scrolls_a_tall_menu_after_a_layout() {
    let mut session = session_over_many_files();
    let files = file_list_scroller(&session.test);
    let files_top = files.vadjustment().value();
    let menu = menu_of(&session.test, "New");
    menu.set_entries(many_new_entries(80));
    session.open_by_click("New");
    let rows = descendants::<gtk::ScrolledWindow>(&menu)
        .into_iter()
        .next()
        .expect("the menu's rows scroll");
    let adjustment = rows.vadjustment();
    assert!(
        adjustment.upper() > adjustment.page_size() + 1.0,
        "the menu is taller than the screen"
    );
    let top = adjustment.value();
    let row = row_of(&menu, "Template 03");
    let point = session.on_screen(&row, (24.0, f64::from(row.height()) / 2.0));

    rest_on_menu_through_a_layout(&mut session, point);
    for _ in 0..3 {
        session.pointer.press(Button::WheelDown);
        session.pointer.release(Button::WheelDown);
    }
    wait_for(QUIET);

    assert!(adjustment.value() > top + 1.0, "the menu scrolled");
    assert!(
        (files.vadjustment().value() - files_top).abs() < f64::EPSILON,
        "the file list did not scroll"
    );
    assert_eq!(session.open_menus(), ["New"]);
}
