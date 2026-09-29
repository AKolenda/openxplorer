// SPDX-License-Identifier: AGPL-3.0-only
//! The search box: filtering a folder nobody indexed, searching the cache
//! of an indexed one, the search strip, the Folder path column, the status
//! bar, and caching a folder from the window.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::{
    Caching, GioFolderReader, HiddenItems, IndexService, RootOrigin, RootStatus, SearchIndex,
};

use crate::folder_view::sorting::SortColumn;
use crate::search::SearchScope;
use crate::test_support::harness::{capture, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::activation::{activation_for, Activation};
use crate::window::folder_pane::PanePage;

impl TestWindow {
    /// Types `text` into the search box and waits until its search has
    /// run.
    pub(super) fn search_for(&self, text: &str) {
        self.window.search_box().entry().set_text(text);
        wait_until("the search to run", || {
            let count = self.status_count();
            !count.starts_with("Searching")
        });
    }

    /// The status bar's item count.
    pub(super) fn status_count(&self) -> String {
        let (count, _selection) = self.window.status_bar().texts();
        count
    }

    /// Whether the details view shows `column`.
    pub(super) fn shows_column(&self, column: SortColumn) -> bool {
        let details = self.window.folder_pane().details();
        details.column(column).is_some_and(|column| column.is_visible())
    }
}

/// parity: SRCH-001, SRCH-002, SRCH-003, SRCH-012, VIEW-042, VIEW-050
#[gtk::test]
fn typing_filters_a_folder_nobody_indexed_and_says_so() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();

    test.search_for("notes");

    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt"]);
    let strip = test.window.search_strip();
    assert!(strip.is_visible());
    assert_eq!(strip.caption(), "Current folder + subfolders");
    assert!(strip.offers_to_cache_folder());
    assert_eq!(strip.shown_note(), None);
    assert_eq!(test.status_count(), "2 results");
    assert!(test.shows_column(SortColumn::FolderPath));
    assert!(!test.shows_column(SortColumn::Modified));
    capture(&test.window, "search-folder-filter.png");
}

/// A folder nobody indexed is searched live with its subfolders, and
/// wildcards work there too.
///
/// parity: SRCH-035
#[gtk::test]
fn a_folder_nobody_indexed_is_searched_with_its_subfolders() {
    let fixture = Fixture::standard();
    fs::create_dir_all(fixture.path("Documents/Deep")).expect("fixture subfolder");
    fs::write(fixture.path("Documents/Deep/notes archive.TXT"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());

    test.search_for("notes");

    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes archive.TXT"]);
    assert_eq!(test.status_count(), "3 results");
    let deep = test.window.folder_model().item(2).expect("a third result");
    let expected = fixture.path("Documents/Deep");
    assert_eq!(deep.folder_path().text, expected.to_string_lossy());

    test.search_for("*.txt notes");

    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes archive.TXT"]);
}

/// parity: SRCH-001, SRCH-002
#[gtk::test]
fn escape_empties_the_box_and_brings_the_listing_back() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes 10");
    assert_eq!(test.names(), ["Notes 10.txt"]);

    test.window.search_box().entry().emit_stop_search();

    wait_until("the listing to return", || test.names() == STANDARD_NAMES);
    assert_eq!(test.window.search_box().entry().text().as_str(), "");
    assert!(!test.window.search_strip().is_visible());
    assert!(test.shows_column(SortColumn::Modified));
    assert!(!test.shows_column(SortColumn::FolderPath));
    assert_eq!(test.status_count(), "4 items");
}

/// Enter moves to the results and keeps the search; Escape empties the
/// box, and a second Escape moves to the view.
///
/// parity: SRCH-006
#[gtk::test]
fn enter_and_a_second_escape_move_focus_to_the_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    let entry = test.window.search_box().entry();
    let pane = test.window.folder_pane();
    test.activate("search", None);
    wait_until("the box to take focus", || entry.focus_child().is_some());

    entry.emit_activate();

    wait_until("the results to take focus", || pane.view_has_focus());
    assert_eq!(entry.text().as_str(), "notes");
    test.activate("search", None);
    wait_until("the box to take focus again", || entry.focus_child().is_some());
    entry.emit_stop_search();
    assert_eq!(entry.text().as_str(), "");
    assert!(!pane.view_has_focus(), "the first Escape only empties the box");
    entry.emit_stop_search();
    wait_until("the view to take focus", || pane.view_has_focus());
}

/// parity: SRCH-012
#[gtk::test]
fn the_strips_clear_button_ends_the_search() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");

    test.window.search_strip().click_clear();

    wait_until("the listing to return", || test.names() == STANDARD_NAMES);
    assert!(!test.window.search_strip().is_visible());
}

/// parity: SRCH-007, SRCH-009, SRCH-012, VIEW-042, VIEW-050
#[gtk::test]
fn an_indexed_folder_is_searched_in_the_cache_with_its_subfolders() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Documents/Deep")).expect("fixture subfolder");
    fs::write(fixture.path("Documents/Deep/notes archive.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());

    test.search_for("notes");

    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes archive.txt"]);
    let strip = test.window.search_strip();
    assert_eq!(strip.caption(), "Cached names & paths");
    assert!(!strip.offers_to_cache_folder());
    assert_eq!(
        strip.shown_note().as_deref(),
        Some("Cached metadata · see update coverage in Settings")
    );
    assert_eq!(test.status_count(), "3 results · Cached");
    let deep = test.window.folder_model().item(2).expect("a third result");
    let expected = fixture.path("Documents/Deep");
    assert_eq!(deep.folder_path().text, expected.to_string_lossy());
    capture(&test.window, "search-cached.png");
}

/// A folder with only an indexed subfolder shows its own matches and the
/// cached ones below it (the 1.1.0 fix).
///
/// parity: SRCH-007, SRCH-012
#[gtk::test]
fn a_folder_with_an_indexed_subfolder_shows_both() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/notes inside.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri_of("Documents"));

    test.search_for("notes");

    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes inside.txt"]);
    let strip = test.window.search_strip();
    assert_eq!(strip.caption(), "Current folder + cached subfolders");
    assert!(strip.offers_to_cache_folder());
    assert_eq!(
        strip.shown_note().as_deref(),
        Some("Other subfolders are not indexed.")
    );
}

/// parity: SRCH-011
#[gtk::test]
fn all_cached_folders_searches_only_the_cache() {
    let fixture = Fixture::standard();
    let elsewhere = Fixture::standard();
    fs::write(elsewhere.path("notes elsewhere.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&elsewhere.uri());
    test.search_for("elsewhere");
    assert!(test.names().is_empty());

    test.window
        .search_strip()
        .choose_scope(SearchScope::AllCachedFolders);

    wait_until("the search of every cached folder", || {
        test.names() == ["notes elsewhere.txt"]
    });
    assert_eq!(test.window.search_strip().caption(), "Cached names & paths");
}

/// parity: SRCH-013
#[gtk::test]
fn a_search_that_finds_nothing_says_why() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.search_for("budget");

    let pane = test.window.folder_pane();
    assert_eq!(pane.page(), Some(PanePage::Empty));
    assert_eq!(pane.empty_page().title(), "No matching items");
    assert_eq!(
        pane.empty_page().message(),
        "No items found in this folder or its subfolders."
    );
}

/// parity: SRCH-018, SRCH-028
#[gtk::test]
fn a_new_file_in_an_indexed_folder_appears_in_the_shown_search() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Documents/Deep")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("invoice");
    assert!(test.names().is_empty());

    fs::write(fixture.path("Documents/Deep/invoice.pdf"), b"x").expect("fixture file");

    wait_until("the live update to reach the search", || {
        test.names() == ["invoice.pdf"]
    });
}

/// parity: SRCH-014, SRCH-015
#[gtk::test]
fn open_file_location_selects_the_result_in_its_folder() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/deep notes.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("deep");
    assert_eq!(test.names(), ["deep notes.txt"]);
    test.window.folder_model().select_only(0);

    test.activate("open-file-location", None);

    test.wait_for_listing("the result's folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.selected_names(), ["deep notes.txt"]);
    assert_eq!(test.window.search_box().entry().text().as_str(), "");
}

/// A folder result middle-clicked opens behind in a new tab and the
/// search stays; a ZIP result opens in the archive browser.
///
/// parity: SRCH-014
#[gtk::test]
fn a_middle_clicked_folder_result_opens_a_tab_and_keeps_the_search() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Documents/Reports")).expect("fixture subfolder");
    fs::write(fixture.path("Documents/reports 2026.zip"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("reports");
    assert_eq!(test.names(), ["Reports", "reports 2026.zip"]);
    let result = |position| test.window.folder_model().item(position).expect("a result");
    let zip = result(1);
    assert_eq!(activation_for(zip.entry()), Activation::Archive);
    let Activation::Folder(folder) = activation_for(result(0).entry()) else {
        panic!("a folder result opens as a folder");
    };

    test.activate("open-tab-background", Some(&folder));

    wait_until("the folder to open in a second tab", || {
        test.window.imp().session.borrow().tabs().len() == 2
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.window.search_box().entry().text().as_str(), "reports");
    assert_eq!(test.names(), ["Reports", "reports 2026.zip"]);
}

/// parity: SRCH-020
#[gtk::test]
fn caching_the_folder_from_the_strip_searches_the_cache() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.search_for("notes");

    test.activate("cache-folder", None);

    test.wait_for_root(&fixture.uri(), RootStatus::Ready);
    wait_until("the search to run in the cache", || {
        test.window.search_strip().caption() == "Cached names & paths"
    });
    assert_eq!(
        test.window.shown_message().as_str(),
        "Caching filenames and paths in the background. No file contents are downloaded."
    );
    let state = test
        .window
        .window_action_state(crate::window::WindowAction::CacheFolder);
    assert_eq!(state.and_then(|state| state.get::<bool>()), Some(true));

    test.activate("cache-folder", None);

    wait_until("the folder to stop being cached", || {
        let roots = test.context.search_cache().roots();
        roots.iter().all(|root| !root.is_enabled())
    });
    assert_eq!(
        test.window.shown_message().as_str(),
        "Cache disabled; this root’s indexed names were removed."
    );
}

/// parity: NAV-013
#[gtk::test]
fn refresh_while_searching_runs_the_search_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    fs::write(fixture.path("notes 3.txt"), b"x").expect("fixture file");

    test.activate("refresh", None);

    assert_eq!(test.window.search_box().entry().text().as_str(), "notes");
    assert!(test.window.search_strip().is_visible(), "the search stays");
}

/// parity: SRCH-040
#[gtk::test]
fn pinning_a_folder_indexes_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.start_search_cache();

    test.activate("pin-folder", None);

    wait_until("the pinned folder to be indexed", || {
        let roots = test.context.search_cache().roots();
        roots
            .iter()
            .any(|root| root.uri == fixture.uri_of("Documents") && root.origin == RootOrigin::Pin)
    });
}

/// parity: SRCH-001
#[gtk::test]
fn ctrl_f_moves_focus_to_the_box_and_selects_its_text() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    test.window.folder_pane().focus_view();

    test.activate("search", None);

    let entry = test.window.search_box().entry();
    wait_until("the box to take focus", || entry.focus_child().is_some());
    assert_eq!(entry.selection_bounds(), Some((0, 5)));
}

/// parity: SRCH-002, SEL-015
#[gtk::test]
fn typing_clears_the_selection_and_scrolls_to_the_top() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let last = pane.model().n_items() - 1;
    pane.model().select_only(last);
    pane.reveal(last);
    wait_until("the view to scroll", || pane.scroll_position() > 0.0);

    test.window.search_box().entry().set_text("file");

    wait_until("the view to scroll back", || pane.scroll_position() == 0.0);
    assert!(test.selected_names().is_empty());
}

/// A folder another process indexes shows up in this window's cache
/// status, and its search runs in the cache.
///
/// parity: SRCH-018, SRCH-027
#[gtk::test]
fn a_folder_another_process_indexed_is_searched_in_the_cache() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    let cache_directory = test.settings_directory().join("cache");
    let other_process = IndexService::start(
        SearchIndex::open(&cache_directory).expect("the shared cache opens"),
        GioFolderReader,
        || {},
    )
    .expect("a second service starts beside the owner");

    other_process
        .configure(&fixture.uri(), Caching::Enabled, "Elsewhere", HiddenItems::Skip)
        .expect("the folder can be indexed");

    test.wait_for_root(&fixture.uri(), RootStatus::Ready);
    test.search_for("notes");
    wait_until("the search to use the cache", || {
        test.window.search_strip().caption() == "Cached names & paths"
    });
}
