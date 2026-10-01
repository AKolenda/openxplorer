// SPDX-License-Identifier: AGPL-3.0-only
//! Narrow windows, against the `@media(max-width)` rules of
//! `v2.0.0:desktop/ui/style.css`: the details pane gives way at 960 pixels
//! without changing the preference, tabs narrow, and a compact window
//! drops the search box, some commands and two columns.

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out};
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};

/// Resizes the window to `width` pixels and waits until it is laid out.
fn resize(test: &TestWindow, width: i32) {
    test.window.set_default_size(width, 720);
    wait_until("the new width", || (test.window.width() - width).abs() <= 8);
    wait_for_frames(&test.window, 3);
}

/// parity: LOOK-022
#[gtk::test]
fn the_details_pane_gives_way_below_961_pixels_and_comes_back() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let pane = test.window.details_pane();
    assert!(pane.is_visible());
    resize(&test, 940);
    assert!(!pane.is_visible(), "no room for the pane");
    let switched_on = test
        .window
        .action_state("details-pane")
        .and_then(|state| state.get::<bool>());
    assert_eq!(switched_on, Some(true), "still switched on");
    let tab = test.window.tab_strip().tab_list().first_child().expect("a tab");
    assert_eq!(bounds(&test, &tab).width, 180, "narrower tabs");
    resize(&test, 1320);
    assert!(pane.is_visible(), "the pane comes back");
}

/// parity: LOOK-022
#[gtk::test]
fn a_compact_window_drops_the_search_box_some_commands_and_two_columns() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 672);
    assert!(!test.window.search_box().is_visible());
    for action in ["win.cut", "win.rename", "win.copy-path"] {
        assert!(!button_for(&test, action).is_visible(), "{action} is hidden");
    }
    assert!(button_for(&test, "win.copy").is_visible(), "Copy stays");
    let details = test.window.folder_pane().details();
    for column in [SortColumn::Modified, SortColumn::Type] {
        let shown = details.column(column).is_some_and(|column| column.is_visible());
        assert!(!shown, "{column:?} is hidden");
    }
    resize(&test, 1320);
    assert!(test.window.search_box().is_visible());
    assert!(button_for(&test, "win.cut").is_visible());
}

/// parity: LOOK-022
#[gtk::test]
fn the_name_column_keeps_260_pixels_and_the_list_scrolls_sideways() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 1000);
    let details = test.window.folder_pane().details();
    let name = details.column(SortColumn::Name).expect("a Name column");
    let header = details.column_view().first_child().expect("the header");
    let name_title = header.first_child().expect("the Name title");
    assert_eq!(
        bounds(&test, &name_title).width,
        260 + 14,
        "260 pixels and the 14-pixel end"
    );
    assert!(name.expands(), "Name still takes any room left");
}
