// SPDX-License-Identifier: AGPL-3.0-only
//! New, Rename, Delete, Duplicate, Undo and the transfer panel in a real
//! window, against `newItem`, `newTemplateDialog`, `rename`, `trash` and
//! `runOperation` of `v2.0.0:desktop/ui/app.js`: the same dialogs, messages and
//! buttons, and the folder listed again afterwards. The tests that move
//! items to the Trash use the test run's private Recycle Bin.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{PreferencesUpdate, Settings};
use ox_core::transfer::{Cancellation, Progress, ProgressScope};

use super::file_ops_support::{
    dialog_over, is_enabled, is_renaming_in_place, name_editor, open_dialog, require_private_trash,
    select_names, text_field, wait_for_no_dialog,
};
use crate::dialog::Dialog;
use crate::locations::Page;
use crate::test_support::harness::{application, descendants, wait_until, Fixture, TestWindow};
use crate::window::file_drop::DropAction;
use crate::window::transfer_panel::TransferKind;

/// parity: OPS-001, CMD-004
#[gtk::test]
fn new_folder_asks_for_a_name_then_creates_and_selects_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("new-folder", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "New folder");
    assert_eq!(
        dialog.message_text(),
        "A slash makes a folder inside the one before it."
    );
    assert_eq!(dialog.button_labels(), ["Cancel", "Save"]);
    let field = text_field(&dialog);
    assert_eq!(field.text(), "New folder");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 10)),
        "the whole name is selected"
    );
    dialog.press("Save");
    wait_for_no_dialog(&test);
    wait_until("the new folder to be listed and selected", || {
        test.selected_names() == ["New folder"]
    });
    assert!(fixture.path("New folder").is_dir());
}

/// parity: OPS-006, OPS-008
#[gtk::test]
fn a_refused_name_stays_in_the_dialog_and_nothing_is_overwritten() {
    let fixture = Fixture::standard();
    fixture.write("Documents/keep.txt");
    let test = TestWindow::open(&fixture.uri());
    test.activate("new-folder", None);
    let dialog = open_dialog(&test);
    let field = text_field(&dialog);

    field.set_text("Documents");
    dialog.press("Save");
    wait_until("the refusal", || dialog.error_text().is_some());
    let taken = dialog.error_text();
    field.set_text("a\\b");
    dialog.press("Save");
    wait_until("the name check", || dialog.error_text() != taken);
    let invalid = dialog.error_text();
    dialog.press("Cancel");
    wait_for_no_dialog(&test);

    assert_eq!(
        taken.as_deref(),
        Some("An item named “Documents” already exists. Nothing was overwritten.")
    );
    assert_eq!(
        invalid.as_deref(),
        Some("Use names without backslashes or control characters, one slash between folders.")
    );
    assert!(
        fixture.path("Documents/keep.txt").is_file(),
        "the folder was not replaced"
    );
    assert!(!fixture.path("a\\b").exists());
}

/// parity: OPS-002
#[gtk::test]
fn a_new_menu_file_starts_from_its_template_and_is_created_from_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("new-markdown-document", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "New from template");
    assert_eq!(
        dialog.message_text(),
        "Create a new copy without changing the template."
    );
    assert_eq!(dialog.button_labels(), ["Cancel", "Create"]);
    assert_eq!(text_field(&dialog).text(), "New document.md");
    dialog.press("Create");
    wait_for_no_dialog(&test);
    wait_until("the new file to be selected", || {
        test.selected_names() == ["New document.md"]
    });
    let contents = fs::read_to_string(fixture.path("New document.md")).expect("the new file");
    assert_eq!(contents, "# New document\n");
}

/// parity: OPS-004
#[gtk::test]
fn new_link_asks_for_the_path_and_selects_the_new_link() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("new-link", None);
    let dialog = open_dialog(&test);
    let fields = descendants::<gtk::Entry>(&dialog);
    let (target, name) = (&fields[0], &fields[1]);
    target.set_text(&fixture.path("Documents").to_string_lossy());
    dialog.press("Create");
    wait_until("the refusal", || dialog.error_text().is_some());
    let taken = dialog.error_text();
    name.set_text("Documents link");
    dialog.press("Create");
    wait_for_no_dialog(&test);

    assert_eq!(dialog.title_text(), "New link");
    assert_eq!(
        taken.as_deref(),
        Some("An item named “Documents” already exists. Nothing was overwritten.")
    );
    wait_until("the link to be selected", || {
        test.selected_names() == ["Documents link"]
    });
    let points_to = fs::read_link(fixture.path("Documents link")).expect("a symbolic link");
    assert_eq!(points_to, fixture.path("Documents"));
}

/// parity: CMD-002
#[gtk::test]
fn new_is_disabled_where_nothing_can_be_created() {
    let test = TestWindow::open(Page::ThisPc.uri());

    assert!(!is_enabled(&test, "new-folder"));
    assert!(!is_enabled(&test, "new-file"));
    assert!(!is_enabled(&test, "paste"));
}

/// parity: OPS-009, OPS-010, OPS-029, OPS-031
#[gtk::test]
fn rename_edits_the_name_in_place_and_undo_and_redo_walk_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.activate("rename", None);
    let field = name_editor(&test);

    assert_eq!(field.text(), "Notes 2.txt");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 7)),
        "the extension stays unselected"
    );
    field.set_text("Plans.txt");
    field.emit_activate();
    wait_until("the renamed file to be selected", || {
        test.selected_names() == ["Plans.txt"]
    });
    assert!(!is_renaming_in_place(&test));
    assert!(fixture.path("Plans.txt").is_file());
    assert!(!fixture.path("Notes 2.txt").exists());

    test.activate("undo", None);
    wait_until("the rename to be undone", || {
        fixture.path("Notes 2.txt").is_file()
    });
    assert!(!fixture.path("Plans.txt").exists());
    wait_until("the undone toast", || {
        test.window.shown_message() == "Rename undone."
    });
    test.activate("redo", None);
    wait_until("the rename to be redone", || fixture.path("Plans.txt").is_file());
    assert!(!fixture.path("Notes 2.txt").exists());
    wait_until("the redone toast", || {
        test.window.shown_message() == "Rename redone."
    });
}

/// The file keys work only in the file pane; the keys that act on the
/// window (F5, Ctrl+L, Ctrl+F, Alt+Enter, Ctrl+comma and the text-size
/// keys) are application accelerators, which GTK runs from any focus.
///
/// parity: CMD-017
#[gtk::test]
fn the_file_keys_leave_text_fields_and_the_settings_page_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.window.folder_pane().focus_view();
    let in_file_list = test.window.file_keys_apply();
    test.window.search_box().focus();
    let in_search = test.window.file_keys_apply();
    let has_crumb = descendants::<gtk::Button>(test.window.address_bar())
        .iter()
        .any(WidgetExt::grab_focus);
    assert!(has_crumb, "the address bar has a crumb to focus");
    let on_crumb = test.window.file_keys_apply();
    assert!(test.window.workspace().grab_focus(), "the splitter takes focus");
    let on_splitter = test.window.file_keys_apply();
    test.activate("settings", None);
    let on_settings = test.window.file_keys_apply();

    assert!(in_file_list);
    assert!(
        !in_search,
        "the search field keeps Delete, F2 and the clipboard keys"
    );
    assert!(!on_crumb, "the crumbs keep their keys");
    assert!(!on_splitter, "the splitter keeps its keys");
    assert!(!on_settings);
    let app = test.window.application().expect("the window has an application");
    for (action, key) in [
        ("win.refresh", "F5"),
        ("win.location", "<Control>l"),
        ("win.search", "<Control>f"),
        ("win.properties", "<Alt>Return"),
        ("win.settings", "<Control>comma"),
    ] {
        let keys = app.accels_for_action(action);
        assert!(keys.iter().any(|shown| shown == key), "{action}: {keys:?}");
    }
    assert!(
        app.accels_for_action("win.context-menu").is_empty(),
        "Menu and Shift+F10 belong to the file views; a text field keeps its own menu"
    );
}

/// parity: OPS-006, OPS-008, OPS-010
#[gtk::test]
fn a_refused_name_in_place_keeps_the_field_for_another_try() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text("Notes 10.txt");
    field.emit_activate();
    let taken = "An item named “Notes 10.txt” already exists. Nothing was overwritten.";
    wait_until("the refusal", || test.window.shown_message() == taken);
    wait_until("the field to come back", || field.is_sensitive());
    field.set_text("a/b");
    field.emit_activate();
    let invalid = test.window.shown_message();
    field.set_text("Notes 2.txt");
    field.emit_activate();

    wait_until("the unchanged name to end the rename", || {
        !is_renaming_in_place(&test)
    });
    assert_eq!(invalid, "Use a name without slashes or control characters.");
    assert!(fixture.path("Notes 2.txt").is_file());
    let kept = fs::read(fixture.path("Notes 10.txt")).expect("the other file stays");
    assert_eq!(kept, b"Synthetic test data\n");
}

/// parity: OPS-009
#[gtk::test]
fn an_item_that_is_not_on_screen_is_renamed_with_the_dialog() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["file 0399.txt"]);

    test.activate("rename", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Rename");
    assert_eq!(dialog.message_text(), "Names must not contain slashes.");
    assert_eq!(text_field(&dialog).text(), "file 0399.txt");
    assert_eq!(dialog.button_labels(), ["Cancel", "Save"]);
    dialog.press("Cancel");
    wait_for_no_dialog(&test);
}

/// parity: OPS-014, OPS-029, OPS-032
#[gtk::test]
fn several_selected_items_are_renamed_with_one_numbered_name_and_undone_together() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("rename", None);
    let dialog = open_dialog(&test);
    let field = text_field(&dialog);

    assert_eq!(dialog.title_text(), "Rename items");
    assert_eq!(dialog.message_text(), "Rename the 2 selected items to:");
    assert_eq!(field.text(), "New name #");
    assert_eq!(field.selection_bounds(), Some((0, 9)), "the number stays");
    field.set_text("Notes");
    dialog.press("Rename");
    wait_until("the refusal", || dialog.error_text().is_some());
    field.set_text("Plan #");
    dialog.press("Rename");
    wait_for_no_dialog(&test);
    wait_until("the renamed files to be selected", || {
        test.selected_names() == ["Plan 1.txt", "Plan 2.txt"]
    });
    let first = fs::read_to_string(fixture.path("Plan 1.txt")).expect("renamed in view order");
    assert_eq!(test.window.shown_message(), "2 item(s) renamed.");

    test.window.imp().toast.get().press_action();
    wait_until("the batch to be renamed back", || {
        fixture.path("Notes 2.txt").is_file() && fixture.path("Notes 10.txt").is_file()
    });
    assert_eq!(first, "Synthetic test data\n");
    assert!(!fixture.path("Plan 1.txt").exists());
}

/// parity: OPS-015, OPS-018, OPS-023, OPS-029, OPS-032
#[gtk::test]
fn delete_asks_then_moves_to_the_trash_and_the_toasts_undo_restores() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);

    test.activate("trash", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Move to Trash?");
    assert_eq!(
        dialog.message_text(),
        "Résumé.txt\n\nItems go to the Trash and can be restored from there."
    );
    assert_eq!(dialog.button_labels(), ["Cancel", "Move to Trash"]);
    dialog.press("Move to Trash");
    wait_until("the file to leave the folder", || {
        !fixture.path("Résumé.txt").exists() && !test.names().contains(&"Résumé.txt".to_owned())
    });
    assert_eq!(test.window.shown_message(), "1 item(s) sent to Trash.");
    let toast = test.window.imp().toast.get();
    assert_eq!(toast.action_label().as_deref(), Some("Undo"));

    toast.press_action();
    wait_until("the file to come back", || fixture.path("Résumé.txt").is_file());
    assert_eq!(toast.action_label(), None, "the step is undone");
}

/// With "Ask before moving items to the Recycle Bin" off, Delete trashes
/// at once, and with "Ask before deleting permanently" off Shift+Delete
/// deletes at once; with "Ask before closing a window with several tabs"
/// on, closing a window with two tabs asks first, once however often it
/// is asked, and "Close all tabs" closes it.
///
/// parity: SET-010, TAB-051
#[gtk::test]
fn the_confirmation_settings_decide_what_asks() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let update = PreferencesUpdate {
        confirm_trash: Some(false),
        confirm_delete: Some(false),
        confirm_close_tabs: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choices");
    test.context.reload_settings();
    wait_until("the window to read the choices", || {
        !test.context.settings_data().preferences.confirm_trash
    });
    select_names(&test, &["Résumé.txt"]);

    test.activate("trash", None);
    wait_until("the file to go to the Trash unasked", || {
        !fixture.path("Résumé.txt").exists()
    });
    select_names(&test, &["Notes 10.txt"]);
    test.activate("delete-permanently", None);
    wait_until("the file to be deleted unasked", || {
        !fixture.path("Notes 10.txt").exists()
    });
    assert!(dialog_over(&test).is_none(), "nothing asked");

    test.activate("new-tab", None);
    test.window.close();
    test.window.close();
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Close all tabs?");
    let over_window = Some(test.window.upcast_ref::<gtk::Window>());
    let questions = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<Dialog>().ok())
        .filter(|dialog| dialog.is_visible() && dialog.transient_for().as_ref() == over_window)
        .count();
    assert_eq!(questions, 1, "a second close asks no second question");
    dialog.press("Cancel");
    wait_for_no_dialog(&test);
    assert!(test.window.is_visible(), "the window stays open");

    test.window.close();
    open_dialog(&test).press("Close all tabs");
    wait_until("the window to close", || !test.window.is_visible());
}

/// parity: OPS-015
#[gtk::test]
fn cancelling_the_delete_confirmation_keeps_the_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("trash", None);
    let dialog = open_dialog(&test);
    let message = dialog.message_text();
    dialog.press("Cancel");
    wait_for_no_dialog(&test);

    assert!(message.starts_with("2 selected items\n\n"), "{message}");
    assert!(fixture.path("Notes 2.txt").is_file());
    assert!(fixture.path("Notes 10.txt").is_file());
}

/// parity: OPS-016, SEL-017
#[gtk::test]
fn shift_delete_deletes_permanently_after_its_own_confirmation() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Documents"]);

    test.activate("delete-permanently", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete permanently?");
    assert_eq!(
        dialog.message_text(),
        "Documents\n\nThe items are deleted permanently, without the Trash, and cannot be recovered."
    );
    assert_eq!(dialog.button_labels(), ["Cancel", "Delete permanently"]);
    dialog.press("Delete permanently");
    wait_until("the folder to be deleted", || !fixture.path("Documents").exists());
    wait_until("the toast", || {
        test.window.shown_message() == "1 item(s) permanently deleted."
    });
    wait_until("the next item to be selected", || {
        test.selected_names() == ["Notes 2.txt"]
    });
}

/// The command bar's Delete button, the button that runs Delete.
fn delete_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(test.window.command_bar())
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.trash"))
        .expect("the command bar has Delete")
}

/// Clicking Delete in the command bar with Shift held deletes
/// permanently, after Shift+Delete's confirmation, as in Windows Explorer.
///
/// parity: OPS-016
#[gtk::test]
fn shift_clicking_delete_in_the_bar_deletes_permanently_after_asking() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.window.hold_shift_for_tests(true);
    delete_button(&test).emit_clicked();
    test.window.hold_shift_for_tests(false);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete permanently?");
    assert_eq!(dialog.button_labels(), ["Cancel", "Delete permanently"]);
    dialog.press("Delete permanently");
    wait_until("the file to be deleted", || !fixture.path("Notes 2.txt").exists());
    wait_until("the toast", || {
        test.window.shown_message() == "1 item(s) permanently deleted."
    });
}

/// Choosing Delete in the right-click menu with Shift held asks to delete
/// permanently; Cancel keeps the file.
///
/// parity: OPS-016
#[gtk::test]
fn shift_choosing_delete_in_the_right_click_menu_asks_to_delete_permanently() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .right_click(Some(super::context_menus::position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    wait_until("the menu", || menu.is_visible());

    test.window.hold_shift_for_tests(true);
    menu.row("Move to Trash").emit_activate();
    test.window.hold_shift_for_tests(false);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete permanently?");
    dialog.press("Cancel");
    wait_for_no_dialog(&test);
    assert!(fixture.path("Notes 2.txt").is_file(), "Cancel keeps the file");
}

/// The folder tree's Move to Trash with Shift held asks to delete the
/// folder permanently.
///
/// parity: OPS-016, SIDE-028
#[gtk::test]
fn shift_with_the_folder_trees_move_to_trash_asks_to_delete_permanently() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folder = fixture.uri_of("Documents");

    test.window.hold_shift_for_tests(true);
    test.activate("trash-folder", Some(&folder));
    test.window.hold_shift_for_tests(false);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete permanently?");
    dialog.press("Cancel");
    wait_for_no_dialog(&test);
    assert!(fixture.path("Documents").is_dir(), "Cancel keeps the folder");
}

/// Without Shift, clicking Delete in the command bar still moves to the
/// Trash.
///
/// parity: OPS-015
#[gtk::test]
fn clicking_delete_in_the_bar_without_shift_moves_to_the_trash() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    delete_button(&test).emit_clicked();
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Move to Trash?");
    dialog.press("Cancel");
    wait_for_no_dialog(&test);
}

/// parity: OPS-034, OPS-029
#[gtk::test]
fn duplicate_copies_next_to_the_item_and_selects_the_copy() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.activate("duplicate", None);
    wait_until("the copy to be selected", || {
        let selected = test.selected_names();
        selected.len() == 1 && selected[0].starts_with("Notes 2 (copy")
    });
    let copy = test.selected_names().remove(0);
    assert!(fixture.path(&copy).is_file());
    assert!(fixture.path("Notes 2.txt").is_file());
    assert_eq!(test.window.shown_message(), "1 item(s) duplicated.");

    test.activate("undo", None);
    wait_until("the copy to go to the Trash", || !fixture.path(&copy).exists());
}

/// parity: OPS-019, OPS-022, OPS-024
#[gtk::test]
fn the_transfer_panel_shows_the_running_operation_and_cancel_stops_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.imp().transfer_panel.get();

    let context = test.window.begin_operation("Preparing copy…");
    let second = test.window.begin_operation("Moving items…");

    let context = context.expect("the first operation starts");
    assert!(second.is_none(), "one operation at a time");
    assert!(panel.is_visible());
    assert_eq!(panel.status_text(), "Preparing copy…");
    assert!(
        !is_enabled(&test, "trash"),
        "file commands wait for the operation"
    );
    assert!(is_enabled(&test, "cancel-operation"));
    test.activate("cancel-operation", None);
    assert!(context.cancel.is_cancelled());
    assert_eq!(panel.status_text(), "Cancelling…");
    test.window.end_operation();
    assert!(!panel.is_visible());
    assert!(!is_enabled(&test, "cancel-operation"));
}

/// A copied file's bytes fill a bar of their own: a full file bar leaves
/// the batch bar where the batch is.
///
/// parity: OPS-020
#[gtk::test]
fn a_full_file_bar_is_never_shown_as_the_batch_finishing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.imp().transfer_panel.get();
    let report = |label: &str, fraction: f64, scope: ProgressScope| Progress {
        label: label.to_owned(),
        fraction,
        scope,
        bytes: None,
    };
    let _context = test.window.begin_operation("Preparing copy…");

    panel.show_progress(&report("Copy: a.txt (1/2)", 0.0, ProgressScope::Batch));
    panel.show_progress(&report("Copying a.txt · 10 / 10 bytes", 1.0, ProgressScope::File));
    let file_done = panel.fractions();
    panel.show_progress(&report("Copy: b.txt (2/2)", 0.5, ProgressScope::Batch));
    let next_item = panel.fractions();
    test.window.end_operation();

    assert_eq!(file_done, (0.0, Some(1.0)));
    assert_eq!(next_item, (0.5, None));
}

/// parity: TAB-049
#[gtk::test]
fn closing_during_an_operation_asks_and_closes_only_once_it_stopped() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let context = test
        .window
        .begin_operation("Preparing copy…")
        .expect("the operation starts");

    test.window.close();
    let question = open_dialog(&test);
    let title = question.title_text();
    let buttons = question.button_labels();
    question.press("Keep open");
    wait_for_no_dialog(&test);
    let kept = test.window.is_visible() && !context.cancel.is_cancelled();
    test.window.close();
    open_dialog(&test).press("Cancel and close");
    wait_until("the operation to be cancelled", || context.cancel.is_cancelled());
    let open_while_running = test.window.is_visible();
    test.window.end_operation();

    assert_eq!(title, "A file operation is running");
    assert_eq!(buttons, ["Keep open", "Cancel and close"]);
    assert!(kept, "Keep open changes nothing");
    assert!(open_while_running, "the window waits for the operation to stop");
    wait_until("the window to close", || !test.window.is_visible());
}

/// While an operation runs its panel holds the session's logout and
/// suspend inhibitor. When it ends in the window that has focus, the
/// toast is enough; a window in the background would also notify the
/// desktop (`background_notice.rs`).
///
/// parity: INT-028
#[gtk::test]
fn a_running_operation_inhibits_logout_and_a_focused_one_only_shows_the_toast() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.imp().transfer_panel.get();
    crate::window::background_notice::take_sent();

    let context = test.window.begin_operation("Preparing copy…");
    assert!(context.is_some() && panel.inhibits_logout());
    test.window.end_operation();
    assert!(!panel.inhibits_logout());

    select_names(&test, &["Notes 2.txt"]);
    test.activate("duplicate", None);
    wait_until("the toast", || {
        test.window.shown_message() == "1 item(s) duplicated."
    });
    assert!(test.window.is_active(), "the test window has focus");
    assert!(crate::window::background_notice::take_sent().is_empty());
}

/// While a file operation or an archive operation runs, the transfer
/// panel tells the dock: an `Update` with the bar shown when it starts and with the bar
/// hidden when it ends.
///
/// parity: INT-027
#[gtk::test]
fn running_operations_show_their_progress_on_the_dock_icon() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let connection = application().dbus_connection().expect("the test bus");
    let updates: Rc<RefCell<Vec<bool>>> = Rc::default();
    let recorded = Rc::clone(&updates);
    let _subscription = connection.subscribe_to_signal(
        None,
        Some("com.canonical.Unity.LauncherEntry"),
        Some("Update"),
        None,
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let properties = signal.parameters.child_value(1);
            let visible = glib::VariantDict::new(Some(&properties))
                .lookup::<bool>("progress-visible")
                .ok()
                .flatten();
            recorded.borrow_mut().push(visible.unwrap_or_default());
        },
    );

    let context = test.window.begin_operation("Preparing copy…");
    assert!(context.is_some());
    wait_until("the bar on the icon", || *updates.borrow() == [true]);
    test.window.end_operation();
    wait_until("the bar hidden", || *updates.borrow() == [true, false]);

    test.window.transfer_panel().start(
        TransferKind::Archive,
        "Preparing extraction…",
        Cancellation::new(),
    );
    wait_until("the archive operation's bar", || updates.borrow().len() == 3);
    test.window.finish_archive_operation();
    wait_until("the bar hidden again", || {
        *updates.borrow() == [true, false, true, false]
    });
}

/// A drop into a subfolder that ends while a window outside the
/// application has focus, as another app's would, sends one notification
/// with the toast's words. Clicking it brings back the window that ran
/// it; its Show button opens the subfolder there with the copy selected.
///
/// parity: INT-026
#[gtk::test]
fn an_operation_ending_in_the_background_notifies_the_desktop() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    crate::window::background_notice::take_sent();
    // Not added to the application: it stands in for another app's window.
    let other = gtk::Window::new();
    other.present();
    wait_until("the other window has focus", || {
        other.is_active() && !test.window.is_active()
    });

    let taken = test.window.drop_files(
        &[source.uri_of("Notes 2.txt")],
        Some(test.position_of("Documents")),
        DropAction::Copy,
    );
    assert!(taken);
    wait_until("the notice", || {
        crate::window::background_notice::SENT.with(|sent| !sent.borrow().is_empty())
    });

    let sent = crate::window::background_notice::take_sent();
    other.destroy();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, None);
    assert_eq!(sent[0].window, test.window.id());
    let copy = fixture.uri_of("Documents/Notes 2.txt");
    assert_eq!(sent[0].destination.items, std::slice::from_ref(&copy));
    let (action, target) = sent[0].show_action();
    assert_eq!(action, "app.show-destination");
    let (id, folder, items) = target
        .get::<(u32, String, Vec<String>)>()
        .expect("the Show target");
    assert_eq!(id, test.window.id());
    assert_eq!(folder, "");
    assert_eq!(items, [copy]);

    test.window.show_destination(None, &items);
    wait_until("the subfolder with the copy selected", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
            && test.selected_names() == ["Notes 2.txt"]
    });
}
