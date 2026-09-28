// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's geometry, measured against the current app (see
//! [`super::geometry`]): 35-pixel rows 36 pixels apart, the Quick access
//! rows 4 pixels further in and 35 pixels apart, the accent bar on the
//! selected row, the 6-pixel resizer and the "Map network location"
//! button below the list.

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out, Bounds};
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

/// A window whose Quick access holds the fixture folder and Documents.
fn with_two_pins(fixture: &Fixture) -> TestWindow {
    let test = laid_out(&fixture.uri());
    test.activate("pin-folder", None);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("Documents");
    test.activate("pin-folder", None);
    wait_until("both pins", || {
        test.window.sidebar().labels().contains(&"Documents".to_owned())
    });
    wait_for_frames(&test.window, 3);
    test
}

#[gtk::test]
fn quick_access_rows_sit_4_pixels_in_on_a_35_pixel_pitch() {
    let fixture = Fixture::standard();
    let test = with_two_pins(&fixture);
    let home = bounds(&test, &row_named(&test, "Home"));
    assert_eq!(
        home,
        Bounds::new(7, 173, 196, 35),
        "Home is 14 pixels below the command bar"
    );
    let quick_access: Vec<Bounds> = descendants::<gtk::ListBoxRow>(test.window.sidebar().list())
        .into_iter()
        .filter(|row| row.has_css_class("quick-access"))
        .map(|row| bounds(&test, &row))
        .collect();
    assert!(quick_access.len() >= 2, "the known folders and two pins");
    assert_eq!(quick_access[0], Bounds::new(11, 234, 188, 35));
    for pair in quick_access.windows(2) {
        assert_eq!(pair[1].y - pair[0].y, 35, "Quick access rows touch");
    }
    let last = quick_access[quick_access.len() - 1];
    let this_pc = bounds(&test, &row_named(&test, "This PC"));
    let local_disk = bounds(&test, &row_named(&test, "Local Disk"));
    let separator_gap = 12 + 1 + 12;
    assert_eq!(
        this_pc.y,
        last.y + 35 + 4 + 1 + separator_gap,
        "4px padding and a 1px border end Quick access, then a separator"
    );
    assert_eq!(local_disk.y - this_pc.y, 36, "other rows are a pixel apart");
    assert_eq!(this_pc.x, 7);
}

#[gtk::test]
fn the_selected_row_shows_the_accent_bar_at_its_edge() {
    let fixture = Fixture::standard();
    let test = with_two_pins(&fixture);
    let documents = row_named(&test, "Documents");
    assert!(documents.is_selected(), "the open folder's row is selected");
    let bar = descendants::<gtk::Box>(&documents)
        .into_iter()
        .find(|part| part.has_css_class("pill"))
        .expect("every row has the accent bar");
    let row = bounds(&test, &documents);
    // 3 x 16 with 2px corners (ui-spec.md §4.4; the web's is 3 x 15).
    assert_eq!(bounds(&test, &bar), Bounds::new(row.x, row.y + 10, 3, 16));
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

/// parity: SIDE-001
#[gtk::test]
fn map_network_location_waits_below_the_list_for_its_milestone() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let button = button_for(&test, "win.map-network-location");
    assert!(!button.is_sensitive(), "connecting to a share is not ported yet");
    let tooltip = button.tooltip_text().unwrap_or_default();
    assert!(tooltip.contains("network and device support"), "{tooltip}");
    let place = bounds(&test, &button);
    let window_height = test.window.height();
    assert_eq!((place.x, place.width, place.height), (7, 196, 34));
    assert_eq!(
        place.y + place.height + 11 + 30,
        window_height,
        "above the status bar"
    );
}
