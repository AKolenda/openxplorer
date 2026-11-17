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
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
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

/// The point `at` of `widget` on the screen, and in the window's own
/// coordinates, where its pointer motion reports it.
fn points_of(
    test: &TestWindow,
    pointer: &mut Pointer,
    widget: &impl IsA<gtk::Widget>,
    at: (f64, f64),
) -> ((i32, i32), (f64, f64)) {
    let screen = screen_point(pointer, widget, at, (200, 200));
    #[expect(clippy::cast_possible_truncation, reason = "points in a window are small")]
    let local = gtk::graphene::Point::new(at.0 as f32, at.1 as f32);
    let in_window = widget
        .compute_point(&test.window, &local)
        .expect("the widget is in the window");
    (screen, (f64::from(in_window.x()), f64::from(in_window.y())))
}

/// Moves the real pointer to `screen` and waits until the window has seen
/// it at `in_window`: GTK merges quick moves, so each move is waited for
/// rather than timed.
fn move_and_wait(test: &TestWindow, pointer: &mut Pointer, screen: (i32, i32), in_window: (f64, f64)) {
    pointer.move_to(screen.0, screen.1);
    wait_until("the window to see the pointer", || {
        test.window
            .pointer_seen_at()
            .is_some_and(|(x, y)| (x - in_window.0).abs() <= 2.0 && (y - in_window.1).abs() <= 2.0)
    });
}

/// How many times GTK is asked to look under the pointer for a tooltip.
const TOOLTIP_TRIES: u32 = 6;

/// Runs the main loop until `condition` holds, for at most two seconds,
/// which covers GTK's tooltip delay; whether it held.
fn wait_briefly(condition: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if condition() {
            return true;
        }
        wait_for(std::time::Duration::from_millis(20));
    }
    condition()
}

/// Moves the pointer onto the middle of `widget` and waits for its
/// tooltip. The pointer first rests on the empty part of the file list,
/// where nothing has a tooltip, until no tooltip shows and tooltips may
/// show again; so neither where an earlier test left the pointer nor a
/// tooltip hidden by the last key counts.
fn hover(test: &TestWindow, pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>, what: &str) {
    let view = test.window.folder_pane().view_widget();
    let empty = (f64::from(view.width()) / 2.0, f64::from(view.height()) - 24.0);
    let (screen, in_window) = points_of(test, pointer, &view, empty);
    move_and_wait(test, pointer, screen, in_window);
    wait_until("no tooltip, and tooltips allowed", || {
        shown_tooltip(test).is_none() && test.window.tooltips_may_show()
    });
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    let (screen, in_window) = points_of(test, pointer, widget, middle);
    move_and_wait(test, pointer, screen, in_window);
    // A second move inside the widget, as a hand that comes to rest.
    let (screen, in_window) = points_of(test, pointer, widget, (middle.0 + 2.0, middle.1 + 1.0));
    move_and_wait(test, pointer, screen, in_window);
    // GTK starts a tooltip's wait on a motion it handles, and a busy
    // display can merge or drop it: ask GTK to look under the resting
    // pointer again, as another motion would, until the tooltip shows.
    for _ in 0..TOOLTIP_TRIES {
        widget.trigger_tooltip_query();
        if wait_briefly(|| shown_tooltip(test).is_some()) {
            return;
        }
    }
    panic!("the tooltip of {what} did not show");
}

/// Whether no menu or other popover of the window is open.
fn no_menu_open(test: &TestWindow) -> bool {
    descendants::<gtk::Popover>(&test.window)
        .iter()
        .all(|popover| !popover.is_visible())
}

/// Makes sure real keys reach the window: on a display without a window
/// manager, the keyboard can be left on the root window once an earlier
/// test's window closed, and then no key arrives.
fn keep_keyboard_in(test: &TestWindow) {
    if !test.window.is_active() {
        test.window.present();
    }
    wait_until("the window to have the keyboard", || test.window.is_active());
}

/// The first drawn widget of type `T` in the view that `matches`, once
/// the view has drawn one: the view may still be laying out its rows.
fn drawn_in_view<T: IsA<gtk::Widget>>(test: &TestWindow, what: &str, matches: impl Fn(&T) -> bool) -> T {
    let find = || {
        descendants::<T>(&test.window.folder_pane().view_widget())
            .into_iter()
            .find(|widget| widget.is_drawable() && widget.width() > 0 && matches(widget))
    };
    wait_until(what, || find().is_some());
    find().expect("wait_until returned only once it was drawn")
}

/// The cell of the item called `name` in the view.
fn cell_of(test: &TestWindow, name: &str) -> FileCell {
    drawn_in_view(test, &format!("the view to show {name}"), |cell: &FileCell| {
        descendants::<gtk::Label>(cell)
            .iter()
            .any(|label| label.text() == name)
    })
}

/// The Details view's column title that names `column`.
fn column_title(test: &TestWindow, column: &str) -> gtk::Widget {
    drawn_in_view(test, &format!("the {column} column"), |widget: &gtk::Widget| {
        widget
            .tooltip_text()
            .is_some_and(|text| text.starts_with("Drag to resize"))
            && descendants::<gtk::Label>(widget)
                .iter()
                .any(|label| label.text() == column)
    })
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
        keep_keyboard_in(&test);
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

    keep_keyboard_in(&test);
    hover(
        &test,
        &mut pointer,
        &column_title(&test, "Name"),
        "the Name column",
    );
    pointer.press(Button::Secondary);
    pointer.release(Button::Secondary);
    wait_until("no tooltip over the column menu", || {
        shown_tooltip(&test).is_none()
    });
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the column menu to close", || no_menu_open(&test));

    keep_keyboard_in(&test);
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
