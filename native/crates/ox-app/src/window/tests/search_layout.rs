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

/// Turns Settings > Search > "Show the search filter bar" on or off.
fn show_filter_bar(test: &TestWindow, on: bool) {
    test.show_search_filter_bar(on);
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

/// Each new search names its own words in the tab and the window title,
/// not the first search's: three searches in a row, then Back and
/// Forward between them, and the same in a split pane's tab.
///
/// parity: SRCH-046
#[gtk::test]
fn every_new_search_names_its_own_words_in_the_tab_and_title() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    let titles = |test: &TestWindow| {
        let window = test.window.title().map(|title| title.to_string());
        (test.active_tab_title(), window)
    };
    let expected = |query: &str| {
        let tab = format!("{query} - Search Results in {root}");
        let window = Some(format!("{tab} — OpenXplorer"));
        (tab, window)
    };

    // Typed on, as a user types, so the box never empties between them.
    test.search_for("notes");
    assert_eq!(titles(&test), expected("notes"));
    for query in ["notes 1", "notes 10"] {
        test.type_on_in_search_box(query);
        assert_eq!(titles(&test), expected(query), "once {query} is searched");
    }
    test.activate("back", None);
    wait_until("Back to the folder", || test.names() == STANDARD_NAMES);
    test.activate("forward", None);
    wait_until("Forward to the last search", || {
        titles(&test) == expected("notes 10")
    });

    test.activate("split-view", None);
    test.search_for("notes");
    test.type_on_in_search_box("notes 2");
    let (tab, window) = titles(&test);
    let named = "notes 2 - Search Results in ";
    assert!(tab.starts_with(named), "the split pane's tab: {tab}");
    assert!(
        window.is_some_and(|window| window.starts_with(named)),
        "the split pane's title"
    );
}

impl TestWindow {
    /// Types the rest of `query` after the search box's text, which
    /// `query` starts with, and waits until its search has run.
    fn type_on_in_search_box(&self, query: &str) {
        let entry = self.window.search_box().entry();
        let typed = entry.text().to_string();
        let rest = query
            .strip_prefix(typed.as_str())
            .expect("the query goes on from the text");
        let mut position = -1;
        entry.insert_text(rest, &mut position);
        assert_eq!(entry.text(), query);
        wait_until("the search to run", || {
            !self.status_count().starts_with("Searching")
        });
    }
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
            "Last month",
            "This year",
            "Last year"
        ]
    );
    let kinds = menu.open_submenu_of("Kind").row_labels();
    assert_eq!(
        kinds,
        ["Any kind", "Document", "Folder", "Music", "Picture", "Video"]
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
    menu.open_submenu_of("Kind").click_row("Picture");
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

    // Current folder with File contents still reads the folder's own
    // files, not its subfolders'.
    open();
    menu.click_row("Current folder");
    wait_until("the folder's own matches by name and text", || {
        test.names() == ["minutes.txt", "Notes 2.txt", "Notes 10.txt", "notes.png"]
    });
}

/// Current folder is part of the results' step: Back to the folder and
/// Forward to the results searches the folder alone again.
///
/// parity: SRCH-046, SRCH-042
#[gtk::test]
fn current_folder_is_kept_by_back_and_forward() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    let menu = menu_of(&test, "Search options");
    menu.popup();
    wait_until("the menu", || menu.is_visible());
    menu.click_row("Current folder");
    let own = ["Notes 2.txt", "Notes 10.txt"];
    wait_until("the folder's own matches", || test.names() == own);

    test.activate("back", None);
    wait_until("the folder", || test.names() == STANDARD_NAMES);
    test.activate("forward", None);

    wait_until("the folder's own matches again", || test.names() == own);
    menu.popup();
    wait_until("the menu", || menu.is_visible());
    assert_eq!(menu.checked_labels(), ["Current folder"]);
    menu.popdown();
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

/// On results, "…" offers Pin to Quick access even in a pinned folder,
/// and it saves the search to the navigation pane, as Windows 11's "…"
/// pins a search; once the search ends, "…" unpins the folder again.
///
/// parity: SRCH-046, SRCH-038
#[gtk::test]
fn pin_to_quick_access_on_results_saves_the_search() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    let pins = |test: &TestWindow| {
        let places = test.window.places();
        let pinned = places.quick_access.iter();
        pinned.filter(|place| place.uri == fixture.uri()).count()
    };
    test.activate("pin-folder", None);
    wait_until("the folder to be pinned", || pins(&test) == 1);
    let more = || menu_of(&test, "See more").row_labels();
    wait_until("Unpin in the folder", || {
        more().contains(&"Unpin from Quick access".to_owned())
    });

    test.search_for("notes");

    wait_until("Pin on the results", || {
        more().contains(&"Pin to Quick access".to_owned())
    });
    let menu = menu_of(&test, "See more");
    menu.popup();
    wait_until("the menu", || menu.is_visible());
    menu.click_row("Pin to Quick access");
    let label = format!("Search for notes in {root}");
    wait_until("the saved search row", || {
        test.window.sidebar().labels().contains(&label)
    });
    assert_eq!(pins(&test), 1, "the folder's pin is left as it was");

    test.window.search_box().clear();

    wait_until("Unpin in the folder again", || {
        more().contains(&"Unpin from Quick access".to_owned())
    });
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

/// The progress rings the tab strip shows in place of tab icons.
fn tab_progress_rings(test: &TestWindow) -> usize {
    crate::window::widget_tree::descendants::<gtk::Widget>(test.window.tab_strip())
        .into_iter()
        .filter(|widget| widget.css_name() == "progressring" && widget.is_visible())
        .count()
}

/// As in Windows 11 File Explorer (build 26200), the tab in front shows a
/// spinning progress ring in place of its icon while a search's results
/// are still coming in, and the magnifier once the search has finished.
///
/// parity: SRCH-048
#[gtk::test]
fn the_tab_shows_a_progress_ring_while_the_search_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(tab_progress_rings(&test), 0, "a listed folder shows its icon");

    test.window.search_box().entry().set_text("notes");
    assert_eq!(tab_progress_rings(&test), 1, "the search has started");

    wait_until("the search to finish", || {
        let (count, _) = test.window.status_bar().texts();
        !count.starts_with("Searching")
    });
    wait_until("the ring to give way to the magnifier", || {
        tab_progress_rings(&test) == 0
    });
    assert!(test.window.is_searching(), "the results still show");
}

/// A search that is canceled, by emptying the box, takes its ring away.
///
/// parity: SRCH-048
#[gtk::test]
fn a_canceled_search_takes_the_ring_away() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.search_box().entry().set_text("notes");
    assert_eq!(tab_progress_rings(&test), 1);

    test.window.search_box().clear();

    assert_eq!(tab_progress_rings(&test), 0);
}

/// A tab listing a folder shows the ring too, as Windows 11 shows it
/// while a folder loads, and its icon once the folder is listed.
///
/// parity: SRCH-048, VIEW-068
#[gtk::test]
fn a_tab_listing_a_folder_shows_the_ring_until_it_is_listed() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a valid address");
    assert!(test.window.is_loading(), "the new tab is still listing");
    assert_eq!(tab_progress_rings(&test), 1);

    wait_until("the listing", || !test.window.is_loading());
    wait_until("the ring to go", || tab_progress_rings(&test) == 0);
}

/// The ring turns while the desktop's animations are on and stands still,
/// at its first picture, while they are off (ACC-010).
///
/// parity: SRCH-048, ACC-010
#[gtk::test]
fn the_ring_stands_still_while_animations_are_off() {
    let settings = gtk::Settings::default().expect("a display");
    let before = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(true);
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.search_box().entry().set_text("notes");
    let ring = crate::window::widget_tree::descendants::<crate::progress_ring::ProgressRing>(
        test.window.tab_strip(),
    )
    .into_iter()
    .next()
    .expect("the tab's progress ring");
    wait_until("the ring to be shown", || ring.is_mapped());
    let first = ring.shown_frame();
    wait_until("the arc to turn", || ring.shown_frame() != first);
    let turning = ring.is_turning();

    settings.set_gtk_enable_animations(false);
    let still = !ring.is_turning() && ring.shown_frame() == 0;
    settings.set_gtk_enable_animations(before);

    assert!(turning, "the ring turns with animations on");
    assert!(still, "the ring stands still with animations off");
}
