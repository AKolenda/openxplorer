// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' context menus in a real window, against `entryMenu`,
//! `backgroundMenu` and `openNewMenu` of `desktop/ui/app.js`: the item a
//! right-click selects, both menu styles, the items the selection
//! disables, and the menus "Show more options" and "New…" open in place.

use std::fs;

use gtk::prelude::*;
use ox_core::settings::{ContextMenu, PreferencesUpdate, Settings};

use super::file_ops_support::select_names;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuStyle;

/// The position of the item called `name` in `test`'s view.
fn position_of(test: &TestWindow, name: &str) -> u32 {
    let model = test.window.folder_model();
    (0..model.n_items())
        .find(|position| model.name_at(*position).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} is listed"))
}

/// Saves the "Right-click menu" choice `style`, as Settings does, and
/// waits until the window reads it.
fn choose_menu_style(test: &TestWindow, style: ContextMenu) {
    let update = PreferencesUpdate {
        context_menu: Some(style),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.context_menu == style
    });
}

/// parity: CMD-008, CMD-009, CMD-015, SEL-003
#[gtk::test]
fn right_clicking_a_file_selects_it_and_opens_the_classic_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 10.txt"]);

    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();

    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert!(menu.is_visible());
    assert_eq!(menu.style(), MenuStyle::Classic);
    assert!(menu.strip_labels().is_empty());
    assert_eq!(
        menu.row_labels(),
        [
            "Open",
            "Open containing folder in Terminal",
            "Open with…",
            "-",
            "Cut",
            "Copy",
            "Paste",
            "-",
            "Rename",
            "Move to Trash",
            "Duplicate",
            "Copy path",
            "Compress to ZIP file",
            "Compress to…",
            "-",
            "Previous versions",
            "Properties",
        ]
    );
    assert!(menu.row("Rename").is_sensitive());
    assert!(
        menu.row("Open with…").is_sensitive(),
        "Open with works for one item"
    );
}

/// The keyboard starts on the first enabled item, Up and Down wrap
/// around, and closing the menu gives the keyboard back to the file pane.
///
/// parity: CMD-015
#[gtk::test]
fn the_keyboard_starts_on_the_first_item_wraps_and_returns_to_the_files() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    wait_for_frames(&test.window, 2);
    let enabled: Vec<gtk::ListBoxRow> = menu.rows().into_iter().filter(WidgetExt::is_sensitive).collect();
    let (first, last) = (enabled[0].clone(), enabled[enabled.len() - 1].clone());
    assert!(first.has_focus(), "the first enabled item has the keyboard");

    let list = first
        .parent()
        .and_downcast::<gtk::ListBox>()
        .expect("rows are in a list");
    list.emit_move_cursor(gtk::MovementStep::DisplayLines, -1, false, false);
    assert!(last.has_focus(), "Up on the first item wraps to the last");
    list.emit_move_cursor(gtk::MovementStep::DisplayLines, 1, false, false);
    assert!(first.has_focus(), "Down on the last item wraps to the first");

    menu.popdown();
    let view = test.window.folder_pane().view_widget();
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
    assert!(
        focus.is_some_and(|focus| focus == view || focus.is_ancestor(&view)),
        "the file pane has the keyboard again"
    );
}

/// parity: CMD-009, OPS-014, SEL-003
#[gtk::test]
fn with_several_items_selected_the_one_item_commands_are_disabled() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();

    assert_eq!(
        test.selected_names().len(),
        2,
        "a selected item keeps the selection"
    );
    for disabled in ["Open", "Copy path"] {
        assert!(!menu.row(disabled).is_sensitive(), "{disabled}");
    }
    // CMD-031: a disabled item says why.
    let tooltip = menu.row("Copy path").tooltip_text().unwrap_or_default();
    assert_eq!(tooltip, "Copy path\nSelect only one item for this command.");
    // Rename renames them together (OPS-014); Properties describe them
    // together (PROP-002).
    for enabled in ["Cut", "Copy", "Rename", "Move to Trash", "Duplicate", "Properties"] {
        assert!(menu.row(enabled).is_sensitive(), "{enabled}");
    }
}

/// parity: CMD-008, CMD-010
#[gtk::test]
fn the_compact_menu_has_the_strip_and_show_more_options_opens_the_classic_one() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);

    test.window.right_click(Some(position_of(&test, "Documents")));
    let menu = test.window.context_menu();

    assert_eq!(menu.style(), MenuStyle::Compact);
    assert_eq!(
        menu.strip_labels(),
        ["Cut", "Copy", "Paste", "Rename", "Move to Trash"]
    );
    assert!(!menu.row_labels().contains(&"Cut".to_owned()));
    menu.row("Show more options").emit_activate();
    wait_until("the classic menu", || {
        menu.is_visible() && menu.style() == MenuStyle::Classic
    });
    assert!(menu.row_labels().contains(&"Cut".to_owned()));
    assert!(menu.strip_labels().is_empty());
}

/// parity: CMD-011, CMD-004
#[gtk::test]
fn right_clicking_blank_space_opens_the_folder_menu_and_new_opens_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.window.right_click(None);
    let menu = test.window.context_menu();

    assert!(
        test.selected_names().is_empty(),
        "blank space clears the selection"
    );
    assert_eq!(menu.row_labels()[0], "New…");
    assert!(menu.row("Refresh").is_sensitive());
    assert!(!menu.row("Undo").is_sensitive(), "nothing to undo yet");
    menu.row("New…").emit_activate();
    wait_until("the New menu", || {
        menu.is_visible() && menu.row_labels().first().map(String::as_str) == Some("Folder")
    });
    assert_eq!(
        menu.row_labels(),
        [
            "Folder",
            "Text document",
            "File…",
            "-",
            "Markdown document",
            "CSV file",
            "JSON file",
            "HTML document",
            "-",
            "From template…",
            "-",
            "Link to file or folder…",
        ]
    );
}

/// parity: CMD-013, CMD-014
#[gtk::test]
fn the_menu_key_opens_the_classic_menu_of_the_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    select_names(&test, &["Résumé.txt"]);

    test.activate("context-menu", None);
    let menu = test.window.context_menu();

    assert!(menu.is_visible());
    assert_eq!(
        menu.style(),
        MenuStyle::Classic,
        "the keyboard opens the classic menu"
    );
    assert_eq!(menu.row_labels()[0], "Open");
}

/// parity: SIDE-009, SIDE-014
#[gtk::test]
fn unpin_from_the_pin_menu_removes_only_the_pin() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Pinned here")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri_of("Pinned here"));
    let is_pinned = || test.window.sidebar().labels().contains(&"Pinned here".to_owned());
    test.activate("pin-folder", None);
    wait_until("the pin", is_pinned);

    test.activate("unpin", Some(&fixture.uri_of("Pinned here")));

    wait_until("the pin to go", || !is_pinned());
    assert_eq!(
        test.window.shown_message(),
        "Unpinned. The folder was not deleted."
    );
    assert!(fixture.path("Pinned here").is_dir());
}

/// parity: SIDE-014
#[gtk::test]
fn right_clicking_a_pin_opens_its_menu() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Pinned menu")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri_of("Pinned menu"));
    test.activate("pin-folder", None);
    wait_until("the pin", || {
        test.window.sidebar().labels().contains(&"Pinned menu".to_owned())
    });

    // The new row is laid out a frame after it is added.
    wait_until("the pin's menu", || {
        test.window.sidebar().right_click_row("Pinned menu").is_visible()
    });
    let menu = test.window.sidebar().right_click_row("Pinned menu");

    assert_eq!(menu.row_labels()[0], "Open");
    assert!(menu.row("Unpin from Quick access").is_sensitive());
    assert!(
        menu.row("Properties").is_sensitive(),
        "Properties of the pin works"
    );
}

/// parity: TAB-012, TAB-013
#[gtk::test]
fn right_clicking_a_tab_opens_its_menu_and_duplicate_tab_opens_the_same_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the subfolder");
    let tab = test
        .window
        .tab_strip()
        .tab_list()
        .first_child()
        .expect("the window has a tab");
    let right_click = tab
        .observe_controllers()
        .into_iter()
        .filter_map(|controller| controller.ok().and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == gtk::gdk::BUTTON_SECONDARY)
        .expect("a tab opens its menu on a right-click");

    right_click.emit_by_name::<()>("pressed", &[&1_i32, &5.0_f64, &5.0_f64]);
    let menu = test.window.tab_strip().menu();

    assert!(menu.is_visible());
    assert_eq!(
        menu.row_labels(),
        [
            "Move tab to new window",
            "Move tab to window…",
            "Duplicate tab",
            "Open windows…",
            "-",
            "Close tab",
            "Close other tabs",
        ]
    );
    assert!(
        menu.row("Move tab to new window").is_sensitive(),
        "tabs move between windows"
    );
    assert!(menu.row("Move tab to window…").is_sensitive());
    menu.row("Duplicate tab").emit_activate();
    wait_until("the duplicate tab", || test.window.tab_count() == 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert!(!test.window.is_action_enabled("back"), "the history stays behind");
}

/// Mount disk image and Analyse disk usage start their tool on the
/// item's local path.
///
/// parity: DEV-011, PROP-015
#[gtk::test]
fn the_disk_tools_run_on_the_items_local_path() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("distro.iso"), b"image").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());

    test.activate("mount-disk-image", Some(&fixture.uri_of("distro.iso")));
    test.activate("analyse-disk-usage", Some(&fixture.uri_of("Documents")));

    assert_eq!(
        test.context.recorded_launches(),
        [
            format!("MountImage {}", fixture.path("distro.iso").display()),
            format!("AnalyseUsage {}", fixture.path("Documents").display()),
        ]
    );
}
