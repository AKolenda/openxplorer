// SPDX-License-Identifier: AGPL-3.0-only
//! A page the window draws itself, such as Home or This PC, lists no
//! files, so nothing hidden behind it can be selected: Ctrl+A there never
//! selects the folder shown before, and Cut, Copy, Delete, Rename and
//! Properties then have nothing to act on, as in Windows. Every name below
//! is fictional.

use gtk::gdk;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::file_ops_support::{is_enabled, press_shortcut_where_focused, select_names};
use crate::test_support::harness::{wait_until, Fixture, TestWindow, STANDARD_NAMES};

/// The commands that act on the selected items.
const ITEM_COMMANDS: [&str; 5] = ["cut", "copy", "trash", "rename", "properties"];

/// Sets "Open Quick access on Home" for `test`'s settings.
fn open_quick_access_on_home(test: &TestWindow, on: bool) {
    let update = PreferencesUpdate {
        home_quick_access: Some(on),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the setting to be read", || {
        test.context.settings_data().preferences.home_quick_access == on
    });
}

/// Presses Ctrl+A where the keyboard is, as the window's own key does,
/// and runs Select all from the command bar's menu too.
fn select_all_both_ways(test: &TestWindow) {
    press_shortcut_where_focused(test, gdk::Key::a, gdk::ModifierType::CONTROL_MASK);
    test.activate("select-all", None);
}

/// Asserts that `test`'s page hides no listed item, nor a selected one,
/// and, on Home, whose tiles and rows are not selected by Ctrl+A, that
/// the item commands have nothing to act on. This PC's commands act on
/// its own tiles, which it shows.
fn assert_nothing_hidden_is_selected(test: &TestWindow, page: &str) {
    let model = test.window.folder_model();
    let names: Vec<String> = model
        .selected_items()
        .iter()
        .map(|item| item.entry().name.clone())
        .collect();
    assert!(names.is_empty(), "{page} selects nothing hidden: {names:?}");
    assert_eq!(model.n_items(), 0, "{page} keeps no hidden rows");
    if page.starts_with("Home") {
        for command in ITEM_COMMANDS {
            assert!(
                !is_enabled(test, command),
                "{page}: {command} has nothing to act on"
            );
        }
    }
}

/// On Windows 11's Home (Quick access on Home) and on This PC, Ctrl+A and
/// Select all never select the folder shown before, which the page hides,
/// and Delete recycles nothing; the selection the folder had is gone too.
/// With the option off, Home is the home folder, and Ctrl+A selects what
/// it lists.
///
/// parity: SEL-004, HOME-012
#[gtk::test]
fn ctrl_a_on_a_page_never_selects_the_hidden_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    for (page, address) in [("Home", "home"), ("This PC", "pc:")] {
        test.show(&fixture.uri());
        select_names(&test, &["Notes 2.txt"]);
        if address == "home" {
            test.activate("home", None);
        } else {
            test.activate("go-to", Some(address));
        }
        wait_until(&format!("{page} to show"), || {
            test.window
                .current_uri()
                .is_some_and(|uri| !uri.starts_with("file:"))
        });
        assert_nothing_hidden_is_selected(&test, &format!("{page}, on arrival"));

        test.window.folder_pane().focus_view();
        select_all_both_ways(&test);
        assert_nothing_hidden_is_selected(&test, &format!("{page}, after Ctrl+A"));
        test.activate("trash", None);
        test.activate("cut", None);
        for name in STANDARD_NAMES {
            assert!(fixture.path(name).exists(), "{page}: {name} is still there");
        }
    }

    open_quick_access_on_home(&test, false);
    test.activate("home", None);
    test.wait_for_listing("the home folder");
    test.window.folder_pane().focus_view();
    select_all_both_ways(&test);
    let mut shown = test.names();
    let mut selected = test.selected_names();
    shown.sort();
    selected.sort();
    assert_eq!(selected, shown, "the home folder selects what it lists");
}

/// The Settings page selects nothing hidden either, and search results
/// selected before Close search leave nothing selected that the folder
/// does not show.
///
/// parity: SEL-004, SRCH-046
#[gtk::test]
fn settings_and_closed_search_results_keep_no_hidden_selection() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("Documents/notes inside.txt"), b"x").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());

    test.search_for("notes");
    wait_until("the result inside Documents", || {
        test.names().contains(&"notes inside.txt".to_owned())
    });
    test.activate("select-all", None);
    test.activate("close-search", None);
    wait_until("the folder", || test.names() == STANDARD_NAMES);
    let shown = test.names();
    let hidden: Vec<String> = test
        .selected_names()
        .into_iter()
        .filter(|name| !shown.contains(name))
        .collect();
    assert!(hidden.is_empty(), "no hidden result stays selected: {hidden:?}");

    select_names(&test, &["Notes 2.txt"]);
    test.activate("settings", None);
    wait_until("Settings", || test.window.shows_settings());
    test.activate("select-all", None);
    let selected = test.window.folder_model().selected_items();
    assert!(
        selected.iter().all(|item| item.entry().name != "Notes 10.txt"),
        "Settings selects nothing of the folder behind it"
    );
}
