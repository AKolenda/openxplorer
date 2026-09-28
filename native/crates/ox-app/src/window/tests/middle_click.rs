// SPDX-License-Identifier: AGPL-3.0-only
//! Middle-clicks open places in tabs, against `bindMiddleOpen` and
//! `bindMiddleClick` in `desktop/ui/app.js` (TAB-020 to TAB-025): a folder
//! row or tile, a sidebar place and a landing-page card open in a
//! background tab, a file never opens, and a tab closes.
//!
//! The private display has no pointer, so these tests emit what the
//! widgets' middle-button gestures see (see [`super::support::click_at`]).

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::support::{click_at, middle_click_at, middle_of, Release};
use crate::folder_view::grid::IconSize;
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, Fixture, TestWindow};
use crate::window::FolderView;

/// Each tab's location, left to right.
fn tab_uris(test: &TestWindow) -> Vec<String> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
}

/// Whether the last tab has been listed.
fn last_tab_is_listed(test: &TestWindow) -> bool {
    let session = test.window.imp().session.borrow();
    let last = session.tabs().last().expect("a tab");
    !last.listing_state.needs_listing()
}

/// Middle-clicks the item called `name` in the visible folder view.
fn middle_click_item(test: &TestWindow, name: &str) {
    wait_for_frames(&test.window, 2);
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    let cell = pane
        .owners()
        .widget_at(test.position_of(name))
        .unwrap_or_else(|| panic!("{name} is on screen"));
    middle_click_at(&view, middle_of(&cell, &view));
}

/// A folder opens behind the current tab; the current tab keeps its
/// folder and selection, and the new tab is listed only when shown.
///
/// parity: TAB-020, TAB-009
#[gtk::test]
fn middle_clicking_a_folder_row_opens_it_in_a_background_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");

    middle_click_item(&test, "Documents");

    assert_eq!(tab_uris(&test), [fixture.uri(), fixture.uri_of("Documents")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    wait_for(Duration::from_millis(50));
    assert!(!last_tab_is_listed(&test), "nothing is listed until shown");
}

/// parity: TAB-020
#[gtk::test]
fn middle_clicking_a_folder_tile_opens_it_in_a_background_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.show_view(FolderView::Icons(IconSize::Large));

    middle_click_item(&test, "Documents");

    assert_eq!(tab_uris(&test), [fixture.uri(), fixture.uri_of("Documents")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: TAB-022
#[gtk::test]
fn middle_clicking_a_file_opens_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    middle_click_item(&test, "Notes 2.txt");

    assert_eq!(test.window.tab_count(), 1);
    assert!(test.context.recorded_launches().is_empty(), "no app starts");
}

/// A press alone does nothing: the gesture acts on the release.
///
/// parity: TAB-025
#[gtk::test]
fn a_middle_press_without_its_release_opens_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 2);
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    let cell = pane
        .owners()
        .widget_at(test.position_of("Documents"))
        .expect("Documents is on screen");

    click_at(
        &view,
        gtk::gdk::BUTTON_MIDDLE,
        middle_of(&cell, &view),
        Release::Held,
    );

    assert_eq!(test.window.tab_count(), 1);
}

/// parity: TAB-023
#[gtk::test]
fn middle_clicking_a_sidebar_place_opens_it_in_a_background_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 2);
    let sidebar = test.window.sidebar();
    let labels = sidebar.labels();
    let home = labels
        .iter()
        .position(|label| label == "Home")
        .expect("the sidebar lists Home");
    let row = sidebar
        .list()
        .row_at_index(i32::try_from(home).expect("a short sidebar"))
        .expect("a row for every label");

    middle_click_at(sidebar.list(), middle_of(&row, sidebar.list()));

    let home_uri = ox_core::location::file_uri(&gtk::glib::home_dir());
    assert_eq!(tab_uris(&test), [fixture.uri(), home_uri]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: TAB-024
#[gtk::test]
fn middle_clicking_a_this_pc_card_opens_it_behind_the_page() {
    let test = TestWindow::open(Page::ThisPc.uri());
    wait_for_frames(&test.window, 2);
    let landing = test.window.folder_pane().landing();
    let card = descendants::<gtk::Button>(landing)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.go-to"))
        .expect("This PC shows a card for a drive");
    let target = card
        .action_target_value()
        .and_then(|target| target.get::<String>())
        .expect("a card names its location");

    middle_click_at(&card, (4.0, 4.0));

    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::ThisPc.uri()));
    assert_eq!(tab_uris(&test)[1], target);
}

/// parity: TAB-002
#[gtk::test]
fn middle_clicking_a_tab_closes_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    let tab_list = test.window.tab_strip().tab_list();
    let first = tab_list.first_child().expect("two tabs");

    middle_click_at(&first, (4.0, 4.0));

    assert_eq!(tab_uris(&test), [fixture.uri_of("Documents")]);
}
