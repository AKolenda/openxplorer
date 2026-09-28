// SPDX-License-Identifier: AGPL-3.0-only
//! Listing folders: order, filters, reloads that keep the view, the folder
//! watch and failures.

use std::cell::Cell;
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{
    settle, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, STANDARD_NAMES,
};
use crate::window::folder_pane::{FolderView, PanePage};
use crate::window::session::Direction;

use super::geometry::bounds;

/// A folder long enough to scroll in a 810-pixel window.
const LONG_FOLDER: usize = 300;

/// True when a scroll position moved less than one row, as GTK keeps the
/// view anchored to an item rather than to a pixel offset.
fn keeps_position(position: f64, before: f64) -> bool {
    const ROW_HEIGHT: f64 = 40.0;
    (position - before).abs() < ROW_HEIGHT
}

/// Selects and focuses the last item, scrolls it into view and returns the
/// scroll position once it settles.
fn scroll_to_the_end(test: &TestWindow) -> f64 {
    let pane = test.window.folder_pane();
    let last = pane.model().n_items() - 1;
    pane.model().select_only(last);
    pane.focus_view();
    pane.reveal(last);
    wait_until("the view to scroll", || pane.scroll_position() > 0.0);
    wait_for(Duration::from_millis(100));
    pane.scroll_position()
}

/// What the folder pane showed while a reload ran.
#[derive(Debug)]
struct ReloadObservation {
    /// The empty (or loading) page replaced the rows at some point.
    showed_empty_page: bool,
    /// The fewest items the view held.
    fewest_items: u32,
}

/// Runs the main loop until the active tab is listed, watching the pane.
fn observe_reload(test: &TestWindow) -> ReloadObservation {
    let pane = test.window.folder_pane();
    let fewest_items = Rc::new(Cell::new(pane.model().n_items()));
    let fewest = Rc::clone(&fewest_items);
    let handler = pane
        .model()
        .sorted()
        .connect_items_changed(move |model, _, _, _| {
            fewest.set(fewest.get().min(model.n_items()));
        });
    let mut showed_empty_page = false;
    let deadline = Instant::now() + Duration::from_secs(8);
    while test.window.is_loading() {
        assert!(Instant::now() < deadline, "the reload did not finish");
        settle();
        showed_empty_page |= pane.page() == Some(PanePage::Empty);
    }
    pane.model().sorted().disconnect(handler);
    ReloadObservation {
        showed_empty_page,
        fewest_items: fewest_items.get(),
    }
}

/// parity: VIEW-015
#[gtk::test]
fn folders_come_first_then_names_in_natural_order() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(test.window.load_error(), None);
    assert_eq!(
        test.names(),
        STANDARD_NAMES,
        "the hidden .private file stays hidden"
    );
}

/// Rows show while the folder is still being listed, and a selection made
/// among them is kept when the listing ends.
///
/// parity: PERF-002
#[gtk::test]
fn a_selection_made_while_listing_survives_the_end_of_the_listing() {
    let fixture = Fixture::standard();
    let test = TestWindow::without_tabs();
    let selected_while_listing = Rc::new(Cell::new(false));
    let selected = Rc::clone(&selected_while_listing);
    let window = test.window.downgrade();
    let model = test.window.folder_model().sorted().clone();
    model.connect_items_changed(move |_, _, _, _| {
        let Some(window) = window.upgrade() else { return };
        let listing = window.is_loading() && window.folder_model().n_items() > 1;
        if listing && !selected.get() {
            selected.set(true);
            window.folder_model().select_only(1);
        }
    });
    test.show(&fixture.uri());
    assert!(
        selected_while_listing.get(),
        "the selection was made before the listing ended"
    );
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// parity: NAV-013, NAV-014
#[gtk::test]
fn refresh_keeps_the_rows_scroll_position_focus_and_selection() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    let scrolled = scroll_to_the_end(&test);
    let selected = test.selected_names();
    let focused_item = test.window.folder_model().selected_items()[0].clone();
    test.window.refresh();
    let reload = observe_reload(&test);
    let pane = test.window.folder_pane();
    assert!(
        !reload.showed_empty_page,
        "the rows stay on screen while reloading"
    );
    assert_eq!(
        reload.fewest_items,
        u32::try_from(LONG_FOLDER).expect("small count")
    );
    assert!(
        keeps_position(pane.scroll_position(), scrolled),
        "the view keeps its scroll position"
    );
    assert_eq!(test.selected_names(), selected);
    let item_after = test.window.folder_model().selected_items()[0].clone();
    assert_eq!(item_after, focused_item, "unchanged rows keep their item objects");
    assert!(pane.view_has_focus(), "keyboard focus stays in the folder view");
}

/// parity: VIEW-055
#[gtk::test]
fn a_change_on_disk_is_listed_and_keeps_the_scroll_position() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    let scrolled = scroll_to_the_end(&test);
    fixture.write("new download.txt");
    wait_until("the folder watch to list the new file", || {
        test.names().contains(&"new download.txt".to_owned())
    });
    let pane = test.window.folder_pane();
    assert!(
        keeps_position(pane.scroll_position(), scrolled),
        "the view keeps its scroll position"
    );
    assert_eq!(test.selected_names(), ["file 0299.txt"]);
}

/// parity: NAV-015
#[gtk::test]
fn navigating_to_another_folder_starts_at_the_top_without_a_selection() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    fs::create_dir(fixture.path("Subfolder")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri());
    scroll_to_the_end(&test);
    test.window
        .navigate(&fixture.uri_of("Subfolder"))
        .expect("valid folder");
    test.wait_for_listing("the subfolder");
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder again");
    assert!(test.window.folder_pane().scroll_position() < 1.0);
    assert!(test.selected_names().is_empty());
}

#[gtk::test]
fn the_folder_watch_survives_a_reload() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let watch_id = |test: &TestWindow| {
        let session = test.window.imp().session.borrow();
        let tab = session.active().expect("one tab");
        tab.watch.as_ref().map(crate::folder_view::watch::Watch::id)
    };
    let before = watch_id(&test);
    assert!(before.is_some(), "the folder is watched");
    test.window.refresh();
    assert!(test.window.is_loading(), "the reload is still running");
    assert_eq!(watch_id(&test), before, "a reload keeps the same watch");
    test.wait_for_listing("the reload");
    assert_eq!(watch_id(&test), before);
}

#[gtk::test]
fn a_change_seen_while_listing_lists_the_folder_once_more() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let id = test.active_tab().expect("one tab");
    test.window.refresh();
    fixture.write("late change.txt");
    test.window.folder_changed(id);
    let pending = test
        .window
        .imp()
        .session
        .borrow()
        .tab(id)
        .is_some_and(|tab| tab.listing_state.has_pending_reload());
    assert!(pending, "a change during a listing waits for it to finish");
    wait_until("the second listing", || {
        test.names().contains(&"late change.txt".to_owned()) && !test.window.is_loading()
    });
}

/// parity: NAV-016
#[gtk::test]
fn a_superseded_listing_cannot_fill_the_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.window.navigate(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the latest navigation");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.names(), STANDARD_NAMES);
}

/// parity: NAV-035, VIEW-047
#[gtk::test]
fn a_missing_folder_says_it_is_unavailable_and_offers_try_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(test.window.navigate("https://example.invalid").is_err());
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "a refused address changes nothing"
    );
    let missing = fixture.path("Missing folder");
    test.window
        .navigate(missing.to_str().expect("fixture paths are UTF-8"))
        .expect("a missing folder is still a valid address");
    test.wait_for_listing("the failed listing");
    assert!(test.window.load_error().is_some());
    let pane = test.window.folder_pane();
    assert_eq!(pane.page(), Some(PanePage::Empty));
    assert_eq!(pane.empty_page().title(), "This location is unavailable");
    assert!(
        pane.empty_page().offers_try_again(),
        "a visible Try again button runs win.refresh"
    );
    fs::create_dir(&missing).expect("the folder appears");
    test.activate("refresh", None);
    test.wait_for_listing("the retried listing");
    assert_eq!(test.window.load_error(), None);
    assert_eq!(pane.empty_page().title(), "This folder is empty");
}

/// parity: SRCH-003, VIEW-023
#[gtk::test]
fn the_filter_and_hidden_files_change_what_is_listed_until_the_folder_changes() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("hidden", None);
    assert!(test.names().contains(&".private".to_owned()));
    test.activate("hidden", None);
    let search = &test.window.chrome().search.entry;
    search.set_text("notes 10");
    wait_until("the name filter", || test.names() == ["Notes 10.txt"]);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the empty subfolder");
    assert!(test.names().is_empty());
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("history back");
    assert_eq!(test.names(), STANDARD_NAMES);
    assert_eq!(
        search.text().as_str(),
        "",
        "moving to another folder clears the filter"
    );
}

#[gtk::test]
fn only_the_visible_view_holds_the_model() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    assert!(pane.details().model().is_some());
    assert!(
        pane.grid().model().is_none(),
        "the hidden icon view builds no tiles"
    );
    test.activate("view", Some("large"));
    assert_eq!(
        pane.view(),
        FolderView::Icons(crate::folder_view::grid::IconSize::Large)
    );
    assert!(
        pane.details().model().is_none(),
        "the hidden details view builds no rows"
    );
    assert!(pane.grid().model().is_some());
    assert!(
        pane.grid().max_columns() < 64,
        "the tile budget follows the width"
    );
    test.activate("view", Some("details"));
    assert!(pane.details().model().is_some());
    assert!(
        pane.grid().model().is_none(),
        "switching back detaches the icon view"
    );
}

/// The loading line lies over the top of the folder pane, as the web's
/// absolutely positioned `.loading-line`, so showing it never moves the
/// items.
///
/// parity: VIEW-047
#[gtk::test]
fn the_loading_line_lies_over_the_pane_without_moving_the_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 2);
    let pane = test.window.folder_pane();
    let items_before = bounds(&test, &pane.view_widget());
    let line = pane.loading_line();
    // A reload that the folder watch starts may hide the line again, so
    // it is shown until it has been laid out.
    wait_until("the loading line to be laid out", || {
        line.set_visible(true);
        line.height() > 0
    });
    let line_place = bounds(&test, line);
    let pane_place = bounds(&test, pane);
    assert_eq!(
        (line_place.y, line_place.height),
        (pane_place.y, 2),
        "2 pixels over the pane's top"
    );
    assert_eq!(
        bounds(&test, &pane.view_widget()),
        items_before,
        "the items stay put"
    );
}
