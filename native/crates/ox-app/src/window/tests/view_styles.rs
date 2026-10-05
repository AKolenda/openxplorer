// SPDX-License-Identifier: AGPL-3.0-only
//! Display styles: dates, per-folder styles, the display style dialog,
//! groups and folders that expand in place.

use std::fs;

use gtk::prelude::*;
use ox_core::settings::{ColumnWidths, PreferencesUpdate, Settings};

use super::file_ops_support::{open_dialog, wait_for_no_dialog};
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
/// parity: VIEW-020, VIEW-032
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
    let mut style = test.window.current_style();
    style.show_previews = Some(false);
    style.details_columns = Some(vec!["size".to_owned(), "owner".to_owned()]);
    style.column_widths = Some(ColumnWidths {
        name: Some(320),
        size: Some(110),
        ..ColumnWidths::default()
    });
    test.window.apply_style(&style);
    test.window.remember_style();
    wait_until("the folder's style to be saved", || {
        test.context
            .settings_data()
            .preferences
            .view_for(&fixture.uri())
            .show_previews
            == Some(false)
    });
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    assert_eq!(test.action_state("sort").as_deref(), Some("name"));
    assert!(test.window.folder_pane().previews_enabled());
    assert_eq!(
        test.window.folder_pane().details().chosen_columns(),
        [SortColumn::Modified, SortColumn::Type, SortColumn::Size]
    );
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder again");
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    let pane = test.window.folder_pane();
    assert!(!pane.previews_enabled());
    assert!(!pane.owners().previews().shown);
    assert_eq!(
        pane.details().chosen_columns(),
        [SortColumn::Size, SortColumn::Owner]
    );
    assert_eq!(
        ColumnWidths::from_values(&pane.details().widths_to_save()).name,
        Some(320)
    );
    let saved = Settings::open(test.settings_directory());
    assert_eq!(
        saved.data().preferences.view_for(&fixture.uri()).details_columns,
        style.details_columns
    );
    test.window.reset_layout();
    wait_until("per-folder widths reset", || {
        test.context
            .settings_data()
            .preferences
            .view_for(&fixture.uri())
            .column_widths
            == Some(ColumnWidths::default())
    });
    assert_eq!(
        ColumnWidths::from_values(&pane.details().widths_to_save()).name,
        None
    );
}

/// The dialog shows the view, sorting and groups it chose, and saves them.
///
/// parity: VIEW-017, VIEW-021
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
    let checks = descendants::<gtk::CheckButton>(&dialog);
    checks
        .iter()
        .find(|check| check.label().as_deref() == Some("Show previews"))
        .expect("previews")
        .set_active(false);
    checks
        .iter()
        .find(|check| check.label().as_deref() == Some("Show hidden items last"))
        .expect("hidden last")
        .set_active(true);
    checks
        .iter()
        .find(|check| check.label().as_deref() == Some("Owner"))
        .expect("owner column")
        .set_active(true);
    dialog.press("OK");
    wait_until("the chosen style", || {
        test.window.folder_pane().view() == FolderView::Compact
    });
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    assert_eq!(test.action_state("direction").as_deref(), Some("descending"));
    wait_until("the style to be saved", || {
        let saved = test.context.settings_data().preferences.view_defaults;
        saved.is_some_and(|style| {
            style.groups
                && style.sort == "size"
                && style.hidden_last
                && style.show_previews == Some(false)
                && style
                    .details_columns
                    .is_some_and(|columns| columns.contains(&"owner".to_owned()))
        })
    });
}

/// With each folder keeping its own view: the folder shown sorted by size,
/// Documents by type and the shared view by date.
fn three_saved_views(test: &TestWindow, fixture: &Fixture) {
    use ox_core::settings::{ViewProperties, ViewScope};
    let sorted_by = |key: &str| ViewProperties {
        sort: key.to_owned(),
        ..ViewProperties::default()
    };
    let mut settings = Settings::open(test.settings_directory());
    settings
        .update_preferences(&PreferencesUpdate {
            per_folder_views: Some(true),
            view_defaults: Some(sorted_by("modified")),
            ..PreferencesUpdate::default()
        })
        .expect("the settings file takes the change");
    settings
        .remember_view(&fixture.uri(), sorted_by("size"), ViewScope::Folder)
        .expect("a folder's view");
    settings
        .remember_view(&fixture.uri_of("Documents"), sorted_by("type"), ViewScope::Folder)
        .expect("a folder's view");
    test.context.reload_settings();
    wait_until("the saved views", || {
        test.context.settings_data().preferences.folder_views.len() == 2
    });
    test.window.follow_folder_style(&fixture.uri());
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
}

/// Opens the display style dialog, presses `button` and returns the
/// question it asks.
fn ask_from_the_display_style_dialog(test: &TestWindow, button: &str) -> crate::dialog::Dialog {
    test.activate("view-properties", None);
    let dialog = open_dialog(test);
    dialog.press(button);
    wait_until("the question", || {
        open_dialog(test).title_text() != dialog.title_text()
    });
    open_dialog(test)
}

/// Folder views > Apply to all folders, as in Windows' Folder Options:
/// after asking, every folder shows this folder's view and keeps none of
/// its own. Cancel changes nothing.
///
/// parity: VIEW-020
#[gtk::test]
fn apply_to_all_folders_shows_this_folders_view_everywhere_after_asking() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    three_saved_views(&test, &fixture);

    let question = ask_from_the_display_style_dialog(&test, "Apply to all folders");
    assert_eq!(question.title_text(), "Apply this view to all folders?");
    assert_eq!(question.button_labels(), ["Cancel", "Apply"]);
    question.press("Cancel");
    wait_for_no_dialog(&test);
    let unchanged = test.context.settings_data().preferences;
    assert_eq!(
        unchanged.folder_views.len(),
        2,
        "Cancel keeps every folder's view"
    );

    ask_from_the_display_style_dialog(&test, "Apply to all folders").press("Apply");
    wait_until("one view for every folder", || {
        let saved = test.context.settings_data().preferences;
        saved.folder_views.is_empty() && saved.view_defaults.is_some_and(|style| style.sort == "size")
    });
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");

    assert_eq!(
        test.action_state("sort").as_deref(),
        Some("size"),
        "Documents shows the applied view, not its own"
    );
}

/// Folder views > Reset folders: after asking, every folder forgets its
/// view and the shared one, and the window shows the default at once.
/// Cancel changes nothing.
///
/// parity: VIEW-020
#[gtk::test]
fn reset_folders_shows_the_default_view_everywhere_after_asking() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    three_saved_views(&test, &fixture);

    let question = ask_from_the_display_style_dialog(&test, "Reset folders");
    assert_eq!(question.title_text(), "Reset all folders to the default view?");
    assert_eq!(question.button_labels(), ["Cancel", "Reset"]);
    question.press("Cancel");
    wait_for_no_dialog(&test);
    let unchanged = test.context.settings_data().preferences;
    assert_eq!(
        unchanged.folder_views.len(),
        2,
        "Cancel keeps every folder's view"
    );
    assert!(unchanged.view_defaults.is_some());

    ask_from_the_display_style_dialog(&test, "Reset folders").press("Reset");
    wait_until("every view forgotten", || {
        let saved = test.context.settings_data().preferences;
        saved.folder_views.is_empty() && saved.view_defaults.is_none()
    });
    wait_until("the default view shown", || {
        test.action_state("sort").as_deref() == Some("name")
    });
    let saved = Settings::open(test.settings_directory());
    assert!(saved.data().preferences.folder_views.is_empty(), "saved to disk");
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    assert_eq!(test.action_state("sort").as_deref(), Some("name"));
}

/// Saved styles and header callbacks belong to their respective split
/// panes, including the inactive pane's preview policy and column widths.
///
/// parity: VIEW-020, VIEW-059
#[gtk::test]
fn split_panes_keep_their_own_display_styles() {
    use crate::folder_view::sorting::{SortDirection, SortOrder};
    use crate::window::session::PaneSide;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    change_preferences(
        &test,
        &PreferencesUpdate {
            per_folder_views: Some(true),
            ..PreferencesUpdate::default()
        },
    );
    wait_until("per-folder styles", || {
        test.context.settings_data().preferences.per_folder_views
    });
    let mut style = test.window.current_style();
    style.sort = "size".to_owned();
    style.show_previews = Some(false);
    style.details_columns = Some(vec!["size".to_owned()]);
    test.window.apply_style(&style);
    test.window.remember_style();
    wait_until("the left style saved", || {
        test.context
            .settings_data()
            .preferences
            .view_for(&fixture.uri())
            .show_previews
            == Some(false)
    });
    test.window
        .split_tab(Some(&fixture.uri_of("Documents")))
        .expect("split");
    test.wait_for_listing("right folder");
    let left = test.window.pane_on(PaneSide::Start);
    let right = test.window.pane_on(PaneSide::End);
    assert!(!left.owners().previews().shown);
    assert!(right.owners().previews().shown);
    assert_eq!(left.details().chosen_columns(), [SortColumn::Size]);
    right.details().sort_by(SortOrder {
        column: SortColumn::Modified,
        direction: SortDirection::Descending,
    });
    wait_until("right header saved", || {
        test.context
            .settings_data()
            .preferences
            .view_for(&fixture.uri_of("Documents"))
            .sort
            == "modified"
    });
    assert_eq!(test.action_state("sort").as_deref(), Some("modified"));
    assert_eq!(
        test.context
            .settings_data()
            .preferences
            .view_for(&fixture.uri())
            .sort,
        "size"
    );
    test.window.activate_pane(PaneSide::Start);
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    assert_eq!(right.details().sort_order().column, SortColumn::Modified);
}

/// "Show in groups" heads each group of the details view with its title.
///
/// parity: VIEW-022
#[gtk::test]
fn show_in_groups_heads_each_group() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().model().select_only(1);
    let selected = test.selected_names();
    test.activate("groups", None);
    let details = test.window.folder_pane().details().column_view().clone();
    assert!(details.header_factory().is_some());
    assert_eq!(
        test.selected_names(),
        selected,
        "grouping preserves the selection"
    );
    wait_for_frames(&test.window, 2);
    let model = test.window.folder_pane().model();
    let first = model.item(0).expect("an item");
    assert_eq!((model.group_titles())(&first).as_deref(), Some("D"), "Documents");
    test.activate("groups", None);
    assert!(details.header_factory().is_none());
    assert_eq!(test.selected_names(), selected);
}

/// The compact view fills a column downwards before starting the next,
/// and its overflow uses the horizontal adjustment.
///
/// parity: VIEW-008
#[gtk::test]
fn the_compact_view_places_items_down_columns() {
    let fixture = Fixture::with_files(150);
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("compact"));
    wait_for_frames(&test.window, 4);
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    let bounds = |position| {
        pane.owners()
            .file_cell_at(position, &view)
            .and_then(|cell| cell.compute_bounds(&view))
            .expect("a realized compact item")
    };
    let first = bounds(0);
    let next = bounds(1);
    assert!((first.x() - next.x()).abs() < 1.0);
    assert!(next.y() > first.y());
    let adjustment = pane.icon_view().scroll_adjustment();
    wait_until("horizontal overflow", || {
        adjustment.upper() > adjustment.page_size()
    });
    adjustment.set_value(100.0);
    assert!(adjustment.value() > 0.0);
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
    // GTK can deliver a queued search notification after the listing even
    // though the empty entry still shows the same folder.
    test.window
        .search_box()
        .entry()
        .emit_by_name::<()>("search-changed", &[]);
    assert_eq!(
        model.n_items(),
        shown + 1,
        "an unchanged search keeps the branch open"
    );
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
    test.window
        .search_box()
        .entry()
        .emit_by_name::<()>("search-changed", &[]);
    assert_eq!(model.n_items(), shown + 1, "the restored branch stays open");

    let documents = tree.row(0).expect("Documents is listed first");
    tree.set_expanded(&documents, false);
    assert_eq!(model.n_items(), shown);
}
