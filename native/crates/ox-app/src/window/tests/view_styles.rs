// SPDX-License-Identifier: AGPL-3.0-only
//! Display styles: dates, per-folder styles, the display style dialog,
//! groups and folders that expand in place.

use std::fs;

use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::file_ops_support::open_dialog;
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;
use crate::window::session::Direction;

/// Saves `update` as another window would and lets `test` read it.
fn change_preferences(test: &TestWindow, update: &PreferencesUpdate) {
    Settings::open(test.settings_directory())
        .update_preferences(update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
}

/// Files written just now read "Today at …"; turning relative dates off in
/// Settings writes them in full in every window at once.
///
/// parity: VIEW-004
#[gtk::test]
fn recent_dates_read_today_until_settings_turn_that_off() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let dates = || {
        test.window
            .folder_pane()
            .details()
            .cell_texts(SortColumn::Modified, 4)
    };
    assert!(
        dates().iter().all(|date| date.starts_with("Today at ")),
        "{:?}",
        dates()
    );
    change_preferences(
        &test,
        &PreferencesUpdate {
            absolute_dates: Some(true),
            ..PreferencesUpdate::default()
        },
    );
    wait_until("dates in full", || {
        dates().iter().all(|date| !date.starts_with("Today"))
    });
}

/// With "Remember each folder's view" on, a folder keeps the sort order it
/// was given, and another folder shows its own.
///
/// parity: VIEW-020
#[gtk::test]
fn each_folder_keeps_its_own_style_when_asked() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    change_preferences(
        &test,
        &PreferencesUpdate {
            per_folder_views: Some(true),
            ..PreferencesUpdate::default()
        },
    );
    wait_until("the per-folder choice", || {
        test.context.settings_data().preferences.per_folder_views
    });
    test.activate("sort", Some("size"));
    wait_until("the folder's style to be saved", || {
        !test.context.settings_data().preferences.folder_views.is_empty()
    });
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    assert_eq!(test.action_state("sort").as_deref(), Some("name"));
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder again");
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
}

/// The dialog shows the view, sorting and groups it chose, and saves them.
///
/// parity: VIEW-021
#[gtk::test]
fn the_display_style_dialog_applies_its_choices() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view-properties", None);
    let dialog = open_dialog(&test);
    let choices = descendants::<gtk::DropDown>(&dialog);
    let [mode, sort, order] = choices.as_slice() else {
        panic!("three choices: view mode, sort key and order");
    };
    mode.set_selected(1);
    sort.set_selected(3);
    order.set_selected(1);
    let groups = descendants::<gtk::CheckButton>(&dialog)
        .into_iter()
        .find(|check| check.label().as_deref() == Some("Show in groups"))
        .expect("a groups choice");
    groups.set_active(true);
    dialog.press("OK");
    wait_until("the chosen style", || {
        test.window.folder_pane().view() == FolderView::Compact
    });
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    assert_eq!(test.action_state("direction").as_deref(), Some("descending"));
    wait_until("the style to be saved", || {
        let saved = test.context.settings_data().preferences.view_defaults;
        saved.is_some_and(|style| style.groups && style.sort == "size")
    });
}

/// "Show in groups" heads each group of the details view with its title.
///
/// parity: VIEW-022
#[gtk::test]
fn show_in_groups_heads_each_group() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("groups", None);
    let details = test.window.folder_pane().details().column_view().clone();
    assert!(details.header_factory().is_some());
    wait_for_frames(&test.window, 2);
    let model = test.window.folder_pane().model();
    let first = model.item(0).expect("an item");
    assert_eq!((model.group_titles())(&first).as_deref(), Some("D"), "Documents");
    test.activate("groups", None);
    assert!(details.header_factory().is_none());
}

/// A folder expands in place beneath its row, collapses again, and Back
/// returns to it expanded.
///
/// parity: VIEW-035
#[gtk::test]
fn a_folder_expands_in_place_and_back_keeps_it_open() {
    let fixture = Fixture::standard();
    fs::write(
        fixture.path("Documents").join("inner.txt"),
        b"Synthetic test data\n",
    )
    .expect("a file");
    let test = TestWindow::open(&fixture.uri());
    let model = test.window.folder_pane().model();
    let shown = model.n_items();
    let tree = model.tree();
    let documents = tree.row(0).expect("Documents is listed first");
    tree.set_expanded(&documents, true);
    wait_until("the folder's contents", || model.n_items() == shown + 1);
    assert_eq!(model.name_at(1).as_deref(), Some("inner.txt"));
    let cell = test
        .window
        .folder_pane()
        .owners()
        .file_cell_at(1, &test.window.folder_pane().view_widget());
    wait_for_frames(&test.window, 2);
    assert!(
        cell.is_none_or(|cell| cell.expander().is_visible()),
        "the rows keep the arrow's room"
    );

    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder again");
    wait_until("the folder expanded again", || model.n_items() == shown + 1);

    let documents = tree.row(0).expect("Documents is listed first");
    tree.set_expanded(&documents, false);
    assert_eq!(model.n_items(), shown);
}
