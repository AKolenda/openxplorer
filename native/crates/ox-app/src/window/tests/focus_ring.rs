// SPDX-License-Identifier: AGPL-3.0-only
//! Where the keyboard is shows only after keyboard navigation, as in
//! Windows 11 File Explorer: no focus ring at startup, none after a click,
//! none after a shortcut such as Ctrl+C, and a ring once Tab, F6 or an
//! arrow moves the keyboard. These tests press real keys and the pointer
//! with the X server's XTEST extension, so they run only in check.py's
//! isolated session.

use gtk::gdk;
use gtk::prelude::*;

use crate::test_support::harness::{descendants, wait_for, wait_for_frames, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};

/// How long GTK gets to handle a key or a click from the X server.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Lets the window handle what the X server sent.
fn settle(test: &TestWindow) {
    wait_for(SETTLE);
    wait_for_frames(&test.window, 2);
}

/// Clicks the middle of `widget` with the primary button.
fn click(test: &TestWindow, pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    let (x, y) = screen_point(pointer, widget, middle, (720, 500));
    pointer.move_to(x, y);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    settle(test);
}

/// Presses `key` with Ctrl held.
fn tap_with_control(test: &TestWindow, pointer: &mut Pointer, key: gdk::Key) {
    pointer.hold_key(gdk::Key::Control_L);
    pointer.tap_key(key);
    pointer.let_go_key(gdk::Key::Control_L);
    settle(test);
}

/// The navigation pane's Home row.
fn home_row(test: &TestWindow) -> gtk::ListBoxRow {
    descendants::<gtk::ListBoxRow>(test.window.sidebar().list())
        .into_iter()
        .find(|row| {
            descendants::<gtk::Label>(row)
                .iter()
                .any(|label| label.text() == "Home")
        })
        .expect("the navigation pane has a Home row")
}

/// parity: ACC-009
#[gtk::test]
fn the_focus_ring_shows_only_after_keyboard_navigation() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.present();
    settle(&test);
    let window = test.window.upcast_ref::<gtk::Window>();
    let mut seen = vec![("startup", window.gets_focus_visible())];
    let mut pointer = Pointer::connect();

    click(&test, &mut pointer, &home_row(&test));
    test.wait_for_listing("Home");
    seen.push(("a click on Home", window.gets_focus_visible()));
    tap_with_control(&test, &mut pointer, gdk::Key::c);
    seen.push(("Ctrl+C", window.gets_focus_visible()));
    pointer.tap_key(gdk::Key::Tab);
    settle(&test);
    seen.push(("Tab", window.gets_focus_visible()));
    click(&test, &mut pointer, &home_row(&test));
    seen.push(("a click after Tab", window.gets_focus_visible()));
    pointer.tap_key(gdk::Key::F6);
    settle(&test);
    seen.push(("F6", window.gets_focus_visible()));
    click(&test, &mut pointer, &home_row(&test));
    pointer.tap_key(gdk::Key::Down);
    settle(&test);
    seen.push(("Down", window.gets_focus_visible()));
    click(&test, &mut pointer, &home_row(&test));
    seen.push(("a click after Down", window.gets_focus_visible()));
    // Back to where Xvfb starts the pointer (see tests/focus.rs).
    pointer.move_to(720, 500);

    assert_eq!(
        seen,
        [
            ("startup", false),
            ("a click on Home", false),
            ("Ctrl+C", false),
            ("Tab", true),
            ("a click after Tab", false),
            ("F6", true),
            ("Down", true),
            ("a click after Down", false),
        ]
    );
}
