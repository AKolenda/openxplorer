// SPDX-License-Identifier: AGPL-3.0-only
//! The file list, details pane and status bar geometry, measured against
//! the current app (see [`super::geometry`]).

use gtk::prelude::*;

use super::geometry::{bounds, laid_out, Bounds};
use crate::test_support::harness::{descendants, wait_for_frames, Fixture, TestWindow};

/// The children of `widget`, in order.
fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut child = widget.as_ref().first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        found.push(current);
    }
    found
}

/// The column titles of the details view, left to right.
fn column_titles(test: &TestWindow) -> Vec<gtk::Widget> {
    let details = &test.window.content().details;
    let header = children(details)
        .into_iter()
        .find(|child| child.css_name() == "header")
        .expect("the details view has a header");
    children(&header)
}

/// The first row of the details view.
fn first_row(test: &TestWindow) -> gtk::Widget {
    let details = &test.window.content().details;
    let list = children(details)
        .into_iter()
        .find(|child| child.css_name() == "listview")
        .expect("the details view has a list");
    children(&list)
        .into_iter()
        .find(|child| child.css_name() == "row")
        .expect("the fixture has rows")
}

fn right(bounds: Bounds) -> i32 {
    bounds.0 + bounds.2
}

/// parity: VIEW-001
#[gtk::test]
fn columns_run_from_14_pixels_in_with_the_web_widths() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, &test.window.content().details);
    let titles: Vec<Bounds> = column_titles(&test)
        .iter()
        .map(|title| bounds(&test, title))
        .collect();
    let widths: Vec<i32> = titles.iter().map(|title| title.2).collect();
    assert_eq!(
        &widths[1..],
        [152, 135, 90 + 14],
        "Date, Type, and Size with the end padding"
    );
    assert_eq!(titles[0].0, list.0, "Name holds the 14 pixels before the columns");
    assert_eq!(
        right(titles[3]),
        right(list),
        "Size holds the 14 pixels after them"
    );
    assert!(
        titles.iter().all(|title| title.3 == 37),
        "37-pixel titles above a 1-pixel line"
    );
}

#[gtk::test]
fn the_size_title_is_right_aligned_and_only_the_sorted_column_has_an_arrow() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let titles = column_titles(&test);
    let size_title = titles.last().expect("four titles");
    let labels = descendants::<gtk::Label>(size_title);
    let size_label = labels.first().expect("the Size title has a label");
    assert_eq!(size_label.text().as_str(), "Size");
    assert_eq!(
        right(bounds(&test, size_label)),
        right(bounds(&test, size_title)) - 32,
        "12 pixels of padding and the 14-pixel end, as `padding-right:17px` \
         plus the list's padding in style.css, and no room for an arrow"
    );
}

/// parity: VIEW-001
#[gtk::test]
fn rows_are_inset_12_pixels_and_their_cells_sit_under_the_titles() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, &test.window.content().details);
    let row = first_row(&test);
    let (row_x, _, row_width, row_height) = bounds(&test, &row);
    assert_eq!((row_x, row_width, row_height), (list.0 + 12, list.2 - 24, 36));
    let title_x: Vec<i32> = column_titles(&test)
        .iter()
        .map(|title| bounds(&test, title).0)
        .collect();
    let cell_x: Vec<i32> = children(&row).iter().map(|cell| bounds(&test, cell).0).collect();
    assert_eq!(cell_x, title_x);
}

#[gtk::test]
fn the_details_pane_spaces_its_parts_as_the_current_app() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let pane = &test.window.details_pane().root;
    let (pane_x, pane_y, pane_width, _) = bounds(&test, pane);
    assert_eq!(pane_width, 262);
    let frames = descendants::<gtk::CenterBox>(pane);
    let preview = frames.first().expect("the pane has a preview");
    let (preview_x, preview_y, preview_width, preview_height) = bounds(&test, preview);
    assert_eq!(
        (preview_x, preview_width, preview_height),
        (pane_x + 23, 217, 148)
    );
    assert_eq!(
        preview_y,
        pane_y + 22 + 24 + 20,
        "padding, the 24px header and its margin"
    );
}

/// parity: VIEW-006
#[gtk::test]
fn the_status_bar_view_buttons_are_24_pixels_3_apart_with_the_view_highlighted() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let status_buttons: Vec<gtk::Button> = descendants::<gtk::Button>(&test.window.chrome().status.root);
    let placed: Vec<Bounds> = status_buttons
        .iter()
        .map(|button| bounds(&test, button))
        .collect();
    assert!(
        placed.iter().all(|button| (button.2, button.3) == (24, 24)),
        "{placed:?}"
    );
    for pair in placed.windows(2) {
        assert_eq!(pair[1].0 - right(pair[0]), 3, "3 pixels apart");
    }
    assert_eq!(
        test.window.chrome().status.active_view_buttons(),
        ["Details view"]
    );
    test.activate("view", Some("large"));
    wait_for_frames(&test.window, 2);
    assert_eq!(test.window.chrome().status.active_view_buttons(), ["Large icons"]);
}
