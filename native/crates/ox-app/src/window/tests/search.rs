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

use super::file_ops_support::{is_enabled, select_names};
use crate::folder_view::sorting::SortColumn;
use crate::search::SearchScope;
use crate::test_support::harness::{
    capture, wait_for_frames, wait_until, Fixture, OpenedWindows, TestWindow, STANDARD_NAMES,
};
use crate::window::activation::{activation_for, Activation};
use crate::window::folder_pane::PanePage;

impl TestWindow {
    /// Turns Settings > Search > "Show the search filter bar" on or off,
    /// as Settings or another window would (SRCH-046): the tests of the
    /// filter bar itself turn it on.
    pub(super) fn show_search_filter_bar(&self, on: bool) {
        let update = ox_core::settings::PreferencesUpdate {
            show_search_filter_bar: Some(on),
            ..ox_core::settings::PreferencesUpdate::default()
        };
        ox_core::settings::Settings::open(self.settings_directory())
            .update_preferences(&update)
            .expect("the settings file takes the change");
        self.context.reload_settings();
        wait_until("the window to follow the setting", || {
            self.context.settings_data().preferences.show_search_filter_bar == on
        });
    }

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
    test.show_search_filter_bar(true);
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

/// `?` stands for exactly one character, in the folder and in its
/// subfolders, as in Windows 11 File Explorer.
///
/// parity: SRCH-004, SRCH-035
#[gtk::test]
fn a_question_mark_finds_names_with_one_character_there() {
    let fixture = Fixture::standard();
    fs::create_dir_all(fixture.path("Documents/Deep")).expect("fixture subfolder");
    for name in [
        "extra_2.txt",
        "extra_3.txt",
        "extra_30.txt",
        "extra_31.txt",
        "what?.txt",
    ] {
        fs::write(fixture.path(name), b"x").expect("fixture file");
    }
    fs::write(fixture.path("Documents/Deep/extra_32.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());

    test.search_for("extra_??.txt");
    assert_eq!(test.names(), ["extra_30.txt", "extra_31.txt", "extra_32.txt"]);
    test.search_for("extra_?.txt");
    assert_eq!(test.names(), ["extra_2.txt", "extra_3.txt"]);
    test.search_for("what?.txt");
    assert_eq!(test.names(), ["what?.txt"]);
    test.search_for("extra_????.txt");
    assert!(test.names().is_empty());
}

/// parity: SRCH-035, PERF-005
#[gtk::test]
fn a_live_search_caps_the_initial_listing_and_reports_truncation() {
    let fixture = Fixture::standard();
    for number in 0..=crate::search::RESULT_LIMIT {
        fs::write(fixture.path(&format!("match {number}.txt")), b"x").expect("fixture file");
    }
    let test = TestWindow::open(&fixture.uri());
    test.show_search_filter_bar(true);

    test.search_for("match");

    assert_eq!(test.names().len(), crate::search::RESULT_LIMIT);
    assert_eq!(test.status_count(), "500 results (first 500)");
    assert_eq!(
        test.window.search_strip().shown_note().as_deref(),
        Some("First 500 results · narrow your search")
    );
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
    assert!(
        test.status_count().starts_with("4 items"),
        "{}",
        test.status_count()
    );
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

/// Opening another folder ends the search, unless the strip's pin keeps
/// it: then it runs again in the folder opened.
///
/// parity: SRCH-005
#[gtk::test]
fn the_pin_keeps_the_search_when_changing_folders() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/notes inside.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    let strip = test.window.search_strip();
    assert!(!strip.keeps_search(), "off until pressed");
    strip.set_keeps_search(true);

    test.activate("go-to", Some(&fixture.uri_of("Documents")));

    wait_until("the search to run in the folder opened", || {
        test.names() == ["notes inside.txt"] && test.status_count() == "1 result"
    });
    assert_eq!(test.window.search_box().entry().text().as_str(), "notes");
    strip.set_keeps_search(false);
    test.activate("go-to", Some(&fixture.uri()));
    test.wait_for_listing("the first folder");
    assert_eq!(test.window.search_box().entry().text().as_str(), "");
    assert_eq!(test.names(), STANDARD_NAMES);
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
    test.show_search_filter_bar(true);
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
    test.show_search_filter_bar(true);
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

/// The rows a shown search holds now.
fn shown_rows(test: &TestWindow) -> gtk::gio::ListStore {
    let search = test.window.imp().search.borrow();
    search.results().cloned().expect("the search shows its own rows")
}

/// A change in the cache brings a shown cached search up to date in
/// place: the selected result stays the same row, still selected, and the
/// strip does not go back to "Searching…".
///
/// parity: SRCH-018
#[gtk::test]
fn a_cache_change_keeps_the_selected_result_of_a_cached_search() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("notes");
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt"]);
    select_names(&test, &["Notes 2.txt"]);
    let rows = shown_rows(&test);
    let selected = test.window.folder_model().selected_items();

    fs::write(fixture.path("Documents/notes 3.txt"), b"x").expect("fixture file");

    wait_until("the new file to reach the shown search", || {
        test.names().contains(&"notes 3.txt".to_owned())
    });
    assert_eq!(
        shown_rows(&test),
        rows,
        "the rows on screen are updated, not replaced"
    );
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(test.window.folder_model().selected_items(), selected);
    assert_eq!(test.status_count(), "3 results · Cached");
}

/// Indexing another folder changes the cache every few seconds; a walk of
/// a folder nobody indexed does not start over each time, so its rows and
/// selection stay.
///
/// parity: SRCH-018, SRCH-035
#[gtk::test]
fn a_cache_change_elsewhere_leaves_a_walked_search_alone() {
    let fixture = Fixture::standard();
    let elsewhere = Fixture::standard();
    fs::create_dir_all(fixture.path("Documents/Deep")).expect("fixture subfolder");
    fs::write(fixture.path("Documents/Deep/notes archive.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.search_for("notes");
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes archive.txt"]);
    select_names(&test, &["notes archive.txt"]);
    let rows = shown_rows(&test);

    test.index_folder(&elsewhere.uri());

    assert_eq!(shown_rows(&test), rows, "the walk was not started over");
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes archive.txt"]);
    assert_eq!(test.selected_names(), ["notes archive.txt"]);
    assert_eq!(
        test.window.search_strip().caption(),
        "Current folder + subfolders"
    );
}

/// A move made in the window tells the search cache that both folders
/// changed, so a share without a live watch shows it at once.
///
/// parity: SRCH-033
#[gtk::test]
fn file_operations_tell_the_search_cache_which_folders_changed() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    let written = |folder: String| {
        let cache = test.context.search_cache().clone();
        wait_until("the cache to read the folder again", move || {
            cache.written_folders().contains(&folder)
        });
    };
    select_names(&test, &["Notes 2.txt"]);

    test.activate("cut", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);

    written(fixture.uri_of("Documents"));
    written(fixture.uri());
    test.search_for("notes 2");
    assert_eq!(test.names(), ["Notes 2.txt"]);
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

/// "Open file location in new tab" opens the result's folder behind, with
/// the result selected there, and the search stays; "in new window" opens
/// it in a window of its own.
///
/// parity: SRCH-016
#[gtk::test]
fn a_results_folder_opens_in_a_new_tab_or_window_with_it_selected() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/deep notes.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("deep");
    test.window.folder_model().select_only(0);
    let search_tab = test.active_tab().expect("a tab in front");

    test.activate("open-file-location-in-tab", None);

    assert_eq!(test.active_tab(), Some(search_tab), "the new tab opens behind");
    assert_eq!(test.window.search_box().entry().text().as_str(), "deep");
    let tabs: Vec<_> = test
        .window
        .imp()
        .session
        .borrow()
        .tabs()
        .iter()
        .map(|tab| tab.id)
        .collect();
    let [_, located] = tabs[..] else {
        panic!("a second tab: {tabs:?}");
    };
    test.activate_tab(located);
    test.wait_for_listing("the result's folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.selected_names(), ["deep notes.txt"]);

    test.activate_tab(search_tab);
    test.search_for("deep");
    test.window.folder_model().select_only(0);
    test.activate("open-file-location-in-window", None);

    let opened = OpenedWindows::only(&[&test.window]);
    let window = opened.window();
    wait_until("the new window to list the folder", || {
        let listed = window.folder_model().n_items() > 0;
        listed && window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    let selected = window.folder_model().selected_items();
    let names: Vec<&str> = selected.iter().map(|item| item.entry().name.as_str()).collect();
    assert_eq!(names, ["deep notes.txt"]);
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
    assert!(test.window.is_searching(), "the search stays");
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
    for view in ["details", "compact", "large"] {
        let test = TestWindow::open(&fixture.uri());
        test.activate("view", Some(view));
        wait_for_frames(&test.window, 2);
        let pane = test.window.folder_pane();
        let last = pane.model().n_items() - 1;
        pane.model().select_only(last);
        pane.reveal(last);
        wait_until("the view to scroll", || pane.scroll_position() > 0.0);

        // Typing follows Search (Ctrl+F), so the last file no longer owns focus.
        test.activate("search", None);
        let entry = test.window.search_box().entry();
        wait_until("the search box to take focus", || entry.focus_child().is_some());
        entry.set_text("file");

        wait_until(&format!("the {view} view to scroll back"), || {
            pane.scroll_position() == 0.0
        });
        wait_for_frames(&test.window, 2);
        assert!(
            pane.scroll_position().abs() < f64::EPSILON,
            "the view stays at the top after layout"
        );
        assert!(test.selected_names().is_empty());
        entry.set_text("no matching filename");
        wait_until("an empty search", || pane.model().n_items() == 0);
        wait_for_frames(&test.window, 2);
        assert!(pane.scroll_position().abs() < f64::EPSILON);
        assert!(entry.focus_child().is_some(), "scrolling keeps focus in Search");
    }
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

/// Polling a shared cache must invalidate an already displayed search in
/// the follower, even though only the owner's worker emits local notices.
#[gtk::test]
fn a_followers_active_search_updates_when_the_owner_changes_names() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/cached-old.txt"), b"Synthetic data").unwrap();
    let test = TestWindow::open(&fixture.uri());
    let index = SearchIndex::open(&test.settings_directory().join("cache")).unwrap();
    let owner = IndexService::start(index.clone(), GioFolderReader, || {}).unwrap();
    assert!(owner.is_owner());
    owner
        .configure(&fixture.uri(), Caching::Enabled, "Shared", HiddenItems::Skip)
        .unwrap();
    wait_until("the owner's first scan", || {
        index
            .roots()
            .unwrap()
            .iter()
            .any(|root| root.status == RootStatus::Ready)
    });
    test.start_search_cache();
    test.search_for("cached");
    assert_eq!(test.names(), ["cached-old.txt"]);
    assert_eq!(test.window.search_strip().caption(), "Cached names & paths");

    fs::rename(
        fixture.path("Documents/cached-old.txt"),
        fixture.path("Documents/cached-new.txt"),
    )
    .unwrap();
    owner.folder_changed(&fixture.uri_of("Documents")).unwrap();
    wait_until("the owner to index the rename", || {
        owner.tick(&ox_core::search::IndexSettings::default()).unwrap();
        index
            .search(&ox_core::search::SearchQuery::new("cached-new"), None)
            .unwrap()
            .hits
            .len()
            == 1
    });
    test.context.search_cache().refresh_status();
    wait_until("the follower's shown search to follow the owner", || {
        test.names() == ["cached-new.txt"]
    });
    owner.shut_down();
}

/// A live search has its own result store. Finishing a directory reload
/// must replace matches that disappeared while the query stayed open.
#[gtk::test]
fn a_directory_reload_updates_an_active_search_after_a_rename() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt"]);
    fs::rename(fixture.path("Notes 10.txt"), fixture.path("Renamed.txt")).unwrap();
    fs::write(fixture.path("More notes.txt"), "").unwrap();

    test.window.folder_changed(test.active_tab().unwrap());

    wait_until("the search to reflect the completed directory listing", || {
        let mut names = test.names();
        names.sort();
        names == ["More notes.txt", "Notes 2.txt"]
    });
}

/// A search whose walk finds nothing past the folder's own matches keeps
/// showing those rows as typing filtered them, rather than replacing them
/// with a copy, which made the list build all its rows again a moment
/// after every pause in typing.
///
/// parity: SRCH-045
#[gtk::test]
fn a_walk_that_finds_nothing_more_keeps_the_rows_typing_filtered() {
    let fixture = Fixture::with_files(40);
    let test = TestWindow::open(&fixture.uri());
    let model = test.window.folder_pane().model();

    test.search_for("file 001");

    assert_eq!(test.names().len(), 11, "{:?}", test.names());
    assert!(
        test.window.imp().search.borrow().results().is_none(),
        "the walk found nothing more, so the filtered listing stays"
    );
    let position = model
        .position_of_uri(&fixture.uri_of("file 0010.txt"))
        .expect("listed");
    let first = model.item(position).expect("a row");
    // One more key, as typing adds it.
    let entry = test.window.search_box().entry();
    let mut end = -1;
    entry.insert_text("0", &mut end);
    wait_until("the narrower search", || test.names() == ["file 0010.txt"]);
    assert_eq!(model.item(0), Some(first), "the same row object, not a copy");
}

/// A walk that does find more past the folder's own matches adds them
/// to the rows as before.
///
/// parity: SRCH-045
#[gtk::test]
fn a_walk_that_finds_more_shows_them_with_the_folders_own_matches() {
    let fixture = Fixture::standard();
    fixture.write("Documents/Notes 3.txt");
    let test = TestWindow::open(&fixture.uri());

    test.search_for("notes");

    wait_until("the subfolder's match", || test.names().len() == 3);
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 3.txt", "Notes 10.txt"]);
    assert!(test.window.imp().search.borrow().results().is_some());
}
