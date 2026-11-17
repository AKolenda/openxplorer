// SPDX-License-Identifier: AGPL-3.0-only
//! Narrow windows, against the `@media(max-width)` rules of
//! `v2.0.0:desktop/ui/style.css`: the details pane gives way at 960 pixels
//! without changing the preference, tabs narrow, and a compact window
//! drops the search box, some commands and two columns.

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out, laid_out_with_details_pane};
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};

/// Resizes the window to `width` pixels and waits until it is laid out.
pub(super) fn resize(test: &TestWindow, width: i32) {
    test.window.set_default_size(width, 720);
    wait_until("the new width", || (test.window.width() - width).abs() <= 8);
    wait_for_frames(&test.window, 3);
}

/// parity: LOOK-022
#[gtk::test]
fn the_details_pane_gives_way_below_961_pixels_and_comes_back() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
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
    for action in ["win.cut", "win.rename", "win.share"] {
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

/// How a folder pane's Details view lays its columns out: the width its
/// columns take, the width shown, and Name's width, title included.
struct ColumnRoom {
    columns: f64,
    shown: f64,
    name: i32,
}

/// The column room of `pane`'s Details view.
fn column_room(test: &TestWindow, pane: &crate::window::folder_pane::FolderPane) -> ColumnRoom {
    let details = pane.details();
    let view = details.column_view();
    let scroller = view
        .parent()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("the view scrolls");
    let header = view.first_child().expect("the header");
    let name_title = header.first_child().expect("the Name title");
    ColumnRoom {
        columns: scroller.hadjustment().upper(),
        shown: scroller.hadjustment().page_size(),
        name: bounds(test, &name_title).width,
    }
}

/// Name, the column that takes the room left, gives way first when the
/// list is narrower than its columns: down to 140 pixels (plus the
/// 14-pixel end), and only then does the list scroll sideways. With the
/// details pane open in a 1000-pixel window, the list's columns all fit:
/// before, Name kept 260 pixels and Type and Size were cut off at the
/// pane's edge, reachable only by a scroll bar that shows only under the
/// pointer.
///
/// parity: LOOK-022
#[gtk::test]
fn the_columns_fit_beside_the_details_pane_and_name_gives_way_first() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    resize(&test, 1000);
    let pane = test.window.details_pane();
    assert!(pane.is_visible(), "the pane is open");
    let room = column_room(&test, test.window.folder_pane());
    assert!(
        room.columns <= room.shown + 1.0,
        "every column shows beside the pane: {} of {} pixels",
        room.shown,
        room.columns
    );
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    assert!(
        list.x + list.width <= bounds(&test, pane).x,
        "nothing of the list lies under the pane"
    );
    let name = test
        .window
        .folder_pane()
        .details()
        .column(SortColumn::Name)
        .expect("a Name column");
    assert!(name.expands(), "Name still takes any room left");
    resize(&test, 1320);
    let room = column_room(&test, test.window.folder_pane());
    assert!(room.name > 260 + 14, "a wide window gives Name its room back");
}

/// In a narrower window the details pane gives way to the list: between
/// 961 and 1190 pixels it narrows with the window, from 235 pixels down to
/// 180, instead of keeping 235.
///
/// parity: LOOK-022
#[gtk::test]
fn the_details_pane_narrows_with_the_window() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let pane = test.window.details_pane();
    resize(&test, 1180);
    let at_1180 = bounds(&test, pane).width;
    resize(&test, 1000);
    let at_1000 = bounds(&test, pane).width;
    assert!(
        at_1000 < at_1180,
        "narrower in a narrower window: {at_1000} and {at_1180}"
    );
    assert!(
        (180..=190).contains(&at_1000),
        "about 180 pixels near 961: {at_1000}"
    );
    assert!((230..=235).contains(&at_1180), "235 pixels near 1190: {at_1180}");
}

/// A 900-pixel window, alone and split in two: in each pane the columns
/// either all show, or Name has given way to its narrowest and the list
/// scrolls sideways to the rest. Before, Name kept 260 pixels in a split
/// pane while Type and Size were cut off.
///
/// parity: LOOK-022, VIEW-059
#[gtk::test]
fn in_a_900_pixel_window_name_gives_way_before_the_list_scrolls() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 900);
    let check = |what: &str, pane: &crate::window::folder_pane::FolderPane| {
        let room = column_room(&test, pane);
        let fits = room.columns <= room.shown + 1.0;
        assert!(
            fits || room.name == 140 + 14,
            "{what}: the columns fit, or Name is at its narrowest before the list scrolls: Name {} pixels, {} of {} shown",
            room.name,
            room.shown,
            room.columns
        );
        if !fits {
            assert!(
                room.columns > room.shown,
                "{what}: the rest is reached by scrolling"
            );
        }
    };
    check("one pane", test.window.folder_pane());
    let alone = column_room(&test, test.window.folder_pane());
    assert!(
        alone.columns <= alone.shown + 1.0,
        "a 900-pixel window shows every column"
    );
    test.activate("split-view", None);
    wait_for_frames(&test.window, 3);
    for (index, pane) in test.window.folder_panes().iter().enumerate() {
        check(&format!("split pane {index}"), pane);
    }
}
