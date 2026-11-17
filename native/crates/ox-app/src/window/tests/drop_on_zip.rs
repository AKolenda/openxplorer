// SPDX-License-Identifier: AGPL-3.0-only
//! A real drag onto a ZIP opened like a folder (ARC-026, DND-011). Windows
//! adds dropped items to a zipped folder ("To add files or folders to a
//! zipped folder you created earlier, drag them to the zipped folder",
//! support.microsoft.com, Zip and unzip files); this app's ZIPs are
//! read-only, so the drag is refused and the window says why while it
//! hovers, as GTK delivers no drop to a place that takes none.

use gtk::prelude::*;
use ox_core::location::ArchiveLocation;

use super::archives::fixture_with_zip;
use super::right_drag::{item_on_screen, HOLD, PROBE};
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};

/// What the window says when items are dragged onto a ZIP.
const READ_ONLY: &str = "An archive is read-only. Extract it to add items.";

/// A real drag of a file onto a window that shows a ZIP like a folder
/// adds nothing to the ZIP, keeps the file, and says why while it hovers.
///
/// parity: ARC-026, DND-011
#[gtk::test]
fn a_drag_onto_an_open_zip_says_it_is_read_only() {
    let fixture = fixture_with_zip();
    let zip_bytes = std::fs::read(fixture.path("Bundle.zip")).expect("the fixture ZIP");
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    let root = ArchiveLocation::root(&fixture.uri_of("Bundle.zip"));
    let other = test.open_beside(&root.uri());
    wait_until("the ZIP in the other window", || {
        other.window.current_uri() == Some(root.uri())
    });
    wait_for_frames(&other.window, 3);
    // The new window covers the first: drag it down by its blank title
    // bar, so the first window's items show above it.
    let blank = descendants::<gtk::Box>(&other.window.titlebar().expect("a title bar"))
        .into_iter()
        .find(|child| child.has_css_class("title-drag"))
        .expect("the bar has its blank drag area");
    let middle = (f64::from(blank.width()) / 2.0, f64::from(blank.height()) / 2.0);
    let grab = screen_point(&mut pointer, &blank, middle, PROBE);
    pointer.move_to(grab.0, grab.1);
    pointer.press(Button::Primary);
    pointer.glide(grab, (grab.0, grab.1 + 420), 20);
    wait_for(HOLD);
    pointer.release(Button::Primary);
    wait_for(HOLD);
    wait_for_frames(&other.window, 3);

    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let zip_view = other.window.folder_pane().input_widget();
    let onto = screen_point(&mut pointer, &zip_view, (80.0, 60.0), PROBE);
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    wait_for(HOLD);
    let said_while_hovering = other.window.shown_message();
    pointer.release(Button::Primary);
    wait_for(HOLD);
    pointer.move_to(720, 500);

    assert_eq!(said_while_hovering, READ_ONLY);
    assert!(fixture.path("Notes 2.txt").is_file(), "the file stayed");
    assert_eq!(
        std::fs::read(fixture.path("Bundle.zip")).expect("the fixture ZIP"),
        zip_bytes,
        "nothing was added to the ZIP"
    );
}
