// SPDX-License-Identifier: AGPL-3.0-only
//! Right-button drags made with a real pointer on check.py's private
//! display (DND-027): GTK's own drag source and drop target run, so these
//! catch what tests that call the drop handlers directly cannot, such as
//! the button being known only while the drag is prepared.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};

/// Where the pointer first looks for the window.
pub(super) const PROBE: (i32, i32) = (8, 8);

/// How long the pointer rests so GTK's drag catches up.
pub(super) const HOLD: Duration = Duration::from_millis(300);

/// The screen point of the middle of the name of the item `name`.
pub(super) fn item_on_screen(pointer: &mut Pointer, test: &TestWindow, name: &str) -> (i32, i32) {
    let view = test.window.folder_pane().input_widget();
    let position = test.position_of(name);
    let cell = test
        .window
        .folder_pane()
        .owners()
        .file_cell_at(position, &view)
        .expect("the item is shown");
    let bounds = cell.compute_bounds(&view).expect("the item is laid out");
    let point = (
        f64::from(bounds.x()) + 30.0,
        f64::from(bounds.y() + bounds.height() / 2.0),
    );
    screen_point(pointer, &view, point, PROBE)
}

/// The screen point near the bottom of the folder view, on blank space.
pub(super) fn blank_on_screen(pointer: &mut Pointer, test: &TestWindow) -> (i32, i32) {
    let view = test.window.folder_pane().input_widget();
    let point = (60.0, f64::from(view.height()) - 30.0);
    screen_point(pointer, &view, point, PROBE)
}

/// Presses `button` at `from`, moves to `to` and releases there.
fn drag_with(pointer: &mut Pointer, button: Button, from: (i32, i32), to: (i32, i32)) {
    pointer.move_to(from.0, from.1);
    pointer.press(button);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), to, 12);
    wait_for(HOLD);
    pointer.release(button);
}

/// A real right-button drag dropped onto blank space of the folder it
/// comes from opens the right-drag menu there, with no Move here, as in
/// Windows Explorer; the file stays where it is.
///
/// parity: DND-027
#[gtk::test]
fn a_real_right_drag_dropped_in_its_own_folder_opens_the_menu() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let to = blank_on_screen(&mut pointer, &test);

    drag_with(&mut pointer, Button::Secondary, from, to);
    let menu = test.window.drop_menu();
    wait_until("the right-drag menu", || menu.is_mapped());
    let labels = menu.row_labels();
    menu.popdown();

    assert_eq!(labels, ["Copy here", "Create shortcuts here", "-", "Cancel"]);
    assert!(fixture.path("Notes 2.txt").is_file(), "nothing moved");
}

/// A real right-button drag dropped on a folder asks instead of moving,
/// and a real left-button drag there moves, as in Windows Explorer.
///
/// parity: DND-017, DND-027
#[gtk::test]
fn a_real_right_drag_onto_a_folder_asks_and_a_left_drag_moves() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = item_on_screen(&mut pointer, &test, "Documents");

    drag_with(&mut pointer, Button::Secondary, from, onto);
    let menu = test.window.drop_menu();
    wait_until("the right-drag menu", || menu.is_mapped());
    let bold = menu.emphasised_labels();
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    wait_for(HOLD);
    let kept = fixture.path("Notes 2.txt").is_file();
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = item_on_screen(&mut pointer, &test, "Documents");
    drag_with(&mut pointer, Button::Primary, from, onto);

    assert_eq!(bold, ["Move here"]);
    assert!(kept, "the right-button drop moved nothing before the answer");
    wait_until("the move", || fixture.path("Documents/Notes 2.txt").is_file());
}

/// The screen point of the row labelled `label` in the sidebar, `y` in
/// the list's coordinates.
fn sidebar_on_screen(pointer: &mut Pointer, test: &TestWindow, y: f64) -> (i32, i32) {
    let list = test.window.sidebar().list();
    screen_point(pointer, list, (60.0, y), PROBE)
}

/// Pins the fixture's new folder `name` to Quick access and comes back to
/// the fixture's folder; the folder's path.
fn pin_new_folder(test: &TestWindow, fixture: &Fixture, name: &str) -> std::path::PathBuf {
    std::fs::create_dir(fixture.path(name)).expect("fixture subfolder");
    test.window.navigate(&fixture.uri_of(name)).expect("a folder");
    test.wait_for_listing(name);
    test.activate("pin-folder", None);
    wait_until("the pin", || {
        test.window.sidebar().labels().contains(&name.to_owned())
    });
    back_to(test, fixture);
    fixture.path(name)
}

/// Shows the fixture's folder again, laid out, once any operation ended:
/// no drop is taken while one runs.
fn back_to(test: &TestWindow, fixture: &Fixture) {
    wait_until("the operation to end", || {
        test.window.imp().file_operations.borrow().is_idle()
    });
    test.window.navigate(&fixture.uri()).expect("a folder");
    test.wait_for_listing("the fixture");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
}

/// Real drags onto a pinned folder in the navigation pane go into it, as
/// in Windows Explorer: the right button asks with its menu and the left
/// one copies or moves; a drop between pins still pins.
///
/// parity: DND-014, DND-027
#[gtk::test]
fn real_drags_onto_a_sidebar_folder_go_into_it_and_between_pins_pin() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    let shelf = pin_new_folder(&test, &fixture, "Drop shelf");
    let sidebar = test.window.sidebar();

    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = sidebar_on_screen(&mut pointer, &test, sidebar.middle_of("Drop shelf"));
    drag_with(&mut pointer, Button::Secondary, from, onto);
    let menu = test.window.drop_menu();
    wait_until("the right-drag menu", || menu.is_mapped());
    let labels = menu.row_labels();
    menu.click_row("Copy here");
    wait_until("the copy", || shelf.join("Notes 2.txt").is_file());
    back_to(&test, &fixture);

    let from = item_on_screen(&mut pointer, &test, "Notes 10.txt");
    let onto = sidebar_on_screen(&mut pointer, &test, sidebar.middle_of("Drop shelf"));
    drag_with(&mut pointer, Button::Primary, from, onto);
    wait_until("the left drop", || shelf.join("Notes 10.txt").is_file());
    back_to(&test, &fixture);

    std::fs::create_dir(fixture.path("Pinned between")).expect("fixture subfolder");
    wait_until("the new folder", || {
        test.names().contains(&"Pinned between".to_owned())
    });
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Pinned between");
    let between = sidebar_on_screen(&mut pointer, &test, sidebar.top_edge_of("Drop shelf"));
    drag_with(&mut pointer, Button::Primary, from, between);
    wait_until("the pin", || {
        sidebar.labels().contains(&"Pinned between".to_owned())
    });
    let pinned_moved_nothing =
        fixture.path("Pinned between").is_dir() && !shelf.join("Pinned between").exists();
    test.window.unpin(&fixture.uri_of("Pinned between"));
    test.window.unpin(&fixture.uri_of("Drop shelf"));

    assert_eq!(
        labels,
        ["Copy here", "Move here", "Create shortcuts here", "-", "Cancel"]
    );
    assert!(pinned_moved_nothing, "pinning moves nothing");
}
