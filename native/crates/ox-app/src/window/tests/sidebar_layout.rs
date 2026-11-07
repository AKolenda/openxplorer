// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's geometry, measured against the current app (see
//! [`super::geometry`]), with Windows 11's denser rows (owner decision
//! 2026-11-06): 31-pixel rows 32 pixels apart, the pinned folders full
//! width on the same pitch with their icons in Home's column, as in
//! Windows 11's navigation pane, the accent bar's room on the selected
//! row, the 6-pixel resizer and the "Map network drive"
//! button below the list.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::geometry::{bounds, button_for, laid_out, Bounds};
use super::icons::{assert_same_colour, painted_colour};
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// The sidebar row named `label`.
fn row_named(test: &TestWindow, label: &str) -> gtk::ListBoxRow {
    let labels = test.window.sidebar().labels();
    let index = labels.iter().position(|shown| shown == label);
    let index = index.unwrap_or_else(|| panic!("{label} is in the sidebar: {labels:?}"));
    let index = i32::try_from(index).expect("a short sidebar");
    test.window
        .sidebar()
        .list()
        .row_at_index(index)
        .expect("a row for every label")
}

/// How many sidebar rows are labelled `label`.
fn rows_labelled(test: &TestWindow, label: &str) -> usize {
    let labels = test.window.sidebar().labels();
    labels.iter().filter(|shown| *shown == label).count()
}

/// A window whose Quick access holds the fixture folder and Documents.
///
/// The standard Documents folder has a row from the start, so the
/// fixture's Documents is pinned once a second row carries the name.
/// Quick access lists the pins before the standard folders.
fn with_two_pins(fixture: &Fixture) -> TestWindow {
    let test = laid_out(&fixture.uri());
    test.activate("pin-folder", None);
    // One pin request runs at a time.
    wait_until("the first pin", || !test.window.imp().pinning.get());
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("Documents");
    test.activate("pin-folder", None);
    wait_until("both pins", || rows_labelled(&test, "Documents") == 2);
    wait_for_frames(&test.window, 3);
    test
}

/// The pinned folders are rows like Home and This PC, full width on the
/// 32-pixel pitch, as in Windows 11's navigation pane; a divider 24
/// pixels tall separates Home, the pins and This PC with Network, and
/// none separates This PC from Network. Every top-level icon sits in one
/// column, 28 pixels into the pane, a chevron's room after the edge, and
/// the drives' icons a 16-pixel level further in.
///
/// parity: LOOK-014, SIDE-001
#[gtk::test]
fn pinned_rows_line_up_with_home_on_the_32_pixel_pitch() {
    let fixture = Fixture::standard();
    let test = with_two_pins(&fixture);
    let home = bounds(&test, &row_named(&test, "Home"));
    assert_eq!(
        home,
        Bounds::new(7, 152, 196, 31),
        "Home is 14 pixels below the command bar"
    );
    let pinned: Vec<Bounds> = descendants::<gtk::ListBoxRow>(test.window.sidebar().list())
        .into_iter()
        .filter(|row| row.has_css_class("quick-access"))
        .map(|row| bounds(&test, &row))
        .collect();
    assert!(pinned.len() >= 2, "the known folders and two pins");
    let divider = 12 + 1 + 11;
    assert_eq!(pinned[0], Bounds::new(7, home.y + 31 + divider + 1, 196, 31));
    for pair in pinned.windows(2) {
        assert_eq!(pair[1].y - pair[0].y, 32, "pins are 32 apart, as every row");
    }
    let last = pinned[pinned.len() - 1];
    let this_pc = bounds(&test, &row_named(&test, "This PC"));
    let local_disk = bounds(&test, &row_named(&test, "Local Disk"));
    let network = bounds(&test, &row_named(&test, "Network"));
    assert_eq!(this_pc.y, last.y + 31 + divider + 1, "a divider after the pins");
    assert_eq!(local_disk.y - this_pc.y, 32, "32 apart as in Windows 11");
    assert_eq!(
        network.y - local_disk.y,
        32,
        "no divider between This PC and Network"
    );
    assert_eq!(this_pc.x, 7);

    let icon_x = |label: &str| {
        let row = row_named(&test, label);
        let icon = descendants::<gtk::Widget>(&row)
            .into_iter()
            .find(|part| part.has_css_class("side-glyph") || part.has_css_class("side-art"))
            .unwrap_or_else(|| panic!("{label} has an icon"));
        bounds(&test, &icon).x
    };
    let column = icon_x("Home");
    assert_eq!(column, 28, "a chevron's room before the icon");
    assert_eq!(icon_x("Documents"), column, "pins line up with Home");
    assert_eq!(icon_x("This PC"), column, "This PC's chevron sits in that room");
    assert_eq!(icon_x("Network"), column);
    assert_eq!(icon_x("Local Disk"), column + 16, "drives are a level in");
}

/// The open place is marked by its row's fill alone, as in Windows 11
/// Explorer's navigation pane: the accent bar keeps its room at the
/// row's edge but paints nothing there.
///
/// parity: LOOK-014
#[gtk::test]
fn the_selected_row_is_marked_by_its_fill_without_an_accent_bar() {
    let fixture = Fixture::standard();
    let test = with_two_pins(&fixture);
    let documents = row_named(&test, "Documents");
    assert!(documents.is_selected(), "the open folder's row is selected");
    let bar = descendants::<gtk::Box>(&documents)
        .into_iter()
        .find(|part| part.has_css_class("pill"))
        .expect("every row has the accent bar");
    let row = bounds(&test, &documents);
    // 3 x 16 (ui-spec.md §4.4; the web's is 3 x 15), kept for the layout.
    assert_eq!(bounds(&test, &bar), Bounds::new(row.x, row.y + 8, 3, 16));
    assert_same_colour(
        painted_colour(&documents, 1, 18),
        painted_colour(&documents, 6, 18),
        "the bar's place shows the row's fill",
    );
}

/// parity: SIDE-023
#[gtk::test]
fn the_sidebar_is_210_pixels_with_a_6_pixel_resizer() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let sidebar = bounds(&test, test.window.sidebar());
    assert_eq!(sidebar.width, 210);
    let list = bounds(&test, test.window.folder_pane());
    assert_eq!(list.x, 216, "the file list starts after the resizer");
}

/// parity: SIDE-001, NET-001
#[gtk::test]
fn map_network_location_sits_below_the_list() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let button = button_for(&test, "win.map-network-location");
    assert!(button.is_sensitive(), "Map network drive opens its dialog");
    assert_eq!(button.tooltip_text().as_deref(), Some("Map network drive"));
    let place = bounds(&test, &button);
    let window_height = test.window.height();
    assert_eq!((place.x, place.width, place.height), (7, 196, 34));
    assert_eq!(
        place.y + place.height + 11 + 30,
        window_height,
        "above the status bar"
    );
}
