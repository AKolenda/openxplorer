// SPDX-License-Identifier: AGPL-3.0-only
//! Display styles: dates, per-folder styles, Folder Options,
//! groups and folders that expand in place.

use std::fs;

use gtk::prelude::*;
use ox_core::settings::{ColumnWidths, PreferencesUpdate, Settings};

use super::file_ops_support::{open_dialog, wait_for_no_dialog};
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{descendants, settle, wait_for_frames, wait_until, Fixture, TestWindow};
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

/// With each folder keeping its own view: the folder shown sorted by size,
/// Documents by type and the shared view by date, in Icons.
fn three_saved_views(test: &TestWindow, fixture: &Fixture) {
    use ox_core::settings::{View, ViewProperties, ViewScope};
    let sorted_by = |key: &str| ViewProperties {
        sort: key.to_owned(),
        ..ViewProperties::default()
    };
    let mut settings = Settings::open(test.settings_directory());
    settings
        .update_preferences(&PreferencesUpdate {
            per_folder_views: Some(true),
            view: Some(View::Grid),
            view_defaults: Some(ViewProperties {
                mode: "icons".to_owned(),
                ..sorted_by("modified")
            }),
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

/// The sort key `pane` shows.
fn sort_of(pane: &crate::window::folder_pane::FolderPane) -> String {
    crate::window::BrowserWindow::style_of_pane(pane).sort
}

/// Opens Folder Options from `test`'s window.
pub(in crate::window) fn open_folder_options(test: &TestWindow) -> crate::dialog::Dialog {
    test.activate("folder-options", None);
    let options = open_dialog(test);
    assert_eq!(options.title_text(), "Folder Options");
    options
}

/// The button labelled `label` in the body of `options`, such as Apply to
/// Folders.
fn options_button(options: &crate::dialog::Dialog, label: &str) -> gtk::Button {
    descendants::<gtk::Button>(options)
        .into_iter()
        .find(|button| button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("Folder Options has a {label} button"))
}

/// The question shown over `options`, once it shows.
fn question_over(options: &crate::dialog::Dialog) -> crate::dialog::Dialog {
    let find = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|window| window.downcast::<crate::dialog::Dialog>().ok())
            .find(|dialog| {
                dialog.is_visible() && dialog.transient_for().as_ref() == Some(options.upcast_ref())
            })
    };
    wait_until("the question", || find().is_some());
    find().expect("wait_until returned only once the question showed")
}

/// Presses `button` in Folder Options' Folder views box and returns the
/// question it asks.
pub(in crate::window) fn ask_from_folder_options(
    options: &crate::dialog::Dialog,
    button: &str,
) -> crate::dialog::Dialog {
    options_button(options, button).emit_clicked();
    question_over(options)
}

/// Presses `button` (OK or Cancel) in `options` and waits until Folder
/// Options is gone, so no window of it outlives the test.
pub(in crate::window) fn close_folder_options(
    test: &TestWindow,
    options: &crate::dialog::Dialog,
    button: &str,
) {
    options.press(button);
    wait_for_no_dialog(test);
}

/// Waits until the question over `options` is gone.
fn wait_for_no_question(options: &crate::dialog::Dialog) {
    let shown = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|window| window.downcast::<crate::dialog::Dialog>().ok())
            .any(|dialog| {
                dialog.is_visible() && dialog.transient_for().as_ref() == Some(options.upcast_ref())
            })
    };
    wait_until("the question to close", || !shown());
}

/// Folder Options > View > Folder views > Apply to Folders, as in Windows
/// 11: it asks first, No and closing the question change nothing, and Yes
/// gives every folder, every window and both panes of a split tab the
/// view of the folder Folder Options was opened from. Folder Options stays
/// open, as in Windows.
///
/// parity: VIEW-020
#[gtk::test]
fn apply_to_folders_gives_every_folder_and_window_this_folders_view_after_yes() {
    use crate::window::session::PaneSide;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    three_saved_views(&test, &fixture);
    test.window
        .split_tab(Some(&fixture.uri_of("Documents")))
        .expect("split");
    test.wait_for_listing("the right pane");
    test.window.activate_pane(PaneSide::Start);
    let beside = test.open_beside(&fixture.uri_of("Documents"));
    assert_eq!(sort_of(beside.window.folder_pane()), "type");
    assert_eq!(sort_of(test.window.pane_on(PaneSide::End)), "type");

    let options = open_folder_options(&test);
    assert!(options
        .texts()
        .iter()
        .any(|text| text == "You can apply this view (such as Details or Icons) to all folders."));
    let question = ask_from_folder_options(&options, "Apply to Folders");
    assert_eq!(question.title_text(), "Folder Views");
    assert_eq!(
        question.message_text(),
        "Do you want all folders to match this folder's view settings?"
    );
    assert_eq!(question.button_labels(), ["Yes", "No"]);
    question.press("No");
    wait_for_no_question(&options);
    ask_from_folder_options(&options, "Apply to Folders").close();
    wait_for_no_question(&options);
    settle();
    let unchanged = test.context.settings_data().preferences;
    assert_eq!(unchanged.folder_views.len(), 2, "No keeps every folder's view");
    assert_eq!(sort_of(beside.window.folder_pane()), "type");

    ask_from_folder_options(&options, "Apply to Folders").press("Yes");
    wait_until("one view for every folder", || {
        let saved = test.context.settings_data().preferences;
        saved.folder_views.is_empty() && saved.view_defaults.is_some_and(|style| style.sort == "size")
    });
    wait_until("every window and pane to follow", || {
        sort_of(beside.window.folder_pane()) == "size"
            && sort_of(test.window.pane_on(PaneSide::End)) == "size"
    });
    assert!(options.is_visible(), "Folder Options stays open");
    close_folder_options(&test, &options, "Cancel");
    let saved = Settings::open(test.settings_directory());
    assert!(saved.data().preferences.folder_views.is_empty(), "saved to disk");
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

/// Folder Options > View > Folder views > Reset Folders: No changes
/// nothing; Yes makes every folder, window and pane forget its view and
/// show the default one, Details sorted by name.
///
/// parity: VIEW-020
#[gtk::test]
fn reset_folders_returns_every_folder_and_window_to_the_default_view_after_yes() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    three_saved_views(&test, &fixture);
    let beside = test.open_beside(&fixture.uri_of("Documents"));
    assert_eq!(sort_of(beside.window.folder_pane()), "type");

    let options = open_folder_options(&test);
    let question = ask_from_folder_options(&options, "Reset Folders");
    assert_eq!(question.title_text(), "Folder Views");
    assert_eq!(
        question.message_text(),
        "Do you want to reset all folders to their default view settings?"
    );
    assert_eq!(question.button_labels(), ["Yes", "No"]);
    question.press("No");
    wait_for_no_question(&options);
    settle();
    let unchanged = test.context.settings_data().preferences;
    assert_eq!(unchanged.folder_views.len(), 2, "No keeps every folder's view");
    assert!(unchanged.view_defaults.is_some());

    ask_from_folder_options(&options, "Reset Folders").press("Yes");
    wait_until("every view forgotten", || {
        let saved = test.context.settings_data().preferences;
        saved.folder_views.is_empty() && saved.view_defaults.is_none()
    });
    wait_until("the default view in every window", || {
        test.action_state("sort").as_deref() == Some("name") && sort_of(beside.window.folder_pane()) == "name"
    });
    assert_eq!(test.window.folder_pane().view(), FolderView::Details);
    assert_eq!(beside.window.folder_pane().view(), FolderView::Details);
    close_folder_options(&test, &options, "OK");
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    assert_eq!(test.action_state("sort").as_deref(), Some("name"));
    assert_eq!(test.window.folder_pane().view(), FolderView::Details);
}

/// Apply to Folders needs a folder to take the view from, as in Windows:
/// opened from This PC or from search results it cannot be pressed, while
/// Reset Folders can.
///
/// parity: VIEW-020
#[gtk::test]
fn apply_to_folders_is_unavailable_without_a_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(crate::locations::Page::ThisPc.uri());
    let options = open_folder_options(&test);
    assert!(!options_button(&options, "Apply to Folders").is_sensitive());
    assert!(options_button(&options, "Reset Folders").is_sensitive());
    close_folder_options(&test, &options, "Cancel");

    test.window.navigate(&fixture.uri()).expect("a folder");
    test.wait_for_listing("the folder");
    let options = open_folder_options(&test);
    assert!(options_button(&options, "Apply to Folders").is_sensitive());
    close_folder_options(&test, &options, "Cancel");

    test.start_search_cache();
    test.window.search_box().entry().set_text("Documents");
    wait_until("the search", || test.window.is_searching());
    let options = open_folder_options(&test);
    assert!(!options_button(&options, "Apply to Folders").is_sensitive());
    close_folder_options(&test, &options, "Cancel");
}

/// Folder Options' advanced settings that Windows has and the app has
/// too: a change takes effect with Apply or OK, Cancel drops it, and
/// Restore Defaults sets the boxes back to their defaults.
///
/// parity: VIEW-020
#[gtk::test]
fn folder_options_advanced_settings_apply_with_ok_or_apply() {
    const FULL_PATH: &str = "Display the full path in the title bar";
    const CHECK_BOXES: &str = "Use check boxes to select items";
    const RESTORE: &str = "Restore previous folder windows at logon";
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let check = |options: &crate::dialog::Dialog, label: &str| {
        descendants::<gtk::CheckButton>(options)
            .into_iter()
            .find(|check| check.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("Folder Options has {label}"))
    };

    let options = open_folder_options(&test);
    assert_eq!(options.button_labels(), ["OK", "Cancel", "Apply"]);
    assert!(!options.can_press("Apply"), "nothing changed yet");
    assert!(check(&options, CHECK_BOXES).is_active());
    check(&options, FULL_PATH).set_active(true);
    assert!(options.can_press("Apply"));
    close_folder_options(&test, &options, "Cancel");
    settle();
    assert!(!test.context.settings_data().preferences.full_path_in_title);

    let options = open_folder_options(&test);
    check(&options, FULL_PATH).set_active(true);
    check(&options, RESTORE).set_active(true);
    options.press("Apply");
    wait_until("the change saved", || {
        let saved = test.context.settings_data().preferences;
        saved.full_path_in_title && saved.restore_session
    });
    assert!(options.is_visible(), "Apply keeps Folder Options open");
    assert!(!options.can_press("Apply"));
    check(&options, CHECK_BOXES).set_active(false);
    close_folder_options(&test, &options, "OK");
    wait_until("OK saves it", || {
        !test.context.settings_data().preferences.selection_marker
    });

    let options = open_folder_options(&test);
    options_button(&options, "Restore Defaults").emit_clicked();
    assert!(!check(&options, FULL_PATH).is_active());
    assert!(!check(&options, RESTORE).is_active());
    assert!(check(&options, CHECK_BOXES).is_active());
    close_folder_options(&test, &options, "OK");
    wait_until("the defaults saved", || {
        let saved = test.context.settings_data().preferences;
        !saved.full_path_in_title && !saved.restore_session && saved.selection_marker
    });
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

/// Group by "Same as sort" (Dolphin's "Show in groups") heads each group
/// of the details view with the sort key's title.
///
/// parity: VIEW-022
#[gtk::test]
fn grouping_by_the_sort_key_heads_each_group() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().model().select_only(1);
    let selected = test.selected_names();
    test.activate("group-by", Some("sort"));
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
    test.activate("group-by", Some("none"));
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
