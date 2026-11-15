// SPDX-License-Identifier: AGPL-3.0-only
//! No tooltip stays over a menu, the rename box or the next folder, as in
//! Windows Explorer: a tooltip that shows goes once a menu opens, from the
//! keyboard or a right-click, once a name is renamed in place, and once
//! the tab goes to another folder. The pointer and keys are real ones on
//! check.py's private display, so GTK shows and hides its own tooltips.

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{is_renaming_in_place, name_editor, select_names};
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{settle, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::widget_tree::descendants;

/// The text of the tooltip `test`'s window shows now, if any.
fn shown_tooltip(test: &TestWindow) -> Option<String> {
    descendants::<gtk::Widget>(&test.window)
        .into_iter()
        .find(|widget| widget.type_().name() == "GtkTooltipWindow" && widget.is_visible())
        .map(|tooltip| {
            descendants::<gtk::Label>(&tooltip)
                .iter()
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        })
}

/// Moves the pointer onto the middle of `widget`, past its edge first so
/// GTK sees it arrive, and waits for its tooltip.
fn hover(test: &TestWindow, pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>, what: &str) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    let (x, y) = screen_point(pointer, widget, middle, (200, 200));
    pointer.move_to(x - 3, y - 1);
    pointer.move_to(x, y);
    wait_until(&format!("the tooltip of {what}"), || {
        shown_tooltip(test).is_some()
    });
}

/// The cell of the item called `name` in the view.
fn cell_of(test: &TestWindow, name: &str) -> FileCell {
    descendants::<FileCell>(&test.window.folder_pane().view_widget())
        .into_iter()
        .find(|cell| {
            cell.is_drawable()
                && descendants::<gtk::Label>(cell)
                    .iter()
                    .any(|label| label.text() == name)
        })
        .unwrap_or_else(|| panic!("the view shows {name}"))
}

/// The Details view's column title that names `column`. Column titles
/// have no tooltip of their own, as in Windows 11 (VIEW-029).
fn column_title(test: &TestWindow, column: &str) -> gtk::Widget {
    descendants::<gtk::Widget>(&test.window.folder_pane().view_widget())
        .into_iter()
        .find(|widget| {
            widget.is_drawable()
                && widget.type_().name() == "GtkColumnViewTitle"
                && descendants::<gtk::Label>(widget)
                    .iter()
                    .any(|label| label.text() == column)
        })
        .unwrap_or_else(|| panic!("the Details view has a {column} column"))
}

/// Presses `key` with Shift held.
fn shift_tap(pointer: &mut Pointer, key: gdk::Key) {
    pointer.hold_key(gdk::Key::Shift_L);
    pointer.tap_key(key);
    pointer.let_go_key(gdk::Key::Shift_L);
}

/// How a file's menu is opened.
#[derive(Debug, Clone, Copy)]
enum MenuWay {
    ShiftF10,
    MenuKey,
    RightClick,
}

impl MenuWay {
    /// Opens the menu of the item under the pointer, or selected, this
    /// way.
    fn open(self, pointer: &mut Pointer) {
        match self {
            MenuWay::ShiftF10 => shift_tap(pointer, gdk::Key::F10),
            MenuWay::MenuKey => pointer.tap_key(gdk::Key::Menu),
            MenuWay::RightClick => {
                pointer.press(Button::Secondary);
                pointer.release(Button::Secondary);
            }
        }
    }
}

/// A tooltip shown over an item goes as Shift+F10, the Menu key or a
/// right-click opens a menu, as F2 renames the item in place and as the
/// tab opens another folder; a column title's goes as its right-click
/// menu opens.
///
/// parity: VIEW-073
#[gtk::test]
fn no_tooltip_stays_over_a_menu_the_rename_box_or_the_next_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    super::support::show_file_name_extensions(&test);
    let mut pointer = Pointer::connect();
    let menu_shows = |test: &TestWindow| test.window.context_menu().is_visible();

    for way in [MenuWay::ShiftF10, MenuWay::MenuKey, MenuWay::RightClick] {
        select_names(&test, &["Notes 2.txt"]);
        test.window.folder_pane().focus_view();
        hover(&test, &mut pointer, &cell_of(&test, "Notes 2.txt"), "Notes 2.txt");
        way.open(&mut pointer);
        wait_until(&format!("the menu {way:?} opens"), || menu_shows(&test));
        wait_until(&format!("no tooltip over the menu {way:?} opened"), || {
            shown_tooltip(&test).is_none()
        });
        pointer.tap_key(gdk::Key::Escape);
        wait_until("the menu to close", || !menu_shows(&test));
    }

    // The title has no tooltip to wait for (VIEW-029): the pointer only
    // has to be on it.
    let title = column_title(&test, "Name");
    let middle = (f64::from(title.width()) / 2.0, f64::from(title.height()) / 2.0);
    let (x, y) = screen_point(&mut pointer, &title, middle, (200, 200));
    pointer.move_to(x, y);
    settle();
    pointer.press(Button::Secondary);
    pointer.release(Button::Secondary);
    wait_until("no tooltip over the column menu", || {
        shown_tooltip(&test).is_none()
    });
    pointer.tap_key(gdk::Key::Escape);

    select_names(&test, &["Notes 2.txt"]);
    test.window.folder_pane().focus_view();
    hover(&test, &mut pointer, &cell_of(&test, "Notes 2.txt"), "Notes 2.txt");
    pointer.tap_key(gdk::Key::F2);
    let _field = name_editor(&test);
    wait_until("no tooltip over the rename box", || {
        shown_tooltip(&test).is_none()
    });
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(&test));

    hover(&test, &mut pointer, &cell_of(&test, "Notes 2.txt"), "Notes 2.txt");
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("Documents");
    wait_until("no tooltip over the next folder", || {
        shown_tooltip(&test).is_none()
    });
}
