// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's search layout, the default (SRCH-046): while a search
//! shows, the command bar offers Search options and Close search in place
//! of the search filter bar, the tab reads "<query> - Search Results in
//! <folder>", the words searched for are marked in the names, and the
//! results keep the folder's grouping. Settings > Search > "Show the
//! search filter bar" brings back the filter bar and the titles as they
//! were.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::SearchIn;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::command_bar::{command_bar_names, menu_of};
use super::search_history::{root_title, window_with_notes, RESULTS};
use crate::test_support::harness::{wait_until, Fixture, TestWindow, STANDARD_NAMES};

/// The Search options menu, as Windows 11 lists it, without what the app
/// cannot do (System files, Zipped (compressed) folders) and Change indexed
/// locations, which is Windows' own.
const SEARCH_OPTIONS: [&str; 8] = [
    "All subfolders",
    "Current folder",
    "-",
    "Date modified",
    "Kind",
    "Size",
    "-",
    "File contents",
];

/// Turns Settings > Search > "Show the search filter bar" on or off, as
/// Settings or another window would.
fn show_filter_bar(test: &TestWindow, on: bool) {
    let update = PreferencesUpdate {
        show_search_filter_bar: Some(on),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
    wait_until("the window to follow the setting", || {
        test.context.settings_data().preferences.show_search_filter_bar == on
    });
}

/// Whether the command bar shows a control named `name`.
fn shows_command(test: &TestWindow, name: &str) -> bool {
    command_bar_names(test).iter().any(|shown| shown == name)
}

/// By default a search shows Search options and Close search after View in
/// the command bar and no filter bar; the tab names the query, the address
/// bar the results, and neither button stays once the search ends.
///
/// parity: SRCH-046
#[gtk::test]
fn a_search_shows_search_options_and_close_search_and_no_filter_bar() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    assert!(!shows_command(&test, "Search options"));

    test.search_for("notes");

    assert_eq!(test.names(), RESULTS);
    assert!(!test.window.search_strip().is_visible(), "no filter bar");
    let names = command_bar_names(&test);
    let view = names.iter().position(|name| name == "View").expect("View");
    assert_eq!(
        names[view + 1..view + 3],
        ["Search options", "Close search"],
        "{names:?}"
    );
    assert_eq!(
        test.active_tab_title(),
        format!("notes - Search Results in {root}")
    );
    let title = test.window.title().map(|title| title.to_string());
    assert_eq!(
        title,
        Some(format!("notes - Search Results in {root} — OpenXplorer"))
    );
    let crumbs = test.crumb_labels();
    assert_eq!(crumbs.last(), Some(&format!("Search Results in {root}")));

    test.window.search_box().clear();

    wait_until("the folder", || test.names() == STANDARD_NAMES);
    assert!(!shows_command(&test, "Search options"));
    assert!(!shows_command(&test, "Close search"));
}

/// Search options lists Windows 11's items in its order, with the date,
/// kind and size choices in submenus.
///
/// parity: SRCH-046
#[gtk::test]
fn the_search_options_menu_lists_windows_items_in_order() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    let menu = menu_of(&test, "Search options");
    menu.popup();
    wait_until("the menu", || menu.is_visible());

    assert_eq!(menu.row_labels(), SEARCH_OPTIONS);
    assert_eq!(menu.checked_labels(), ["All subfolders"]);
    let dates = menu.open_submenu_of("Date modified").row_labels();
    assert_eq!(
        dates,
        [
            "Any date",
            "Today",
            "Yesterday",
            "This week",
            "Last week",
            "This month",
            "This year"
        ]
    );
    let kinds = menu.open_submenu_of("Kind").row_labels();
    assert_eq!(
        kinds,
        [
            "Any kind",
            "Folders",
            "Documents",
            "Images",
            "Audio files",
            "Videos"
        ]
    );
    let sizes = menu.open_submenu_of("Size").row_labels();
    assert_eq!(sizes[0], "Any size");
    assert_eq!(sizes[7], "Gigantic (> 4 GB)");
    menu.popdown();
}

/// Each item of Search options changes the results: Current folder
/// leaves the subfolders out, the date, kind and size choices narrow the
/// results, and File contents finds files by their text.
///
/// parity: SRCH-046
#[gtk::test]
fn each_search_option_changes_the_results() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("notes.png"), b"\x89PNG\r\n\x1a\n").expect("fixture file");
    fs::write(fixture.path("minutes.txt"), b"Read the notes first").expect("fixture file");
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    let menu = menu_of(&test, "Search options");
    let all = ["Notes 2.txt", "Notes 10.txt", "notes inside.txt", "notes.png"];
    assert_eq!(test.names(), all);
    let open = || {
        menu.popup();
        wait_until("the menu", || menu.is_visible());
    };

    open();
    menu.click_row("Current folder");
    wait_until("the folder's own matches", || {
        test.names() == ["Notes 2.txt", "Notes 10.txt", "notes.png"]
    });
    open();
    assert_eq!(menu.checked_labels(), ["Current folder"]);
    menu.click_row("All subfolders");
    wait_until("the subfolders again", || test.names() == all);

    open();
    menu.open_submenu_of("Kind").click_row("Images");
    wait_until("the pictures", || test.names() == ["notes.png"]);
    open();
    menu.open_submenu_of("Kind").click_row("Any kind");
    open();
    menu.open_submenu_of("Size").click_row("Empty (0 KB)");
    wait_until("no empty file", || test.names().is_empty());
    open();
    menu.open_submenu_of("Size").click_row("Any size");
    open();
    menu.open_submenu_of("Date modified").click_row("Yesterday");
    wait_until("nothing from yesterday", || test.names().is_empty());
    open();
    menu.open_submenu_of("Date modified").click_row("Today");
    wait_until("today's matches", || test.names() == all);

    open();
    menu.click_row("File contents");
    wait_until("the file found by its text", || {
        test.names().contains(&"minutes.txt".to_owned())
    });
    assert_eq!(
        test.window.imp().search.borrow().search_in(),
        SearchIn::NamesAndContents,
        "File contents is Names and contents"
    );
}

/// Close search ends the search and shows the folder again, as Back and
/// Escape do.
///
/// parity: SRCH-046
#[gtk::test]
fn close_search_returns_to_the_folder() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    test.search_for("notes");

    test.activate("close-search", None);

    wait_until("the folder", || test.names() == STANDARD_NAMES);
    assert_eq!(test.search_text(), "");
    assert_eq!(test.active_tab_title(), root);
    assert!(!shows_command(&test, "Close search"));
}

/// The words searched for are marked in the names of the results; with
/// the filter bar on, names are drawn as before.
///
/// parity: SRCH-046
#[gtk::test]
fn the_words_searched_for_are_marked_in_the_names() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    let marks = |name: &str| {
        let position = test.position_of(name);
        let cell = test
            .window
            .folder_pane()
            .owners()
            .file_cell_at(position, test.window.folder_pane().details().column_view())
            .expect("the cell is on screen");
        cell.marked_ranges()
    };

    assert_eq!(marks("Notes 2.txt"), [(0, 5)]);
    assert_eq!(marks("notes inside.txt"), [(0, 5)]);

    show_filter_bar(&test, true);
    test.search_for("notes 2");
    assert!(marks("Notes 2.txt").is_empty(), "the filter bar's look");
}

/// A grouped folder's results are grouped the same way.
///
/// parity: SRCH-046
#[gtk::test]
fn the_results_keep_the_folders_grouping() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    test.activate("group-by", Some("type"));
    let grouping = test.window.folder_model().grouping();
    assert!(grouping.is_some());

    test.search_for("notes");

    assert_eq!(test.window.folder_model().grouping(), grouping);
}

/// With Settings > Search > "Show the search filter bar" on, a search shows
/// the filter bar and the titles as before, and no search buttons in the
/// command bar; turning it off while searching swaps them back.
///
/// parity: SRCH-046
#[gtk::test]
fn the_filter_bar_setting_brings_back_the_filter_bar_and_its_titles() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    show_filter_bar(&test, true);

    test.search_for("notes");

    assert!(test.window.search_strip().is_visible());
    assert!(!shows_command(&test, "Search options"));
    assert!(!shows_command(&test, "Close search"));
    assert_eq!(test.active_tab_title(), format!("Search Results in {root}"));

    show_filter_bar(&test, false);

    wait_until("the Windows layout", || {
        !test.window.search_strip().is_visible() && shows_command(&test, "Search options")
    });
}
