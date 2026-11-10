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

use super::command_bar::{command_bar_controls, control_name};
use super::narrow_windows::resize;
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
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
    _fixture: Fixture,
}

impl Session {
    /// A window on a folder with a few files, wide or `width` pixels.
    fn open(width: Option<i32>) -> Self {
        let mut pointer = Pointer::connect();
        let fixture = Fixture::standard();
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
            _fixture: fixture,
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
    for (first, second) in [
        ("See more", "New"),
        ("New", "See more"),
        ("View", "See more"),
        ("See more", "Sort"),
    ] {
        session.open_by_click(first);
        session.click_button(second);
        session.expect_only_open(Some(second), &format!("{first} open, one click on {second}"));
    }
    session.click_button("Sort");
    session.expect_only_open(None, "Sort open, a click on Sort");
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
    let empty = session.on_screen(
        &bar,
        (f64::from(bar.width()) * 0.62, f64::from(bar.height()) / 2.0),
    );
    let picked = bar.pick(
        f64::from(bar.width()) * 0.62,
        f64::from(bar.height()) / 2.0,
        gtk::PickFlags::DEFAULT,
    );
    assert!(
        picked.is_none_or(|widget| widget.ancestor(gtk::Button::static_type()).is_none()),
        "the point is on no command"
    );
    session.click_at(empty);
    session.expect_only_open(None, "Sort open, a click on the bar's empty part");
    assert_eq!(
        session.test.selected_names(),
        ["Notes 2.txt"],
        "the selection stays"
    );
}

/// A click on an item runs it and closes the menu, opening no other.
///
/// parity: CMD-005
#[gtk::test]
fn a_click_on_an_item_runs_it_and_opens_no_other_menu() {
    let mut session = Session::open(None);
    let window = session.test.window.clone();
    let details_pane = move || {
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
    wait_until("the details pane to open", details_pane);
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
