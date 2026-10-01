// SPDX-License-Identifier: AGPL-3.0-only
//! Views, sorting, the status-bar buttons, saved preferences, text size
//! and appearance.

use gtk::prelude::*;
use ox_core::settings::{Appearance, Theme, View};

use crate::folder_view::cells::FileCell;
use crate::folder_view::column_titles;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::icons::{Art, FileType};
use crate::test_support::harness::{
    application, descendants, skin, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::text_size::Step;
use crate::theme::Skin;
use crate::window::folder_pane::FolderView;
use crate::window::widget_tree::children;
use crate::window::WindowAction;

use super::file_ops_support::press_shortcut_where_focused;
use super::geometry::{pixels, Bounds};

/// The sort direction each shown details header shows: `ascending`,
/// `descending` or `unsorted`, in column order. The Folder path title of
/// a search is left out while it is hidden.
fn header_arrows(test: &TestWindow) -> Vec<String> {
    let details = test.window.folder_pane().details().column_view();
    let header = children(details)
        .find(|child| child.css_name() == "header")
        .expect("the details view has a header");
    let shown_titles = children(&header).filter(WidgetExt::is_visible);
    let indicators = shown_titles.filter_map(|title| {
        let parts = descendants::<gtk::Widget>(&title).into_iter();
        parts.into_iter().find(|part| part.css_name() == "sort-indicator")
    });
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
    let details = test.window.folder_pane().details();
    let modified = details
        .column(SortColumn::Modified)
        .expect("a Date modified column");
    details
        .column_view()
        .sort_by_column(Some(&modified), gtk::SortType::Descending);
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
    let details = test.window.folder_pane().details().column_view();
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
    let status_bar = test.window.status_bar();
    assert_eq!(status_bar.active_view_buttons(), ["Details view"]);
    test.window.folder_model().select_only(1);
    test.activate("view", Some("large"));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(status_bar.active_view_buttons(), ["Large icons"]);
    test.activate("view", Some("small"));
    assert_eq!(
        status_bar.active_view_buttons(),
        ["Large icons"],
        "every icon size is the icon view"
    );
    test.activate("view", Some("details"));
    assert_eq!(status_bar.active_view_buttons(), ["Details view"]);
}

/// The command-bar toggle, View › Details pane and the pane's own close
/// button all run `win.details-pane`, which is saved.
///
/// parity: VIEW-027
#[gtk::test]
fn the_details_button_shows_whether_the_pane_is_open() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let toggle = descendants::<gtk::ToggleButton>(&test.window)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.details-pane"))
        .expect("a Details toggle in the command bar");
    let pane = test.window.details_pane();
    assert_eq!(toggle.is_active(), pane.is_visible());
    test.activate("details-pane", None);
    assert_eq!(toggle.is_active(), pane.is_visible());
    let shown = pane.is_visible();
    wait_until("the pane choice to be saved", || {
        test.context.settings_data().preferences.show_details_pane == shown
    });
    test.activate("details-pane", None);
    assert_eq!(toggle.is_active(), pane.is_visible());
    let runs_the_toggle = descendants::<gtk::Button>(pane)
        .into_iter()
        .any(|button| button.action_name().as_deref() == Some("win.details-pane"));
    assert!(runs_the_toggle, "the pane's close button runs the same toggle");
}

#[gtk::test]
fn both_views_show_each_name_with_its_icon() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for view in ["details", "large"] {
        test.activate("view", Some(view));
        let view_widget = test.window.folder_pane().view_widget();
        wait_until("the cells to be bound", || {
            !descendants::<FileCell>(&view_widget).is_empty()
        });
        let cells = descendants::<FileCell>(&view_widget);
        let notes = cells.iter().find(|cell| cell.name() == "Notes 2.txt");
        let notes = notes.unwrap_or_else(|| panic!("the {view} view shows Notes 2.txt"));
        let text_icon = Some(Art::File(FileType::Text));
        assert_eq!(notes.art(), text_icon, "the {view} view shows the item's icon");
    }
}

/// A real symbolic link carries the link emblem and a file without write
/// permission the lock, as GIO lists them, in both views; other items
/// carry none.
///
/// parity: LOOK-017
#[gtk::test]
fn listed_links_and_read_only_files_carry_their_emblems() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let fixture = Fixture::standard();
    symlink(fixture.path("Notes 2.txt"), fixture.path("Shortcut.txt")).expect("make a link");
    fixture.write("Locked.txt");
    let locked = std::fs::Permissions::from_mode(0o444);
    std::fs::set_permissions(fixture.path("Locked.txt"), locked).expect("make it read-only");
    let test = TestWindow::open(&fixture.uri());
    for view in ["details", "large"] {
        test.activate("view", Some(view));
        let view_widget = test.window.folder_pane().view_widget();
        wait_until("the cells to be bound", || {
            descendants::<FileCell>(&view_widget)
                .iter()
                .any(|cell| cell.name() == "Locked.txt")
        });
        for cell in descendants::<FileCell>(&view_widget) {
            let name = cell.name();
            let emblems = cell.emblems();
            assert_eq!(emblems.link, name == "Shortcut.txt", "{name} in {view}");
            assert_eq!(emblems.read_only, name == "Locked.txt", "{name} in {view}");
        }
    }
}

/// Regression: after a merge the Documents folder's Type column read
/// GIO's "Word 2007 document", "Excel 2007 spreadsheet" and "Plain text
/// document". The files are real, so GIO names their types, the app's
/// loader lists them and the details view draws the cells read here.
///
/// parity: VIEW-002
#[gtk::test]
fn the_type_column_shows_the_interface_names_for_documents() {
    let fixture = Fixture::standard();
    let documents = [
        ("Meeting notes.docx", "Word document"),
        ("Project budget.xlsx", "Excel worksheet"),
        ("Q3 presentation.pptx", "PowerPoint presentation"),
        ("Website assets.zip", "Compressed folder"),
    ];
    for (name, _) in documents {
        fixture.write(name);
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let column_view = test.window.folder_pane().details().column_view().clone();
    let shown_texts = || -> Vec<String> {
        let labels = descendants::<gtk::Label>(&column_view);
        labels.iter().map(|label| label.text().into()).collect()
    };
    wait_until("the Type cells to be bound", || {
        shown_texts().iter().any(|text| text == "Word document")
    });
    let texts = shown_texts();
    for (name, type_label) in documents {
        assert!(texts.iter().any(|text| text == type_label), "{name}: {texts:?}");
    }
    assert!(texts.iter().any(|text| text == "Text document"), "{texts:?}");
}

#[gtk::test]
fn a_dragged_sidebar_stops_where_the_folder_pane_keeps_its_room() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let workspace = test.window.workspace();
    assert_eq!(workspace.position(), 210, "a new sidebar is 210 pixels wide");
    workspace.set_position(5000);
    let details = test.window.details_pane();
    let details_width = if details.is_visible() { details.width() } else { 0 };
    let room_left = workspace.width() - workspace.position() - details_width;
    assert!(workspace.position() <= 560);
    assert!(
        room_left >= 300,
        "the folder pane keeps 300 pixels, not {room_left}"
    );
}

/// parity: SIDE-023
#[gtk::test]
fn a_double_click_on_the_handle_resets_the_sidebar_to_210_and_saves_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let workspace = test.window.workspace();
    workspace.set_position(320);
    let controllers = workspace.observe_controllers();
    let clicks = controllers
        .iter::<gtk::glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok());
    let reset = clicks
        .into_iter()
        .find(|click| click.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the workspace hears clicks on its handle");

    reset.emit_by_name::<()>("pressed", &[&2_i32, &320.0_f64, &10.0_f64]);

    assert_eq!(workspace.position(), 210);
    wait_until("the width to be saved", || {
        test.context.settings_data().preferences.sidebar_width == Some(210)
    });
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
        preferences.view == View::Grid && preferences.show_hidden
    });
    let second = test.open_beside(&fixture.uri());
    let pane = second.window.folder_pane();
    assert_eq!(pane.view(), FolderView::Icons(IconSize::LARGE));
    assert_eq!(second.action_state("view").as_deref(), Some("large"));
    assert!(second.names().contains(&".private".to_owned()));
}

/// Ported from `v2.0.0:desktop/tests/text_size.test.cjs` (unmodified typing
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
    let button = second.window.command_bar().appearance_tooltip();
    assert_eq!(button.as_deref(), Some("Appearance: dark. Click to change."));
}

/// Each theme's settings value is also the `win.theme` target: choosing it
/// sets the skin, shows it as the action's state and saves it; any other
/// target is refused.
#[gtk::test]
fn every_choice_round_trips_through_settings_and_action_keys() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for theme in Theme::ALL {
        test.activate("theme", Some(theme.as_str()));
        assert_eq!(skin().theme(), theme);
        assert_eq!(test.action_state("theme").as_deref(), Some(theme.as_str()));
        wait_until("the theme to be saved", || {
            test.context.settings_data().preferences.theme == theme
        });
    }
    let chosen = skin().theme();
    test.activate("theme", Some("sepia"));
    assert_eq!(skin().theme(), chosen, "an unknown theme is refused");
}

/// The windows follow skins on no display, so the test can ask each skin
/// whether anything is still connected to it: two windows share one skin
/// and a third, open at the same time, has its own.
///
/// parity: TAB-050
#[gtk::test]
fn closing_a_window_disconnects_it_from_the_shared_skin() {
    let fixture = Fixture::standard();
    let shared = Skin::detached();
    let lone = Skin::detached();
    let first = TestWindow::open_with_skin(&fixture.uri(), &shared);
    let second = first.open_beside(&fixture.uri());
    let third = TestWindow::open_with_skin(&fixture.uri(), &lone);
    assert!(shared.has_listeners());
    assert!(lone.has_listeners());
    drop(third);
    assert!(!lone.has_listeners(), "the closed window left no handler");
    drop(second);
    assert!(shared.has_listeners(), "the open window still follows the skin");
    drop(first);
    assert!(!shared.has_listeners(), "the last window left no handler");
}

/// Where the icon view's tiles are, relative to the view's scroller.
fn tile_bounds(test: &TestWindow) -> (i32, Vec<Bounds>) {
    let grid = test.window.folder_pane().icon_view().grid();
    let scroll = grid.parent().expect("the icon view scrolls");
    let cells = descendants::<FileCell>(grid);
    let bounds = cells
        .iter()
        .filter_map(WidgetExt::parent)
        .filter_map(|tile| tile.compute_bounds(&scroll))
        .map(|rect| Bounds::from_rect(&rect))
        .collect();
    (scroll.width(), bounds)
}

/// A window that opened in the icon view used to keep one column, because
/// GTK ignored the column count set while it allocated the view. Tiles sit
/// where `renderRows` in app.js puts them: `floor(width / 135)` columns
/// sharing the width less 20 pixels, the first 10 pixels in and 5 down,
/// 4 pixels apart, 128 pixels tall on a 130-pixel pitch.
///
/// parity: VIEW-005, LOOK-014
#[gtk::test]
fn a_window_that_opens_in_the_icon_view_lays_tiles_out_as_render_rows() {
    let fixture = Fixture::with_files(12);
    let test = TestWindow::without_tabs();
    test.window.show_view(FolderView::Icons(IconSize::LARGE));
    test.show(&fixture.uri());
    wait_for_frames(&test.window, 4);
    let (width, tiles) = tile_bounds(&test);
    let columns = width / 135;
    assert!(columns >= 2, "a {width}-pixel pane holds several columns");
    let tile_width = (width - 20) / columns - 4;
    assert_eq!(tiles[0], Bounds::new(10, 5, tile_width, 128), "the first tile");
    let first_row: Vec<_> = tiles.iter().filter(|tile| tile.y == 5).collect();
    assert_eq!(first_row.len(), usize::try_from(columns).expect("a few columns"));
    let next_row = tiles.iter().find(|tile| tile.y != 5).expect("a second row");
    assert_eq!(next_row.y, 5 + 130, "rows are 130 pixels apart");
    let first_cell = descendants::<FileCell>(test.window.folder_pane().icon_view().grid())
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

/// Choosing a view starts it from the top and saves it; Explorer's keys
/// choose Details (Ctrl+Shift+6) and the icon sizes (Ctrl+Shift+1 to 4),
/// and Alt+Shift+P opens the details pane.
///
/// parity: VIEW-006, VIEW-007, VIEW-009, VIEW-063
#[gtk::test]
fn a_chosen_view_starts_at_the_top_and_has_explorers_keys() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let last = pane.model().n_items() - 1;
    pane.reveal(last);
    wait_until("the list to scroll", || pane.scroll_position() > 0.0);
    test.activate("view", Some("small"));
    wait_until("the icons to start at the top", || pane.scroll_position() == 0.0);
    wait_until("the view to be saved", || {
        test.context.settings_data().preferences.view == View::Grid
    });
    let view = WindowAction::View.detailed_name();
    let keys = |target: &str| application().accels_for_action(&format!("{view}::{target}"));
    assert_eq!(keys("details"), ["<Shift><Control>6"]);
    assert_eq!(keys("extra-large"), ["<Shift><Control>1"]);
    let pane_keys = application().accels_for_action(&WindowAction::DetailsPane.detailed_name());
    assert_eq!(pane_keys, ["<Shift><Alt>p"]);
}

/// Ctrl+H shows hidden files wherever focus is, except in a text field,
/// where it is the field's own key: also on a focused breadcrumb and on
/// the Settings page, as `onKey` handles it before its early return.
///
/// parity: VIEW-023, VIEW-026
#[gtk::test]
fn ctrl_h_shows_hidden_files_outside_text_fields() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(
        application().accels_for_action("win.hidden").is_empty(),
        "Ctrl+H is no accelerator, which would run inside text fields too"
    );
    let ctrl_h =
        || press_shortcut_where_focused(&test, gtk::gdk::Key::h, gtk::gdk::ModifierType::CONTROL_MASK);
    let shows_hidden = || test.context.settings_data().preferences.show_hidden;
    test.window.folder_pane().focus_view();
    assert!(ctrl_h());
    test.wait_for_listing("the listing with hidden files");
    assert!(test.names().contains(&".private".to_owned()));
    wait_for_frames(&test.window, 2);
    let owners = test.window.folder_pane().owners();
    assert_eq!(
        owners.is_shown_hidden(test.position_of(".private")),
        Some(true),
        "hidden items fade"
    );
    assert_eq!(
        owners.is_shown_hidden(test.position_of("Notes 2.txt")),
        Some(false)
    );
    // A cut hidden item carries both classes; the stylesheet fades it
    // further (`.hidden-item.cut`), so the cut still shows.
    let private = test.position_of(".private");
    test.window.folder_pane().model().select_only(private);
    test.activate("cut", None);
    wait_until("the cut hidden item to dim", || {
        owners.is_shown_cut(private) == Some(true)
    });
    assert_eq!(owners.is_shown_hidden(private), Some(true));
    wait_until("the choice to be saved", shows_hidden);

    test.window.search_box().entry().grab_focus();
    wait_until("the search box to take focus", || {
        test.window.focus_is_in_text_field()
    });
    assert!(!ctrl_h(), "Ctrl+H is the text field's own key");
    assert!(shows_hidden(), "hidden files stay shown");

    let crumbs = test.window.address_bar().crumb_buttons();
    let crumb = crumbs.last().expect("the folder has breadcrumbs");
    crumb.grab_focus();
    wait_until("a breadcrumb to take focus", || crumb.has_focus());
    assert!(ctrl_h(), "Ctrl+H works on a breadcrumb");
    wait_until("hidden files to be hidden again", || !shows_hidden());

    test.window.open_settings(None);
    wait_until("the Settings page", || test.window.shows_settings());
    assert!(ctrl_h(), "Ctrl+H works on the Settings page");
    wait_until("hidden files to be shown again", shows_hidden);
}

/// The text-size keys work inside modal dialogs too, which the
/// application's accelerators do not reach: they change the size of the
/// window the dialog belongs to. The window's dialogs, Rename and Map
/// network location among them, are checked; Open with, the
/// update and the Brave dialogs install the same keys.
///
/// parity: VIEW-043
#[gtk::test]
fn text_size_keys_work_inside_dialogs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _theme = ThemeGuard::keep();
    let before = test.window.skin().text_size();
    let rename = crate::window::dialog::Dialog::new(&test.window, "Rename", "");
    let map = crate::dialogs::map_network_dialog(&test.window, |_, _| {});
    let dialogs: [&gtk::Window; 2] = [rename.upcast_ref(), map.upcast_ref()];
    let plus = gtk::ShortcutTrigger::parse_string("<Control>plus").expect("a trigger");
    let mut expected = before;
    for dialog in dialogs {
        let shortcut = dialog
            .observe_controllers()
            .iter::<gtk::glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
            .flat_map(|controller| {
                let shortcuts = controller.iter::<gtk::glib::Object>().filter_map(Result::ok);
                shortcuts
                    .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
                    .collect::<Vec<_>>()
            })
            .find(|shortcut| shortcut.trigger().is_some_and(|trigger| trigger.equal(&plus)))
            .expect("the dialog has Ctrl+plus");
        let action = shortcut.action().expect("the shortcut runs something");
        assert!(action.activate(gtk::ShortcutActionFlags::empty(), dialog, None));
        expected = Step::Increase.apply(expected);
        assert_eq!(test.window.skin().text_size(), expected);
    }
    rename.destroy();
    map.destroy();
    test.window.change_text_size(before);
}

/// Ctrl+wheel zooming changes the view, its menu state and the saved
/// preference, as choosing the view does: past Details and List it steps
/// through every icon size, which the status bar's slider shows and sets.
///
/// parity: VIEW-010, VIEW-011
#[gtk::test]
fn zooming_changes_and_saves_the_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().model().select_only(3);
    test.window.zoom_view(6);
    assert_eq!(
        test.window.folder_pane().view(),
        FolderView::Icons(IconSize::MEDIUM)
    );
    assert_eq!(test.action_state("view").as_deref(), Some("medium"));
    wait_until("the icon view to be saved", || {
        test.context.settings_data().preferences.view == View::Grid
    });
    assert_eq!(test.selected_names().len(), 1, "zooming keeps the selection");
    let slider = test.window.status_bar().zoom_slider();
    assert!(slider.is_visible());
    slider.set_value(slider.value() + 1.0);
    assert_eq!(test.window.folder_pane().view().as_str(), "icons-48");
}
