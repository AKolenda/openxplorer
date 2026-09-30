// SPDX-License-Identifier: AGPL-3.0-only
//! The Settings tab in the window: opening it, what it replaces, leaving
//! it, and the layout reset it offers every window.
//!
//! The page itself is tested in `settings_page::tests`.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{Column, ColumnWidth, PreferencesUpdate, Settings};

use crate::locations::Page;
use crate::settings_page::{Category, SettingsView};
use crate::test_support::harness::{application, wait_for, wait_until, Fixture, TestWindow};
use crate::test_support::python::python_preference;
use crate::window::tests::file_ops_support::open_dialog;
use crate::window::window_action::WindowAction;

/// How long a test lets the work a tab switch queues run, such as giving
/// the file list its focus back, before it checks where focus is.
const FOCUS_SETTLE_TIME: Duration = Duration::from_millis(300);

/// A window on the standard fixture, with its folder in the first tab and
/// Settings opened with Ctrl+, in a second.
fn window_with_settings_open() -> (Fixture, TestWindow) {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("settings", None);
    (fixture, test)
}

/// The title of the tab in front.
fn active_tab_title(test: &TestWindow) -> String {
    let uri = test.window.current_uri().expect("a tab is in front");
    test.window.imp().locations.borrow().title_for(&uri)
}

/// parity: SET-001, SET-019
#[gtk::test]
fn ctrl_comma_opens_settings_as_a_tab_of_its_own() {
    let (_fixture, test) = window_with_settings_open();

    let keys = application().accels_for_action(&WindowAction::Settings.detailed_name());
    assert_eq!(keys, ["<Control>comma"]);
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(active_tab_title(&test), "Settings");
    assert!(test.window.shows_settings());
    let imp = test.window.imp();
    assert!(!imp.navigation_row.is_visible(), "no address or search box");
    assert!(!imp.command_bar.is_visible(), "no command bar");
    assert!(!imp.status_bar.is_visible(), "no status bar");
    assert!(
        !test
            .window
            .lookup_action("location")
            .is_some_and(|action| action.is_enabled()),
        "address editing is off on the Settings tab"
    );
}

/// Arrow keys, Escape and typing reach Settings at once, not the file
/// list the Settings tab hides.
///
/// parity: SET-019
#[gtk::test]
fn opening_settings_moves_keyboard_focus_into_the_page() {
    let (_fixture, test) = window_with_settings_open();

    wait_for(FOCUS_SETTLE_TIME);

    let focus = GtkWindowExt::focus(&test.window).expect("a widget has keyboard focus");
    let page = test.window.imp().settings_page.get();
    assert!(
        focus.is_ancestor(&page),
        "the focus is on a {}, outside Settings",
        focus.type_()
    );
}

/// parity: SET-019
#[gtk::test]
fn an_open_settings_tab_is_shown_again() {
    let (_fixture, test) = window_with_settings_open();
    test.activate("new-tab", None);
    test.wait_for_listing("the new tab");

    test.activate("default-file-explorer", None);

    assert_eq!(test.window.tab_count(), 3, "no second Settings tab");
    assert!(test.window.shows_settings());
    assert_eq!(
        test.window.imp().settings_page.view(),
        SettingsView::Category(Category::DefaultApps),
        "More > Default file explorer… opens Default apps"
    );
}

#[gtk::test]
fn other_tabs_show_the_browsing_area_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folder_tab = test.active_tab().expect("the folder's tab");
    test.activate("settings", None);

    test.activate_tab(folder_tab);

    assert!(!test.window.shows_settings());
    let imp = test.window.imp();
    assert!(imp.navigation_row.is_visible());
    assert!(imp.status_bar.is_visible());
    assert!(test
        .window
        .lookup_action("location")
        .is_some_and(|action| action.is_enabled()));
}

/// Settings had keyboard focus; the files it returns to take it back,
/// rather than the first sidebar row GTK would hand it to.
///
/// parity: SET-019
#[gtk::test]
fn back_to_files_gives_the_file_list_keyboard_focus_again() {
    let (_fixture, test) = window_with_settings_open();

    test.window
        .imp()
        .settings_page
        .back_to_files_button()
        .emit_clicked();
    wait_for(FOCUS_SETTLE_TIME);

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the file list has keyboard focus"
    );
}

/// parity: SET-001
#[gtk::test]
fn back_to_files_closes_the_settings_tab() {
    let (fixture, test) = window_with_settings_open();

    test.window
        .imp()
        .settings_page
        .back_to_files_button()
        .emit_clicked();

    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(!test.window.shows_settings());
}

/// Closing the only tab would close the window, so "Back to files" opens
/// the folder shown before Settings instead.
#[gtk::test]
fn back_to_files_on_the_only_tab_opens_the_folder_shown_before() {
    let (fixture, test) = window_with_settings_open();
    let settings_tab = test.active_tab();
    test.activate("previous-tab", None);
    test.activate("close-tab", None);
    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.active_tab(), settings_tab, "only Settings is left");

    test.window
        .imp()
        .settings_page
        .back_to_files_button()
        .emit_clicked();
    test.wait_for_listing("the folder");

    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.window.is_visible(), "the window stays open");
}

#[gtk::test]
fn ctrl_f_on_the_settings_tab_focuses_the_settings_search() {
    let (_fixture, test) = window_with_settings_open();

    test.activate("search", None);

    let focus = GtkWindowExt::focus(&test.window).expect("something has focus");
    let page = test.window.imp().settings_page.get();
    assert!(focus.is_ancestor(&page), "the focus is in Settings");
    assert!(
        focus.ancestor(gtk::SearchEntry::static_type()).is_some() || focus.is::<gtk::SearchEntry>(),
        "the settings search has focus"
    );
}

#[gtk::test]
fn settings_typed_in_the_address_bar_opens_a_folder_called_settings() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("Settings")).expect("a folder called Settings");
    let test = TestWindow::open(&fixture.uri());

    test.window.submit_address("Settings");

    let folder = fixture.uri_of("Settings");
    wait_until("the Settings folder", || {
        test.window.current_uri().as_ref() == Some(&folder)
    });
    assert!(!test.window.shows_settings());
    let typed_uri = test.window.resolve_address(Page::Settings.uri()).ok();
    assert_eq!(
        typed_uri.as_deref(),
        Some(Page::Settings.uri()),
        "ox:settings still opens it"
    );
}

/// Settings > Appearance > "Reset" returns every window's sidebar and
/// columns to their default widths, and saves the Python app's layout
/// (`resetLayout`: a 210 px sidebar and no column widths).
///
/// parity: SET-015, SET-005
#[gtk::test]
fn reset_returns_every_windows_sidebar_to_its_default_width() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let beside = test.open_beside(&fixture.uri());
    let widened = PreferencesUpdate {
        sidebar_width: Some(320.0),
        column_widths: Some(vec![ColumnWidth {
            column: Column::Name,
            pixels: 400.0,
        }]),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&widened)
        .expect("the test settings take a layout");
    for window in [&test.window, &beside.window] {
        window.imp().workspace.set_position(320);
    }

    test.activate("reset-layout", None);

    for window in [&test.window, &beside.window] {
        assert_eq!(window.imp().workspace.position(), 210);
    }
    let directory = test.settings_directory();
    wait_until("the default layout to be saved", || {
        let saved = Settings::open(directory).data().preferences.clone();
        saved.sidebar_width == Some(210) && saved.column_widths.is_none_or(|widths| widths.is_empty())
    });
    assert_eq!(python_preference(directory, "sidebarWidth"), "210");
    assert_eq!(python_preference(directory, "columnWidths"), "{}");
}

/// "Read license & source information" shows the copyright, where the
/// source is and the AGPL, in a dialog that fits the window and scrolls.
///
/// parity: SET-009
#[gtk::test]
fn the_license_dialog_shows_the_copyright_the_source_and_the_agpl() {
    let (_fixture, test) = window_with_settings_open();

    test.activate("license", None);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "OpenXplorer · License & source");
    let text = dialog.message_text();
    assert!(
        text.starts_with("Copyright (c) 2026 OpenXplorer contributors."),
        "{text}"
    );
    assert!(text.contains("https://github.com/AKolenda/openxplorer, tag v"));
    assert!(text.contains("GNU AFFERO GENERAL PUBLIC LICENSE"));
    wait_until("the dialog to have its size", || dialog.height() > 0);
    assert!(
        dialog.height() < test.window.height(),
        "the licence scrolls in the dialog"
    );
    dialog.press("OK");
}
