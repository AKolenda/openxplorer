// SPDX-License-Identifier: AGPL-3.0-only
//! The file list, details pane and status bar geometry, measured against
//! the current app (see [`super::geometry`]).

use gtk::prelude::*;

use super::geometry::{bounds, laid_out, Bounds};
use crate::test_support::harness::{descendants, wait_for_frames, Fixture, TestWindow, ThemeGuard};
use crate::window::widget_tree::children;

/// The column titles of the details view, left to right.
fn column_titles(test: &TestWindow) -> Vec<gtk::Widget> {
    let details = test.window.folder_pane().details().column_view();
    let header = children(details)
        .find(|child| child.css_name() == "header")
        .expect("the details view has a header");
    // The Folder path title of a search is there but hidden.
    children(&header).filter(WidgetExt::is_visible).collect()
}

/// The first row of the details view.
fn first_row(test: &TestWindow) -> gtk::Widget {
    let details = test.window.folder_pane().details().column_view();
    let list = children(details)
        .find(|child| child.css_name() == "listview")
        .expect("the details view has a list");
    children(&list)
        .find(|child| child.css_name() == "row")
        .expect("the fixture has rows")
}

/// parity: VIEW-001
#[gtk::test]
fn columns_run_from_14_pixels_in_with_the_web_widths() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let titles: Vec<Bounds> = column_titles(&test)
        .iter()
        .map(|title| bounds(&test, title))
        .collect();
    let widths: Vec<i32> = titles.iter().map(|title| title.width).collect();
    assert_eq!(
        &widths[1..],
        [176, 135, 90 + 14],
        "Date, Type, and Size with the end padding"
    );
    assert_eq!(titles[0].x, list.x, "Name holds the 14 pixels before the columns");
    assert_eq!(
        titles[3].right(),
        list.right(),
        "Size holds the 14 pixels after them"
    );
    assert!(
        titles.iter().all(|title| title.height == 37),
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
        bounds(&test, size_label).right(),
        bounds(&test, size_title).right() - 32,
        "12 pixels of padding and the 14-pixel end, as `padding-right:17px` \
         plus the list's padding in style.css, and no room for an arrow"
    );
}

/// parity: VIEW-001
#[gtk::test]
fn rows_are_inset_12_pixels_and_their_cells_sit_under_the_titles() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let row = first_row(&test);
    let row_place = bounds(&test, &row);
    assert_eq!(
        (row_place.x, row_place.width, row_place.height),
        (list.x + 12, list.width - 24, 36)
    );
    let title_x: Vec<i32> = column_titles(&test)
        .iter()
        .map(|title| bounds(&test, title).x)
        .collect();
    let cell_x: Vec<i32> = children(&row).map(|cell| bounds(&test, &cell).x).collect();
    assert_eq!(cell_x, title_x);
}

#[gtk::test]
fn the_details_pane_spaces_its_parts_as_the_current_app() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let pane = test.window.details_pane();
    let pane_place = bounds(&test, pane);
    assert_eq!(pane_place.width, 262);
    let frames = descendants::<gtk::CenterBox>(pane);
    let preview = frames.first().expect("the pane has a preview");
    let preview_place = bounds(&test, preview);
    assert_eq!(
        (preview_place.x, preview_place.width, preview_place.height),
        (pane_place.x + 23, 217, 148)
    );
    assert_eq!(
        preview_place.y,
        pane_place.y + 22 + 24 + 20,
        "padding, the 24px header and its margin"
    );
}

/// parity: VIEW-006
#[gtk::test]
fn the_status_bar_view_buttons_are_24_pixels_3_apart_with_the_view_highlighted() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let status_buttons: Vec<gtk::Button> = descendants::<gtk::Button>(test.window.status_bar());
    let placed: Vec<Bounds> = status_buttons
        .iter()
        .map(|button| bounds(&test, button))
        .collect();
    assert!(
        placed
            .iter()
            .all(|button| (button.width, button.height) == (24, 24)),
        "{placed:?}"
    );
    for pair in placed.windows(2) {
        assert_eq!(pair[1].x - pair[0].right(), 3, "3 pixels apart");
    }
    assert_eq!(test.window.status_bar().active_view_buttons(), ["Details view"]);
    test.activate("view", Some("large"));
    wait_for_frames(&test.window, 2);
    assert_eq!(test.window.status_bar().active_view_buttons(), ["Large icons"]);
}

#[gtk::test]
fn a_selected_tile_keeps_the_text_colour() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("theme", Some("light"));
    test.activate("view", Some("large"));
    test.window.folder_model().select_only(1);
    wait_for_frames(&test.window, 3);
    let grid = test.window.folder_pane().icon_view().grid();
    let labels = descendants::<gtk::Label>(grid);
    let selected = labels.iter().find(|label| label.text() == "Notes 2.txt");
    let other = labels.iter().find(|label| label.text() == "Notes 10.txt");
    let (Some(selected), Some(other)) = (selected, other) else {
        panic!("the grid shows both names");
    };
    assert_eq!(
        selected.color().to_str(),
        other.color().to_str(),
        "not GTK's white selected text on the pale selection"
    );
}
