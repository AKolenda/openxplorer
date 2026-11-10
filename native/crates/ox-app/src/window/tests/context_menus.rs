// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' context menus in a real window, against `entryMenu`,
//! `backgroundMenu` and `openNewMenu` of `v2.0.0:desktop/ui/app.js`: the item a
//! right-click selects, both menu styles, the items the selection
//! disables, "Show more options" opening the classic menu in place, and
//! submenus opening beside a menu.

use std::fs;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{ContextMenu, PreferencesUpdate, Settings, StripPosition};

use super::file_ops_support::{
    dialog_over, is_enabled, is_renaming_in_place, is_triggered_by, name_editor, open_dialog,
    require_private_trash, select_names, shortcuts_of, text_field, wait_for_no_dialog,
};
use crate::test_support::harness::{
    capture_popover, settle, wait_for_frames, wait_until, Fixture, TestWindow,
};
use crate::window::menu_popover::{MenuEntry, MenuStyle, Slide};

/// The position of the item called `name` in `test`'s view.
pub(super) fn position_of(test: &TestWindow, name: &str) -> u32 {
    let model = test.window.folder_model();
    (0..model.n_items())
        .find(|position| model.name_at(*position).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} is listed"))
}

/// Saves the "Right-click menu" choice `style`, as Settings does, and
/// waits until the window reads it.
pub(super) fn choose_menu_style(test: &TestWindow, style: ContextMenu) {
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

/// Saves `position` as the Settings choice "Buttons in the right-click menu"
/// and waits for the window to read it.
pub(super) fn choose_strip_position(test: &TestWindow, position: StripPosition) {
    let update = PreferencesUpdate {
        strip_position: Some(position),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.strip_position == position
    });
}

/// As in Windows Explorer, pressing the right button selects the item and
/// releasing it opens the menu, so a right-button drag can start from the
/// press instead (DND-027).
///
/// parity: CMD-008, DND-027
#[gtk::test]
fn a_right_press_selects_the_item_and_its_release_opens_the_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 10.txt"]);
    let (right_button, x, y) = test
        .window
        .right_button_on_item(position_of(&test, "Notes 2.txt"));
    let menu = test.window.context_menu();

    right_button.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
    let selected_on_press = test.selected_names();
    let open_on_press = menu.is_visible();
    right_button.emit_by_name::<()>("released", &[&1_i32, &x, &y]);

    assert_eq!(selected_on_press, ["Notes 2.txt"]);
    assert!(!open_on_press, "no menu while the button is down");
    assert!(menu.is_visible(), "the release opens the menu");
}

/// parity: CMD-008, CMD-009, CMD-015, SEL-003, CMD-038
#[gtk::test]
fn right_clicking_a_file_selects_it_and_opens_the_classic_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 10.txt"]);

    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();

    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert!(menu.is_visible());
    assert_eq!(menu.style(), MenuStyle::Classic);
    assert!(menu.strip_labels().is_empty());
    // "Open with <app>" rows name whatever the machine has installed.
    let rows: Vec<String> = menu
        .row_labels()
        .into_iter()
        .filter(|label| !label.starts_with("Open with "))
        .collect();
    assert_eq!(
        rows,
        [
            "Open",
            "Open containing folder in Terminal",
            "Open with…",
            "-",
            "Copy as path",
            "Restore previous versions",
            "-",
            "Send to",
            "-",
            "Cut",
            "Copy",
            "-",
            "Duplicate",
            "Delete",
            "Rename",
            "-",
            "Compress to…",
            "-",
            "Properties",
            "Configure service actions…",
        ]
    );
    assert!(menu.row("Rename").is_sensitive());
    assert!(
        menu.row("Open with…").is_sensitive(),
        "Open with works for one item"
    );
}

/// The classic menu is drawn as Windows 11 draws "Show more options":
/// no row shows shortcut text, Open is in bold, and a row Windows draws
/// as text alone, such as Open or Cut, keeps the glyph's column empty, so
/// its text lines up with the rows that keep a glyph, such as Properties.
///
/// parity: LOOK-019, CMD-008
#[gtk::test]
fn the_classic_menu_shows_no_shortcut_text_bolds_open_and_lines_up_text_only_rows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win10);
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    assert_eq!(menu.style(), MenuStyle::Classic);
    let label_of = |row: &gtk::ListBoxRow| {
        crate::window::widget_tree::descendants::<gtk::Label>(row)
            .into_iter()
            .next()
            .expect("a label")
    };
    let shortcuts: Vec<String> = menu
        .rows()
        .iter()
        .flat_map(crate::window::widget_tree::descendants::<gtk::Label>)
        .filter(|label| label.has_css_class("shortcut"))
        .map(|label| label.text().to_string())
        .collect();
    assert_eq!(shortcuts, Vec::<String>::new(), "no shortcut text");
    assert_eq!(menu.emphasised_labels(), ["Open"]);
    let images = |label: &str| crate::window::widget_tree::descendants::<gtk::Image>(&menu.row(label)).len();
    assert_eq!(images("Open"), 0, "Open is text alone");
    assert_eq!(images("Cut"), 0, "Cut is text alone");
    assert_eq!(images("Properties"), 1, "Properties keeps its glyph");
    let left = |label: &str| {
        let row = menu.row(label);
        let text = label_of(&row);
        text.compute_point(&row, &gtk::graphene::Point::zero())
            .map(|point| point.x())
    };
    wait_until("the rows laid out", || {
        menu.row("Open").width() > 0 && left("Open").is_some_and(|x| x > 0.0)
    });
    assert_eq!(left("Open"), left("Properties"), "the text lines up");
    assert_eq!(left("Cut"), left("Properties"));
    menu.popdown();
}

/// Windows 11's compact menu is the default: the edit commands in its
/// strip and "Show more options" last, with nothing added after it.
///
/// parity: CMD-010, SET-016
#[gtk::test]
fn right_clicking_opens_the_compact_menu_with_show_more_options_last() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();

    assert_eq!(menu.style(), MenuStyle::Compact);
    assert!(
        !menu.strip_labels().is_empty(),
        "the edit commands sit in the strip"
    );
    let rows = menu.row_labels();
    assert_eq!(
        rows.last().map(String::as_str),
        Some("Show more options"),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row == "Configure service actions…"),
        "configuring is in the classic menu: {rows:?}"
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

/// parity: CMD-009, OPS-014, OPS-050, SEL-003
#[gtk::test]
fn with_several_items_selected_the_one_item_commands_are_disabled() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();

    assert_eq!(
        test.selected_names().len(),
        2,
        "a selected item keeps the selection"
    );
    assert!(!menu.row("Restore previous versions").is_sensitive());
    // CMD-031: a disabled item tells screen readers why, with no tooltip,
    // as Windows 11's menu rows show none.
    assert_eq!(menu.row("Restore previous versions").tooltip_text(), None);
    assert_eq!(
        menu.disabled_reason_of("Restore previous versions").as_deref(),
        Some("Select only one item for this command.")
    );
    // Open opens each of them (OPEN-003); Rename gives them one numbered
    // name (OPS-050) and Rename items… opens the batch rename (OPS-014);
    // Properties describe them together (PROP-002).
    for enabled in [
        "Open",
        "Cut",
        "Copy",
        "Rename",
        "Rename items…",
        "Delete",
        "Duplicate",
        "Copy as path",
        "Properties",
    ] {
        assert!(menu.row(enabled).is_sensitive(), "{enabled}");
    }
}

/// The classic menu ("Show more options") lists Rename items… under
/// Rename while several items are selected, and not for one item. The row
/// is named for assistive technology and takes the keyboard like the
/// others; it opens the "Rename items" dialog, whose `#` template numbers
/// the items, and one Undo renames them back.
///
/// parity: OPS-014, CMD-009, OPS-029
#[gtk::test]
fn rename_items_in_the_classic_menu_opens_the_numbering_dialog_for_several_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    assert_eq!(menu.style(), MenuStyle::Classic);
    assert!(
        !menu.row_labels().contains(&"Rename items…".to_owned()),
        "not for one item"
    );
    menu.popdown();

    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    let rows = menu.row_labels();
    let rename = rows
        .iter()
        .position(|label| label == "Rename")
        .expect("Rename is listed");
    assert_eq!(rows[rename + 1], "Rename items…");
    let row = menu.row("Rename items…");
    assert!(row.is_sensitive());
    assert!(
        row.is_activatable() && row.is_focusable(),
        "the keyboard reaches it"
    );
    assert!(gtk::test_accessible_has_property(
        &row,
        gtk::AccessibleProperty::Label
    ));
    // Enter on the focused row activates it.
    row.emit_activate();

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Rename items");
    assert_eq!(dialog.message_text(), "Rename the 2 selected items to:");
    text_field(&dialog).set_text("Plan #");
    dialog.press("Rename");
    wait_until("the renamed files to be selected", || {
        test.selected_names() == ["Plan 1.txt", "Plan 2.txt"]
    });
    assert!(!fixture.path("Notes 2.txt").exists());
    test.activate("undo", None);
    wait_until("the batch to be renamed back", || {
        fixture.path("Notes 2.txt").is_file() && fixture.path("Notes 10.txt").is_file()
    });
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
    // Windows 11 shows Paste on a folder only while there is something to
    // paste, and Share only for files.
    assert_eq!(menu.strip_labels(), ["Cut", "Copy", "Rename", "Delete"]);
    menu.popdown();
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.window.right_click(Some(position_of(&test, "Documents")));
    assert_eq!(menu.strip_labels(), ["Cut", "Copy", "Paste", "Rename", "Delete"]);
    // Each button shows its label under its icon, as in Windows 11.
    assert_eq!(menu.strip_texts(), ["Cut", "Copy", "Paste", "Rename", "Delete"]);
    assert!(!menu.row_labels().contains(&"Cut".to_owned()));
    menu.row("Show more options").emit_activate();
    wait_until("the classic menu", || {
        menu.is_visible() && menu.style() == MenuStyle::Classic
    });
    assert!(menu.row_labels().contains(&"Cut".to_owned()));
    assert!(menu.strip_labels().is_empty());
}

/// With the toast's Undo button gone, the folder menu is where Undo is
/// found besides Ctrl+Z: as in Windows Explorer it names the step, "Undo
/// Delete" after a move to the Recycle Bin, and Redo follows once it is
/// undone.
///
/// parity: CMD-011, OPS-029, OPS-031
#[gtk::test]
fn the_folder_menu_names_the_step_undo_and_redo_take_as_windows_does() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic folder menu, which lists Redo; Windows 11's compact menu,
    // the default, lists Undo only.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("trash", None);
    wait_until("the file to go", || {
        !test.names().contains(&"Notes 2.txt".to_owned())
    });

    test.window.right_click(None);
    let menu = test.window.context_menu();
    let undo = menu.row("Undo Delete");
    assert!(undo.is_sensitive());
    undo.emit_activate();
    wait_until("the file to come back", || fixture.path("Notes 2.txt").is_file());
    wait_until("the undo to end", || !test.window.is_writing_files());
    test.window.right_click(None);
    let menu = test.window.context_menu();

    assert!(menu.row("Redo Delete").is_sensitive());
    assert!(!menu.row("Undo").is_sensitive(), "nothing left to undo");
    menu.popdown();
    test.activate("new-folder", None);
    name_editor(&test).emit_activate();
    wait_until("the name to be kept", || !is_renaming_in_place(&test));
    assert!(fixture.path("New folder").is_dir());
    test.window.right_click(None);
    assert!(test.window.context_menu().row("Undo New").is_sensitive());
}

/// parity: CMD-011, CMD-004
#[gtk::test]
fn right_clicking_blank_space_opens_the_folder_menu_and_new_opens_beside_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic folder menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);

    test.window.right_click(None);
    let menu = test.window.context_menu();

    assert!(
        test.selected_names().is_empty(),
        "blank space clears the selection"
    );
    assert!(menu.row("New").is_sensitive());
    assert!(menu.row("Refresh").is_sensitive());
    assert!(!menu.row("Undo").is_sensitive(), "nothing to undo yet");
    let new = menu.open_submenu_of("New");
    assert_eq!(
        new.row_labels(),
        [
            "Folder",
            "Shortcut",
            "-",
            "Bitmap image",
            "Rich Text Document",
            "Text Document",
            "Compressed (zipped) Folder",
        ]
    );
}

/// In a folder the user may not write to (no write permission, on a
/// writable drive), the folder menu's New and Paste stay on, empty
/// folder or not, as in Windows Explorer: the user tries, and is told
/// why. Paste asks Windows' "Destination Folder Access Denied", New says
/// the system's error in the toast, and nothing is made. Tests run as a normal
/// user: root ignores the permission.
///
/// parity: CMD-011, CMD-031, OPS-001, OPS-047
#[gtk::test]
fn a_folder_without_write_permission_keeps_new_and_paste_and_shows_the_error() {
    use std::os::unix::fs::PermissionsExt;

    /// Gives the folders their write permission back, also when the test
    /// fails, so the fixture can be removed.
    struct Locked(Vec<std::path::PathBuf>);
    impl Drop for Locked {
        fn drop(&mut self) {
            for folder in &self.0 {
                let _ = fs::set_permissions(folder, fs::Permissions::from_mode(0o755));
            }
        }
    }

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Locked")).expect("a folder");
    fixture.write("Locked/inside.txt");
    let locked = Locked(vec![fixture.path("Locked"), fixture.path("Documents")]);
    for folder in &locked.0 {
        fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).expect("take write permission away");
    }
    let test = TestWindow::open(&fixture.uri());
    // The classic folder menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    assert!(is_enabled(&test, "paste"), "something to paste");

    for (folder, what) in [
        ("Documents", "an empty folder"),
        ("Locked", "a folder with items"),
    ] {
        test.window.navigate(&fixture.uri_of(folder)).expect("a folder");
        test.wait_for_listing(what);
        settle();
        wait_for_frames(&test.window, 3);
        assert!(is_enabled(&test, "new-folder"), "New in {what}");
        assert!(is_enabled(&test, "paste"), "Paste in {what}");
        test.window.right_click(None);
        let menu = test.window.context_menu();
        for label in ["New", "Paste"] {
            assert!(menu.row(label).is_sensitive(), "{label} in {what}");
        }
        menu.popdown();
    }

    test.activate("paste", None);
    // Windows asks in the progress window, in place of the action's
    // progress.
    let window = crate::window::progress_window::ProgressWindow::of(&test.context);
    wait_until("the action to ask", || {
        window
            .actions()
            .iter()
            .any(crate::window::transfer_panel::TransferPanel::asks_about_failure)
    });
    let panel = window
        .actions()
        .into_iter()
        .find(crate::window::transfer_panel::TransferPanel::asks_about_failure)
        .expect("the asking action");
    assert_eq!(
        panel.caption_shown(),
        "Destination Folder Access Denied",
        "Windows' question about a folder that may not be written to"
    );
    let texts = panel.failure_texts();
    assert!(
        texts
            .iter()
            .any(|text| text == "You'll need to provide administrator permission to copy to this folder"),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "Locked"),
        "it names the folder: {texts:?}"
    );
    assert_eq!(panel.failure_button_labels(), ["Continue", "Skip", "Cancel"]);
    assert!(panel.failure_shows_shield());
    assert!(dialog_over(&test).is_none(), "no dialog over the window");
    panel.press_failure("Cancel");
    // Whether a result follows Cancel is not this test's concern: one that
    // shows is closed.
    wait_until("the paste to end", || {
        is_enabled(&test, "new-folder") || dialog_over(&test).is_some()
    });
    if let Some(report) = dialog_over(&test) {
        report.press("OK");
    }
    wait_for_no_dialog(&test);
    assert!(!fixture.path("Locked/Notes 2.txt").exists(), "nothing pasted");

    wait_until("New to be on again", || is_enabled(&test, "new-folder"));
    let before = test.window.shown_message().to_string();
    test.activate("new-folder", None);
    wait_until("New to say why", || {
        test.window.shown_message() != before.as_str()
    });
    assert!(
        test.window.shown_message().contains("Permission denied"),
        "the system's error: {}",
        test.window.shown_message()
    );
    settle();
    assert!(dialog_over(&test).is_none(), "no dialog");
    assert!(!fixture.path("Locked/New folder").exists(), "no half-made folder");
    assert_eq!(
        fs::read_dir(fixture.path("Locked")).expect("listed").count(),
        1,
        "only inside.txt"
    );
}

/// The folder menu's "Sort by" opens the command bar's Sort menu beside
/// it, and a choice there sorts the folder.
///
/// parity: CMD-012
#[gtk::test]
fn the_folder_menu_sorts_and_changes_the_view_in_submenus() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic folder menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    test.window.right_click(None);
    let menu = test.window.context_menu();
    assert_eq!(menu.row_labels()[..2], ["View", "Sort by"]);

    let sort = menu.open_submenu_of("Sort by");
    assert_eq!(sort.row_labels().first().map(String::as_str), Some("Name"));
    // Size is under More, as in Explorer's Sort menu, beside it.
    let more = sort.open_submenu_of("More");
    assert_eq!(more.row_labels().first().map(String::as_str), Some("Size"));
    more.row("Size").emit_activate();
    wait_until("the menus to close", || !menu.is_visible());

    let sorted = test
        .window
        .action_state("sort")
        .and_then(|state| state.get::<String>());
    assert_eq!(sorted.as_deref(), Some("size"));
    test.window.right_click(None);
    let view = menu.open_submenu_of("View");
    assert_eq!(
        view.row_labels().first().map(String::as_str),
        Some("Extra large icons")
    );
}

/// Right-clicking blank space opens Windows 11's compact folder menu,
/// whose View, Sort by and Group by open beside it in the compact look
/// rather than the classic one.
///
/// parity: CMD-010, CMD-011, CMD-012
#[gtk::test]
fn the_compact_folder_menu_opens_its_submenus_beside_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(None);
    let menu = test.window.context_menu();
    assert_eq!(menu.style(), MenuStyle::Compact);
    assert_eq!(menu.row_labels()[..4], ["View", "Sort by", "Group by", "Refresh"]);
    assert_eq!(
        menu.row_labels().last().map(String::as_str),
        Some("Show more options")
    );
    capture_popover(&test.window, menu.upcast_ref(), "native-menu-compact-folder.png");

    let sort = menu.open_submenu_of("Sort by");
    assert!(sort.row_labels().iter().any(|row| row == "Name"));
    assert_eq!(
        sort.style(),
        MenuStyle::Compact,
        "the submenu keeps the compact look"
    );
    assert!(
        sort.strip_labels().is_empty(),
        "no file actions strip in a submenu"
    );
    assert!(menu.is_visible(), "the folder menu stays open");
}

/// Shift+F10 (`win.context-menu`) and the Menu key both open the
/// right-click menu's style for the selection, Windows 11's compact menu
/// as it comes, as in Windows 11 Explorer, where the classic menu shows
/// only through "Show more options" or Shift+right-click.
///
/// parity: CMD-013, CMD-014
#[gtk::test]
fn shift_f10_and_the_menu_key_open_the_right_click_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    select_names(&test, &["Résumé.txt"]);

    test.activate("context-menu", None);
    let menu = test.window.context_menu();

    assert!(menu.is_visible());
    assert_eq!(
        menu.style(),
        MenuStyle::Compact,
        "Shift+F10 opens the right-click menu, Windows 11's"
    );
    assert_eq!(
        menu.row_labels().last().map(String::as_str),
        Some("Show more options")
    );
    menu.popdown();

    let view = test.window.folder_pane().input_widget();
    let menu_key = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
                .collect::<Vec<_>>()
        })
        .find(|shortcut| {
            shortcut
                .trigger()
                .and_downcast::<gtk::KeyvalTrigger>()
                .is_some_and(|trigger| trigger.keyval() == gtk::gdk::Key::Menu)
        })
        .expect("the view takes the Menu key");
    let action = menu_key.action().expect("the Menu key runs an action");
    assert!(action.activate(gtk::ShortcutActionFlags::empty(), &view, None));
    let menu = test.window.context_menu();
    assert!(menu.is_visible());
    assert_eq!(
        menu.style(),
        MenuStyle::Compact,
        "the Menu key opens the compact menu"
    );
    menu.popdown();

    choose_menu_style(&test, ContextMenu::Win10);
    assert!(action.activate(gtk::ShortcutActionFlags::empty(), &view, None));
    let menu = test.window.context_menu();
    assert_eq!(
        menu.style(),
        MenuStyle::Classic,
        "with the classic menu chosen, the Menu key opens it"
    );
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
    // Until then the click lands on empty space, whose menu differs.
    wait_until("the pin's menu", || {
        let menu = test.window.sidebar().right_click_row("Pinned menu");
        menu.is_visible()
            && menu
                .row_labels()
                .first()
                .is_some_and(|label| label == "Open in new tab")
    });
    let menu = test.window.sidebar().right_click_row("Pinned menu");

    // Windows 11's compact menu of a pinned folder starts with Open in new
    // tab; a click opens the folder.
    assert_eq!(menu.row_labels()[0], "Open in new tab");
    assert!(menu.row("Unpin from Quick access").is_sensitive());
    assert!(
        menu.row("Properties").is_sensitive(),
        "Properties of the pin works"
    );
}

/// parity: TAB-012, TAB-013, TAB-060
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
            "Close tabs to the right",
        ]
    );
    assert!(
        menu.row("Move tab to new window").is_sensitive(),
        "tabs move between windows"
    );
    assert!(menu.row("Move tab to window…").is_sensitive());
    assert!(!menu.row("Close other tabs").is_sensitive(), "the only tab");
    assert!(
        !menu.row("Close tabs to the right").is_sensitive(),
        "the last tab"
    );
    menu.popdown();
    // Off, the tab menu is Windows 11's (TAB-060).
    test.set_more_tab_menu_items(false);
    right_click.emit_by_name::<()>("pressed", &[&1_i32, &5.0_f64, &5.0_f64]);
    let menu = test.window.tab_strip().menu();
    assert_eq!(
        menu.row_labels(),
        [
            "Close tab",
            "Close other tabs",
            "Close tabs to the right",
            "Duplicate tab",
        ]
    );
    menu.row("Duplicate tab").emit_activate();
    wait_until("the duplicate tab", || test.window.tab_count() == 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert!(!test.window.is_action_enabled("back"), "the history stays behind");
}

/// The Menu key and Shift+F10 on the focused tab open that tab's menu
/// under it, with the keyboard on its first item, and closing it gives
/// the keyboard back to the tab, as Shift+F10 opens
/// "the context menu for the selected item" in Windows 11 and arrow keys
/// move along the tab row
/// (support.microsoft.com, "Keyboard shortcuts in Windows").
///
/// parity: TAB-012
#[gtk::test]
fn the_menu_key_and_shift_f10_open_the_focused_tabs_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let tab = test
        .window
        .tab_strip()
        .tab_list()
        .first_child()
        .expect("the window has a tab");
    let keys: Vec<gtk::Shortcut> = tab
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
                .collect::<Vec<_>>()
        })
        .collect();
    let shortcut_for = |key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType| {
        keys.iter()
            .find(|shortcut| {
                shortcut
                    .trigger()
                    .and_downcast::<gtk::KeyvalTrigger>()
                    .is_some_and(|trigger| trigger.keyval() == key && trigger.modifiers() == modifiers)
            })
            .and_then(gtk::Shortcut::action)
    };

    for (key, modifiers) in [
        (gtk::gdk::Key::Menu, gtk::gdk::ModifierType::empty()),
        (gtk::gdk::Key::F10, gtk::gdk::ModifierType::SHIFT_MASK),
    ] {
        let action = shortcut_for(key, modifiers).unwrap_or_else(|| panic!("a focused tab takes {key:?}"));
        assert!(tab.grab_focus(), "the tab in front takes the keyboard");
        assert!(action.activate(gtk::ShortcutActionFlags::empty(), &tab, None));
        let menu = test.window.tab_strip().menu();
        assert!(menu.is_visible(), "{key:?} opens the tab menu");
        assert_eq!(menu.row_labels()[0], "Move tab to new window");
        assert!(
            !menu.row("Close other tabs").is_sensitive(),
            "the only tab has no others to close"
        );
        assert!(
            !test.window.context_menu().is_visible(),
            "the file list's menu stays shut"
        );
        menu.popdown();
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&test.window),
            Some(tab.clone()),
            "Escape gives the keyboard back to the tab"
        );
    }
}

/// Ctrl+W, which a tab's menu shows beside Close tab, closes the tab
/// whose menu is open, not the tab in front: a right-click leaves a
/// background tab in the background, so the menu's own tab is the one
/// Close tab and its shortcut act on. The menu closes with it.
///
/// parity: TAB-012
#[gtk::test]
fn ctrl_w_in_a_background_tabs_menu_closes_that_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for name in ["Documents", "Pictures"] {
        test.window.add_tab(&fixture.uri_of(name)).expect("a folder");
        test.wait_for_listing(name);
    }
    let tab_list = test.window.tab_strip().tab_list();
    let first = tab_list.first_child().expect("three tabs");
    let right_click = first
        .observe_controllers()
        .into_iter()
        .filter_map(|controller| controller.ok().and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == gtk::gdk::BUTTON_SECONDARY)
        .expect("a tab opens its menu on a right-click");
    right_click.emit_by_name::<()>("pressed", &[&1_i32, &5.0_f64, &5.0_f64]);
    let menu = test.window.tab_strip().menu();
    assert!(menu.is_visible());
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri_of("Pictures")),
        "a right-click leaves the tab in the background"
    );

    // The open menu has the keyboard, so Ctrl+W reaches its shortcuts
    // first.
    let ctrl_w = shortcuts_of(menu.upcast_ref(), gtk::PropagationPhase::Capture)
        .into_iter()
        .find(|shortcut| {
            shortcut.trigger().is_some_and(|trigger| {
                is_triggered_by(&trigger, gtk::gdk::Key::w, gtk::gdk::ModifierType::CONTROL_MASK)
            })
        })
        .expect("the tab menu takes Ctrl+W");
    let action = ctrl_w.action().expect("a shortcut runs something");
    assert!(action.activate(gtk::ShortcutActionFlags::empty(), &menu, None));

    wait_until("the first tab to close", || test.window.tab_count() == 2);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri_of("Pictures")),
        "the tab in front stays open"
    );
    let titles: Vec<String> = test
        .window
        .imp()
        .session
        .borrow()
        .tabs()
        .iter()
        .map(|tab| tab.uri().to_owned())
        .collect();
    assert_eq!(titles, [fixture.uri_of("Documents"), fixture.uri_of("Pictures")]);
    wait_until("the menu to close", || !menu.is_visible());
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

/// Format… waits for the window's file operation, as Disconnect does, for
/// a rename or a new item, so no write goes on onto the drive once it is
/// formatted, and for another format, so two never overlap.
///
/// parity: DEV-012
#[gtk::test]
fn format_waits_for_a_running_file_operation() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");
    assert!(operation.is_some(), "no other operation runs");

    test.activate("format-drive", Some("file:///media/demo/USB"));
    let refusal = test.window.shown_message().to_string();
    test.window.end_operation();
    test.window.show_message("");
    let renaming = test.context.hold_write();
    test.activate("format-drive", Some("file:///media/demo/USB"));
    let quick_refusal = test.window.shown_message().to_string();
    drop(renaming);
    let formatting = test.context.hold_writes_for_formatting();
    test.activate("format-drive", Some("file:///media/demo/USB"));
    let second_format = test.window.shown_message().to_string();
    let confirmed_second = test.window.format_refusal();
    drop(formatting);
    let free = test.window.format_refusal();

    assert_eq!(refusal, "Finish the current operation before formatting.");
    assert_eq!(
        quick_refusal, "Finish the current operation before formatting.",
        "a rename or a new item running counts too"
    );
    let waiting = "Wait until the drive is formatted, then try again.";
    assert_eq!(
        second_format, waiting,
        "another format, perhaps of the same drive, runs"
    );
    assert_eq!(
        confirmed_second,
        Some(waiting),
        "checked again once the erase is confirmed"
    );
    assert_eq!(free, None, "a drive may be formatted once nothing else runs");
}

/// While a drive is formatted, no window starts writing files, so a
/// transfer, an extraction, a rename or a new item cannot go on onto the
/// formatted drive.
///
/// parity: DEV-012
#[gtk::test]
fn writes_wait_while_a_drive_is_formatted() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let hold = test.context.hold_writes_for_formatting();

    let refused = test.window.begin_operation("Preparing copy…").is_none();
    let refusal = test.window.shown_message().to_string();
    let archive = test.window.archive_operation_refusal();
    let quick = test.window.admit_write().err();
    let update = crate::window::integration_actions::work_in_windows(test.window.application().as_ref());
    drop(hold);
    let operation = test.window.begin_operation("Preparing copy…");
    test.window.end_operation();
    let admitted = test.window.admit_write().is_ok();

    let waiting = "Wait until the drive is formatted, then try again.";
    assert!(refused, "no copy starts while a drive is formatted");
    assert_eq!(refusal, waiting);
    assert_eq!(archive.as_deref(), Some(waiting), "nor an extraction");
    assert_eq!(quick.as_deref(), Some(waiting), "nor a rename or a new item");
    assert_eq!(update, ox_core::update::Activity::Busy, "nor an update's restart");
    assert!(operation.is_some(), "writes go on once it is formatted");
    assert!(admitted, "quick writes too");
}

/// A format's result is shown in the window it was started from, or, once
/// that window is closed, in another one, so a failure is never lost.
///
/// parity: DEV-012
#[gtk::test]
fn a_formats_result_outlives_its_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri());
    let (started, app) = (test.window.downgrade(), test.window.application());
    let report = crate::window::disk_tools::report_format_result;

    report(&started, app.as_ref(), "Formatted while open.");
    let while_open = test.window.shown_message_text();
    let elsewhere_before = other.window.shown_message_text();
    test.window.close();
    settle();
    report(&started, app.as_ref(), "Formatted after closing.");

    assert_eq!(while_open, "Formatted while open.");
    assert_ne!(
        elsewhere_before, "Formatted while open.",
        "only its own window says so"
    );
    assert!(!test.window.is_visible());
    assert_eq!(other.window.shown_message_text(), "Formatted after closing.");
}

/// Windows' "Copy as path", with its Ctrl+Shift+C, copies the selected
/// item's path in double quotes; the command bar's Share no longer does.
///
/// parity: CLIP-012, CLIP-013
#[gtk::test]
fn copy_as_path_in_the_menu_copies_the_items_path() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    let row = menu.row("Copy as path");
    let labels: Vec<String> = crate::test_support::harness::descendants::<gtk::Label>(&row)
        .iter()
        .map(|label| label.text().to_string())
        .collect();
    assert!(labels.iter().any(|label| label == "Ctrl+Shift+C"), "{labels:?}");

    row.emit_activate();

    let clipboard = test.window.clipboard();
    let copied = gtk::glib::MainContext::default().block_on(clipboard.read_text_future());
    let copied = copied.ok().flatten().map(|text| text.to_string());
    let expected = format!("\"{}\"", fixture.path("Notes 2.txt").display());
    assert_eq!(copied, Some(expected));
}

/// A pinned folder's compact menu, as Windows 11's: Open in new tab, Open
/// in new window, Unpin from Quick access, Compress to, Copy as path, New,
/// Properties, Open in Terminal and Show more options; a standard folder
/// such as Downloads has only Copy in its strip, and the user's own pin
/// has Cut, Copy, Rename and Delete. The strip is on the side nearest the
/// pointer: at the top of a menu opening downward and at the bottom of
/// one opening upward, as Windows 11 draws it, unless Settings puts it
/// always at the top or always at the bottom. With
/// `OX_NATIVE_CAPTURE_DIR` set, the menus are saved there as
/// `context-menu-match-strip-{pointer,top,bottom}-{down,up}-{light,dark}.png`
/// and `context-menu-match-pinned-{down,up}-{light,dark}.png`.
///
/// parity: SIDE-014, CMD-010
#[gtk::test]
fn a_pinned_folder_menu_is_windows_11_and_keeps_its_strip_nearest_the_pointer() {
    let _theme = crate::test_support::harness::ThemeGuard::keep();
    let fixture = Fixture::standard();
    let names = [
        "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliett", "Kilo",
        "Lima", "Mike", "November",
    ];
    for name in names {
        fs::create_dir(fixture.path(name)).expect("fixture subfolder");
    }
    let test = TestWindow::open(&fixture.uri());
    test.window.set_default_size(960, 980);
    wait_until("the taller window", || test.window.height() >= 940);
    for name in names {
        test.window.navigate(&fixture.uri_of(name)).expect("a folder");
        test.wait_for_listing(name);
        test.activate("pin-folder", None);
        wait_until("the pin", || {
            test.window.sidebar().labels().contains(&name.to_owned())
        });
    }
    test.window.navigate(&fixture.uri()).expect("the fixture");
    test.wait_for_listing("the fixture");
    let open_menu = |label: &str| {
        wait_until("the pin's menu", || {
            let menu = test.window.sidebar().right_click_row(label);
            let shown = menu.is_visible()
                && menu
                    .row_labels()
                    .first()
                    .is_some_and(|first| first == "Open in new tab");
            if !shown {
                menu.popdown();
            }
            shown
        });
        let menu = test.window.sidebar().right_click_row(label);
        crate::test_support::harness::wait_for_frames(&test.window, 3);
        menu
    };

    let menu = open_menu("Downloads");
    assert_eq!(
        menu.row_labels(),
        [
            "Open in new tab",
            "Open in new window",
            "Unpin from Quick access",
            "Compress to",
            "Copy as path",
            "New",
            "Properties",
            "-",
            "Open in Terminal",
            "-",
            "Show more options",
        ]
    );
    assert_eq!(menu.strip_labels(), ["Copy"]);
    let compress = menu
        .items()
        .into_iter()
        .find(|item| item.label == "Compress to")
        .expect("Compress to");
    let labels: Vec<String> = compress
        .submenu
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item.label.clone()),
            MenuEntry::Divider => None,
        })
        .collect();
    assert_eq!(labels, ["ZIP File", "TAR File", "Additional options"]);
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());

    let menu = open_menu("Alpha");
    assert_eq!(menu.strip_labels(), ["Cut", "Copy", "Rename", "Delete"]);
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());

    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        for position in StripPosition::ALL {
            choose_strip_position(&test, position);
            // Alpha, high in the pane, opens its menu downward; Downloads,
            // below the user's pins, opens it upward.
            for (label, slide, direction) in [("Alpha", Slide::Down, "down"), ("Downloads", Slide::Up, "up")]
            {
                let menu = open_menu(label);
                wait_until("the menu placed", || menu.slide() == Some(slide));
                let expected = position.at_bottom(slide == Slide::Up);
                assert_eq!(
                    menu.strip_at_bottom(),
                    expected,
                    "{position:?}, opening {direction}"
                );
                let name = position.as_str();
                crate::test_support::harness::capture_with_menus(
                    &test.window,
                    &format!("context-menu-match-strip-{name}-{direction}-{theme}.png"),
                );
                if position == StripPosition::Pointer {
                    let pinned = if direction == "down" {
                        "pinned-down"
                    } else {
                        "pinned-up"
                    };
                    crate::test_support::harness::capture_with_menus(
                        &test.window,
                        &format!("context-menu-match-{pinned}-{theme}.png"),
                    );
                }
                menu.popdown();
                wait_until("the menu to close", || !menu.is_mapped());
            }
        }
    }
}

/// A pinned folder's Copy as path, Compress to › ZIP File and TAR File
/// and New › Folder act on that folder, not on the file list's selection:
/// the path is copied in quotes, the ZIP and TAR are made beside the
/// folder, and New opens
/// the folder and makes the new folder there.
///
/// parity: SIDE-014, CLIP-013, ARC-023
#[gtk::test]
fn a_pinned_folder_menu_copies_compresses_and_creates_in_that_folder() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Pinned work")).expect("fixture subfolder");
    let pinned = fixture.uri_of("Pinned work");
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(0);

    test.activate("copy-path-of", Some(&pinned));
    let clipboard = test.window.clipboard();
    let copied = gtk::glib::MainContext::default().block_on(clipboard.read_text_future());
    let copied = copied.ok().flatten().map(|text| text.to_string());
    assert_eq!(
        copied,
        Some(format!("\"{}\"", fixture.path("Pinned work").display()))
    );

    test.activate("compress-to-zip-of", Some(&pinned));
    wait_until("the ZIP beside the folder", || {
        fixture.path("Pinned work.zip").exists()
    });
    test.wait_for_listing("the folder listed again");
    test.activate("compress-to-tar-of", Some(&pinned));
    wait_until("the TAR beside the folder", || {
        fs::read(fixture.path("Pinned work.tar"))
            .is_ok_and(|bytes| bytes.get(257..262) == Some(b"ustar".as_slice()))
    });
    test.wait_for_listing("the folder listed again");

    let new_folder =
        crate::window::place_commands::new_menu_in(&pinned, crate::window::command_bar::new_menu(Vec::new()));
    let MenuEntry::Item(folder_item) = &new_folder[0] else {
        panic!("New › Folder is an item");
    };
    assert_eq!(folder_item.label, "Folder");
    let target = folder_item
        .target
        .as_ref()
        .and_then(|target| target.str().map(str::to_owned))
        .expect("a text target");
    test.activate("new-in", Some(&target));
    wait_until("the pinned folder to open", || {
        test.window.current_uri().as_deref() == Some(pinned.as_str())
    });
    wait_until("the new folder inside it", || {
        fixture.path("Pinned work/New folder").is_dir()
    });
}

/// New in a pinned folder's menu still makes the item while a search
/// shows: one in that folder, or one kept from another folder ("Keep
/// search when changing folders"). The search ends first, as New is off
/// during a search.
///
/// parity: SIDE-014
#[gtk::test]
fn new_in_a_pinned_folder_ends_a_search_first() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Pinned work")).expect("fixture subfolder");
    fs::write(fixture.path("Pinned work/plan.txt"), "plan").expect("fixture file");
    let pinned = fixture.uri_of("Pinned work");
    let new_folder_in = |uri: &str| {
        let new_menu =
            crate::window::place_commands::new_menu_in(uri, crate::window::command_bar::new_menu(Vec::new()));
        let MenuEntry::Item(folder_item) = &new_menu[0] else {
            panic!("New › Folder is an item");
        };
        folder_item
            .target
            .as_ref()
            .and_then(|target| target.str().map(str::to_owned))
            .expect("a text target")
    };

    let test = TestWindow::open(&pinned);
    test.search_for("plan");
    assert!(test.window.is_searching());
    test.activate("new-in", Some(&new_folder_in(&pinned)));
    wait_until("the new folder inside it", || {
        fixture.path("Pinned work/New folder").is_dir()
    });
    assert!(!test.window.is_searching());

    test.activate("go-to", Some(&fixture.uri()));
    test.wait_for_listing("the first folder");
    test.search_for("notes");
    test.window.search_strip().set_keeps_search(true);
    test.activate("new-in", Some(&new_folder_in(&pinned)));
    wait_until("a second new folder in the pinned one", || {
        fixture.path("Pinned work/New folder (2)").is_dir()
    });
    assert_eq!(test.window.current_uri().as_deref(), Some(pinned.as_str()));
    test.window.search_strip().set_keeps_search(false);
}
