// SPDX-License-Identifier: AGPL-3.0-only
//! Escape while a file is dragged with the button still held calls the
//! drag off, so releasing over a folder afterwards does nothing, as in
//! Windows 11 File Explorer (DND-029). These drag with check.py's private
//! display's real pointer and keyboard, so GTK's own drag source and drop
//! targets run.

use gtk::gdk;
use gtk::subclass::prelude::*;

use gtk::prelude::*;

use super::right_drag::{item_on_screen, HOLD, PROBE};
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{Button, Pointer};
use crate::window::file_drag::shown_drag_label;

/// Presses on `from`, drags to `onto` until the drag icon says `label`
/// (or, for `None`, a moment), taps Escape there with the button still
/// held, and releases over `onto`.
fn drag_escape_and_release(
    pointer: &mut Pointer,
    test: &TestWindow,
    from: (i32, i32),
    onto: (i32, i32),
    label: Option<&str>,
) {
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    match label {
        Some(label) => wait_until(&format!("the label {label:?}"), || {
            shown_drag_label().as_deref() == Some(label)
        }),
        None => wait_for(HOLD),
    }
    pointer.tap_key(gdk::Key::Escape);
    wait_for(HOLD);
    pointer.glide(onto, (onto.0 + 3, onto.1), 2);
    wait_for(HOLD);
    pointer.release(Button::Primary);
    wait_until("the drag to end", || {
        test.window.imp().outgoing_drag.borrow().is_none()
    });
    wait_for(HOLD);
}

/// Drags from `from` onto `onto` and releases, without Escape, once the
/// drag icon says `label` (or, for `None`, after a moment).
fn drag_and_release(
    pointer: &mut Pointer,
    test: &TestWindow,
    from: (i32, i32),
    onto: (i32, i32),
    label: Option<&str>,
) {
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    match label {
        Some(label) => wait_until(&format!("the label {label:?}"), || {
            shown_drag_label().as_deref() == Some(label)
        }),
        None => wait_for(HOLD),
    }
    pointer.release(Button::Primary);
    wait_until("the drag to end", || {
        test.window.imp().outgoing_drag.borrow().is_none()
    });
}

/// Escape during a real drag of a file onto a folder of the same list
/// calls the drag off: the release over the folder moves nothing. A drag
/// after that still moves the file.
///
/// parity: DND-029
#[gtk::test]
fn escape_during_a_drag_onto_a_folder_moves_nothing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = item_on_screen(&mut pointer, &test, "Documents");

    drag_escape_and_release(&mut pointer, &test, from, onto, Some("Move to Documents"));

    assert!(
        fixture.path("Notes 2.txt").is_file(),
        "Escape called the drag off: the file stayed"
    );
    assert!(
        !fixture.path("Documents/Notes 2.txt").exists(),
        "nothing was moved"
    );

    // A normal drag still works afterwards.
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = item_on_screen(&mut pointer, &test, "Documents");
    drag_and_release(&mut pointer, &test, from, onto, Some("Move to Documents"));
    wait_until("the file to be moved", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
}

/// The screen point of the middle of `widget`.
fn middle_on_screen(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    crate::test_support::pointer::screen_point(pointer, widget, middle, PROBE)
}

/// Escape during a real drag of a file onto a folder pinned in the
/// navigation pane calls the drag off: the release over the row moves
/// nothing.
///
/// parity: DND-029
#[gtk::test]
fn escape_during_a_drag_onto_a_navigation_pane_row_moves_nothing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.activate("pin-folder", None);
    wait_until("the pin", || {
        test.window.sidebar().labels().contains(&"Documents".to_owned())
    });
    test.window.navigate(&fixture.uri()).expect("the fixture");
    test.wait_for_listing("the fixture");
    wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let sidebar = test.window.sidebar();
    let list = sidebar.list().clone();
    let row_y = sidebar.middle_of("Documents");
    let onto = crate::test_support::pointer::screen_point(&mut pointer, &list, (60.0, row_y), PROBE);

    drag_escape_and_release(&mut pointer, &test, from, onto, Some("Move to Documents"));

    assert!(fixture.path("Notes 2.txt").is_file(), "the file stayed");
    assert!(
        !fixture.path("Documents/Notes 2.txt").exists(),
        "nothing was moved"
    );
    pointer.move_to(720, 500);
}

/// Escape during a real drag of a file onto another tab leaves the file
/// where it was after the release over the tab. (On check.py's display a
/// real drag shows no label over a tab, and a drop there without Escape
/// moves nothing either, so this guards the outcome only.)
///
/// parity: DND-029
#[gtk::test]
fn escape_during_a_drag_onto_a_tab_moves_nothing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    test.activate("previous-tab", None);
    wait_until("the first tab in front", || {
        test.window.current_uri().as_deref() == Some(fixture.uri().as_str())
    });
    test.wait_for_listing("the fixture");
    wait_for_frames(&test.window, 3);
    let first = test
        .window
        .tab_strip()
        .tab_list()
        .first_child()
        .expect("the first tab");
    let second = first.next_sibling().expect("the second tab");
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = middle_on_screen(&mut pointer, &second);

    drag_escape_and_release(&mut pointer, &test, from, onto, None);

    assert!(fixture.path("Notes 2.txt").is_file(), "the file stayed");
    assert!(
        !fixture.path("Documents/Notes 2.txt").exists(),
        "nothing was moved"
    );
    assert_eq!(test.window.tab_count(), 2, "no tab opened or closed");
    pointer.move_to(720, 500);
}

/// Escape during a real drag of a file onto another window of the app
/// calls the drag off: the release over that window's folder moves
/// nothing. A normal drag there still moves the file afterwards.
///
/// parity: DND-029
#[gtk::test]
fn escape_during_a_drag_onto_another_window_moves_nothing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    wait_for_frames(&other.window, 3);
    // The new window covers the first: drag it down by its blank title
    // bar, so the first window's items show above it.
    let blank =
        crate::test_support::harness::descendants::<gtk::Box>(&other.window.titlebar().expect("a title bar"))
            .into_iter()
            .find(|child| child.has_css_class("title-drag"))
            .expect("the bar has its blank drag area");
    let grab = middle_on_screen(&mut pointer, &blank);
    pointer.move_to(grab.0, grab.1);
    pointer.press(Button::Primary);
    pointer.glide(grab, (grab.0, grab.1 + 420), 20);
    wait_for(HOLD);
    pointer.release(Button::Primary);
    wait_for(HOLD);
    wait_for_frames(&other.window, 3);

    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let other_view = other.window.folder_pane().input_widget();
    let onto = crate::test_support::pointer::screen_point(&mut pointer, &other_view, (80.0, 60.0), PROBE);
    assert!(onto.1 > from.1 + 100, "the other window lies below the item");

    drag_escape_and_release(&mut pointer, &test, from, onto, Some("Move to Documents"));

    assert!(fixture.path("Notes 2.txt").is_file(), "the file stayed");
    assert!(
        !fixture.path("Documents/Notes 2.txt").exists(),
        "nothing was moved"
    );

    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    drag_and_release(&mut pointer, &test, from, onto, Some("Move to Documents"));
    wait_until("the file to be moved", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    pointer.move_to(720, 500);
}
