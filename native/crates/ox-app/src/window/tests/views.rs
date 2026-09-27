// SPDX-License-Identifier: AGPL-3.0-only
//! Views, sorting, the status-bar buttons, saved preferences, text size
//! and appearance.

use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{
    application, descendants, skin, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::text_size::Step;
use crate::theme::Appearance;
use crate::window::content::FolderView;

/// The sort direction each details header shows: `ascending`,
/// `descending` or `unsorted`, in column order.
fn header_arrows(test: &TestWindow) -> Vec<String> {
    let details = &test.window.content().details;
    let indicators = descendants::<gtk::Widget>(details)
        .into_iter()
        .filter(|widget| widget.css_name() == "sort-indicator");
    let direction_of = |indicator: gtk::Widget| {
        ["ascending", "descending"]
            .into_iter()
            .find(|direction| indicator.has_css_class(direction))
            .unwrap_or("unsorted")
            .to_owned()
    };
    indicators.map(direction_of).collect()
}

/// parity: VIEW-014
#[gtk::test]
fn sorting_by_a_header_updates_the_sort_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = &test.window.content().details;
    let modified = crate::folder_view::details::view_column(details, SortColumn::Modified)
        .expect("a Date modified column");
    details.sort_by_column(Some(&modified), gtk::SortType::Descending);
    assert_eq!(test.action_state("sort").as_deref(), Some("modified"));
    assert_eq!(test.action_state("direction").as_deref(), Some("descending"));
}

/// parity: VIEW-013
#[gtk::test]
fn the_sort_menu_leaves_one_arrow_on_the_sorted_column() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("sort", Some("modified"));
    assert_eq!(
        header_arrows(&test),
        ["unsorted", "ascending", "unsorted", "unsorted"]
    );
    test.activate("direction", Some("descending"));
    assert_eq!(
        header_arrows(&test),
        ["unsorted", "descending", "unsorted", "unsorted"]
    );
    test.activate("sort", Some("name"));
    assert_eq!(
        header_arrows(&test),
        ["descending", "unsorted", "unsorted", "unsorted"]
    );
}

/// parity: VIEW-005, VIEW-006
#[gtk::test]
fn switching_views_keeps_the_selection_and_shows_the_active_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let chrome = test.window.chrome();
    assert_eq!(chrome.active_view_buttons(), ["Details view"]);
    test.window.folder_model().select_only(1);
    test.activate("view", Some("large"));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(chrome.active_view_buttons(), ["Large icons"]);
    test.activate("view", Some("small"));
    assert_eq!(
        chrome.active_view_buttons(),
        ["Large icons"],
        "every icon size is the icon view"
    );
    test.activate("view", Some("details"));
    assert_eq!(chrome.active_view_buttons(), ["Details view"]);
}

#[gtk::test]
fn the_details_button_shows_whether_the_pane_is_open() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let toggle = descendants::<gtk::ToggleButton>(&test.window)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.details-pane"))
        .expect("a Details toggle in the command bar");
    let pane = &test.window.details_pane().root;
    assert_eq!(toggle.is_active(), pane.is_visible());
    test.activate("details-pane", None);
    assert_eq!(toggle.is_active(), pane.is_visible());
    test.activate("details-pane", None);
    assert_eq!(toggle.is_active(), pane.is_visible());
}

#[gtk::test]
fn both_views_show_each_name_with_its_icon() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for view in ["details", "large"] {
        test.activate("view", Some(view));
        let view_widget = test.window.content().view_widget();
        wait_until("the cells to be bound", || {
            !descendants::<FileCell>(&view_widget).is_empty()
        });
        let cells = descendants::<FileCell>(&view_widget);
        let notes = cells.iter().find(|cell| cell.name() == "Notes 2.txt");
        let notes = notes.unwrap_or_else(|| panic!("the {view} view shows Notes 2.txt"));
        assert!(notes.has_art(), "the {view} view draws the item's icon");
    }
}

/// parity: SET-015, SET-016
#[gtk::test]
fn changed_view_preferences_are_saved_for_new_windows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("large"));
    test.activate("hidden", None);
    wait_until("the preferences to be saved", || {
        let preferences = test.context.settings_data().preferences;
        preferences.view == "grid" && preferences.show_hidden
    });
    let second = test.open_beside(&fixture.uri());
    let content = second.window.content();
    assert_eq!(content.view(), FolderView::Icons(IconSize::Large));
    assert_eq!(second.action_state("view").as_deref(), Some("large"));
    assert!(second.names().contains(&".private".to_owned()));
}

/// Ported from desktop/tests/text_size.test.cjs::unmodified typing ignored,
/// ::AltGraph ignored and ::Alt ignored: GTK matches an accelerator only
/// with exactly its modifiers, and every text-size accelerator holds Ctrl
/// alone, so plain, Alt and AltGr presses never resize text. (The web
/// app's "composing ignored" case is the input method's: it consumes keys
/// before accelerators see them.)
///
/// parity: VIEW-043
#[gtk::test]
fn the_text_size_shortcuts_are_the_table_with_ctrl_alone() {
    let app = application();
    let parse = |accelerator: &str| gtk::accelerator_parse(accelerator).expect("a valid accelerator");
    for step in Step::ALL {
        let installed = app.accels_for_action(&format!("win.{}", step.action_name()));
        let installed: Vec<_> = installed.iter().map(|accelerator| parse(accelerator)).collect();
        let expected: Vec<_> = step
            .accelerators()
            .iter()
            .map(|accelerator| parse(accelerator))
            .collect();
        assert_eq!(installed, expected, "{step:?}");
        for (key, modifiers) in installed {
            assert_eq!(modifiers, gtk::gdk::ModifierType::CONTROL_MASK, "{key:?}");
        }
    }
    let underscore = (gtk::gdk::Key::underscore, gtk::gdk::ModifierType::CONTROL_MASK);
    assert!(Step::Decrease
        .accelerators()
        .iter()
        .any(|accelerator| parse(accelerator) == underscore));
    let keypad_insert = (gtk::gdk::Key::KP_Insert, gtk::gdk::ModifierType::CONTROL_MASK);
    assert!(Step::Reset
        .accelerators()
        .iter()
        .any(|accelerator| parse(accelerator) == keypad_insert));
}

/// parity: VIEW-044
#[gtk::test]
fn text_size_steps_stay_between_80_and_200_percent() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let skin = skin();
    let before = skin.text_size();
    for _ in 0..10 {
        test.activate("text-larger", None);
    }
    assert_eq!(skin.text_size(), 200);
    for _ in 0..10 {
        test.activate("text-smaller", None);
    }
    assert_eq!(skin.text_size(), 80);
    test.activate("text-reset", None);
    assert_eq!(skin.text_size(), 100);
    skin.set_text_size(before);
}

/// parity: LOOK-003
#[gtk::test]
fn a_theme_chosen_in_one_window_reaches_every_window() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let first = TestWindow::open(&fixture.uri());
    let second = first.open_beside(&fixture.uri_of("Documents"));
    first.activate("theme", Some("dark"));
    assert_eq!(skin().appearance(), Appearance::Dark);
    assert_eq!(second.action_state("theme").as_deref(), Some("dark"));
    let button = second.window.chrome().appearance_tooltip();
    assert_eq!(button.as_deref(), Some("Appearance: dark. Click to change."));
}

/// parity: TAB-050
#[gtk::test]
fn closing_a_window_disconnects_it_from_the_shared_skin() {
    let fixture = Fixture::standard();
    let listeners = skin().listener_count();
    let first = TestWindow::open(&fixture.uri());
    let second = first.open_beside(&fixture.uri());
    assert_eq!(skin().listener_count(), listeners + 2);
    drop(second);
    assert_eq!(
        skin().listener_count(),
        listeners + 1,
        "the closed window left no listener"
    );
    drop(first);
    assert_eq!(skin().listener_count(), listeners);
}
