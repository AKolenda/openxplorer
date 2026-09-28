// SPDX-License-Identifier: AGPL-3.0-only
//! Views, sorting, the status-bar buttons, saved preferences, text size
//! and appearance.

use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::folder_view::column_titles;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::test_support::harness::{
    application, descendants, skin, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::text_size::Step;
use crate::theme::Appearance;
use crate::window::content::FolderView;

use super::geometry::{pixels, Bounds};

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

/// The sorted column shows the current app's chevron, up while
/// ascending, and no other column shows an arrow (`renderColumns`).
/// parity: VIEW-014
#[gtk::test]
fn only_the_sorted_column_shows_the_apps_arrow() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = &test.window.content().details;
    assert_eq!(
        column_titles::shown_carets(details),
        [Some(SortDirection::Ascending), None, None, None]
    );
    test.activate("sort", Some("size"));
    test.activate("direction", Some("descending"));
    assert_eq!(
        column_titles::shown_carets(details),
        [None, None, None, Some(SortDirection::Descending)]
    );
}

/// parity: VIEW-005, VIEW-006
#[gtk::test]
fn switching_views_keeps_the_selection_and_shows_the_active_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let chrome = test.window.chrome();
    assert_eq!(chrome.status.active_view_buttons(), ["Details view"]);
    test.window.folder_model().select_only(1);
    test.activate("view", Some("large"));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(chrome.status.active_view_buttons(), ["Large icons"]);
    test.activate("view", Some("small"));
    assert_eq!(
        chrome.status.active_view_buttons(),
        ["Large icons"],
        "every icon size is the icon view"
    );
    test.activate("view", Some("details"));
    assert_eq!(chrome.status.active_view_buttons(), ["Details view"]);
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

#[gtk::test]
fn a_dragged_sidebar_stops_where_the_folder_pane_keeps_its_room() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let workspace = &test.window.chrome().workspace;
    assert_eq!(workspace.position(), 210, "a new sidebar is 210 pixels wide");
    workspace.set_position(5000);
    let details = &test.window.details_pane().root;
    let details_width = if details.is_visible() { details.width() } else { 0 };
    let room_left = workspace.width() - workspace.position() - details_width;
    assert!(workspace.position() <= 560);
    assert!(
        room_left >= 300,
        "the folder pane keeps 300 pixels, not {room_left}"
    );
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

/// Ported from `desktop/tests/text_size.test.cjs` (unmodified typing
/// ignored, `AltGraph` ignored and Alt ignored): GTK matches an accelerator
/// only with exactly its modifiers, and every text-size accelerator holds
/// Ctrl alone, so plain, Alt and `AltGr` presses never resize text. (The web
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
    assert_eq!(skin.text_size().percent(), 200);
    for _ in 0..10 {
        test.activate("text-smaller", None);
    }
    assert_eq!(skin.text_size().percent(), 80);
    test.activate("text-reset", None);
    assert_eq!(skin.text_size().percent(), 100);
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
    let button = second.window.chrome().commands.appearance_tooltip();
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

/// Where the icon view's tiles are, relative to the view's scroller.
fn tile_bounds(test: &TestWindow) -> (i32, Vec<Bounds>) {
    let grid = &test.window.content().grid;
    let scroll = grid.parent().expect("the icon view scrolls");
    let tiles = descendants::<FileCell>(grid);
    let bounds = tiles
        .iter()
        .filter_map(WidgetExt::parent)
        .filter_map(|tile| tile.compute_bounds(&scroll))
        .map(|rect| {
            (
                pixels(rect.x()),
                pixels(rect.y()),
                pixels(rect.width()),
                pixels(rect.height()),
            )
        })
        .collect();
    (scroll.width(), bounds)
}

/// A window that opened in the icon view used to keep one column, because
/// GTK ignored the column count set while it allocated the view. Tiles sit
/// where `renderRows` in app.js puts them: `floor(width / 135)` columns
/// sharing the width less 20 pixels, the first 10 pixels in and 5 down,
/// 4 pixels apart, 128 pixels tall on a 130-pixel pitch.
///
/// parity: VIEW-005
#[gtk::test]
fn a_window_that_opens_in_the_icon_view_lays_tiles_out_as_render_rows() {
    let fixture = Fixture::with_files(12);
    let test = TestWindow::without_tabs();
    test.window.show_view(FolderView::Icons(IconSize::Large));
    test.show(&fixture.uri());
    wait_for_frames(&test.window, 4);
    let (width, tiles) = tile_bounds(&test);
    let columns = width / 135;
    assert!(columns >= 2, "a {width}-pixel pane holds several columns");
    let tile_width = (width - 20) / columns - 4;
    assert_eq!(tiles[0], (10, 5, tile_width, 128), "the first tile");
    let first_row: Vec<_> = tiles.iter().filter(|tile| tile.1 == 5).collect();
    assert_eq!(first_row.len(), usize::try_from(columns).expect("a few columns"));
    let next_row = tiles.iter().find(|tile| tile.1 != 5).expect("a second row");
    assert_eq!(next_row.1, 5 + 130, "rows are 130 pixels apart");
    let first_cell = descendants::<FileCell>(&test.window.content().grid)
        .into_iter()
        .next()
        .expect("a tile");
    let name = descendants::<gtk::Label>(&first_cell)
        .into_iter()
        .next()
        .expect("a name");
    let name_top = name.compute_bounds(&first_cell).map(|rect| pixels(rect.y()));
    assert_eq!(name_top, Some(56 + 8), "the name starts 8 pixels under the icon");
}
