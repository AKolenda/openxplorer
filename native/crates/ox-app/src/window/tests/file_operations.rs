// SPDX-License-Identifier: AGPL-3.0-only
//! New, Rename, Delete, Duplicate, Undo and the transfer panel in a real
//! window, against `newItem`, `newTemplateDialog`, `rename`, `trash` and
//! `runOperation` of `desktop/ui/app.js`: the same dialogs, messages and
//! buttons, and the folder listed again afterwards. The tests that move
//! items to the Trash use the test run's private Recycle Bin.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::file_ops_support::{
    is_enabled, open_dialog, require_private_trash, select_names, text_field, wait_for_no_dialog,
};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::window::file_drop::DropAction;

/// parity: OPS-001, CMD-004
#[gtk::test]
fn new_folder_asks_for_a_name_then_creates_and_selects_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("new-folder", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "New folder");
    assert_eq!(dialog.message_text(), "Names must not contain slashes.");
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
    field.set_text("a/b");
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
        Some("Use a name without slashes or control characters.")
    );
    assert!(
        fixture.path("Documents/keep.txt").is_file(),
        "the folder was not replaced"
    );
    assert!(!fixture.path("a").exists());
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

/// parity: CMD-002
#[gtk::test]
fn new_is_disabled_where_nothing_can_be_created() {
    let test = TestWindow::open(Page::ThisPc.uri());

    assert!(!is_enabled(&test, "new-folder"));
    assert!(!is_enabled(&test, "new-file"));
    assert!(!is_enabled(&test, "paste"));
}

/// The field that edits a name in place in `test`'s view, once it shows.
fn name_editor(test: &TestWindow) -> gtk::Entry {
    let find = || {
        descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
            .into_iter()
            .find(|field| field.has_css_class("rename-field"))
    };
    wait_until("the name to become editable", || find().is_some());
    find().expect("wait_until returned only once the field showed")
}

/// Whether `test`'s view edits a name in place.
fn is_renaming_in_place(test: &TestWindow) -> bool {
    descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
        .iter()
        .any(|field| field.has_css_class("rename-field"))
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

/// parity: CMD-017
#[gtk::test]
fn the_file_keys_leave_text_fields_and_the_settings_page_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.window.folder_pane().focus_view();
    let in_file_list = test.window.file_keys_apply();
    test.window.search_box().focus();
    let in_search = test.window.file_keys_apply();
    test.activate("settings", None);
    let on_settings = test.window.file_keys_apply();

    assert!(in_file_list);
    assert!(
        !in_search,
        "the search field keeps Delete, F2 and the clipboard keys"
    );
    assert!(!on_settings);
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

/// parity: OPS-009
#[gtk::test]
fn rename_does_nothing_with_several_items_selected() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    assert!(!is_enabled(&test, "rename"));
}

/// parity: OPS-015, OPS-018, OPS-023, OPS-029
#[gtk::test]
fn delete_asks_then_moves_to_the_trash_and_undo_restores() {
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

    test.activate("undo", None);
    wait_until("the file to come back", || fixture.path("Résumé.txt").is_file());
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

/// parity: OPS-016
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
