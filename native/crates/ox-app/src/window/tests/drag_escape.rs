// SPDX-License-Identifier: AGPL-3.0-only
//! Escape while a file is dragged with the button still held calls the
//! drag off, so releasing over a folder afterwards does nothing, as in
//! Windows 11 File Explorer (DND-029). These drag with check.py's private
//! display's real pointer and keyboard, so GTK's own drag source and drop
//! targets run.

use gtk::gdk;
use gtk::subclass::prelude::*;

use super::right_drag::{item_on_screen, HOLD};
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{Button, Pointer};
use crate::window::file_drag::shown_drag_label;

/// Presses on `from`, drags to `onto` until the drag icon says `label`,
/// taps Escape there with the button still held, and releases over
/// `onto`.
fn drag_escape_and_release(
    pointer: &mut Pointer,
    test: &TestWindow,
    from: (i32, i32),
    onto: (i32, i32),
    label: &str,
) {
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    wait_until(&format!("the label {label:?}"), || {
        shown_drag_label().as_deref() == Some(label)
    });
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

/// Drags `name` from `from` onto `onto` and releases, without Escape.
fn drag_and_release(
    pointer: &mut Pointer,
    test: &TestWindow,
    from: (i32, i32),
    onto: (i32, i32),
    label: &str,
) {
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    wait_until(&format!("the label {label:?}"), || {
        shown_drag_label().as_deref() == Some(label)
    });
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

    drag_escape_and_release(&mut pointer, &test, from, onto, "Move to Documents");

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
    drag_and_release(&mut pointer, &test, from, onto, "Move to Documents");
    wait_until("the file to be moved", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
}
