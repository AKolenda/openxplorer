// SPDX-License-Identifier: AGPL-3.0-only
//! New ▸ Text document in a real window, as Windows Explorer makes one:
//! the file is created at once as "New Text Document.txt", or
//! "New Text Document (2).txt" and so on when that is taken, then
//! selected and renamed in place with only its name before ".txt"
//! selected. The other New items keep their dialogs.

use std::fs;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    dialog_over, is_renaming_in_place, name_editor, open_dialog, require_private_trash, text_field,
    wait_for_no_dialog,
};
use crate::test_support::harness::{settle, wait_for, wait_until, Fixture, TestWindow};

/// Presses `key` in the in-place name field `editor`, as far as its own
/// key handler goes.
fn press_in(editor: &gtk::Entry, key: gdk::Key) {
    let keys = editor
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .last()
        .expect("the field has its own keys");
    let _: bool = keys.emit_by_name(
        "key-pressed",
        &[&key.into_glib(), &0_u32, &gdk::ModifierType::empty()],
    );
}

/// Runs New ▸ Text document and returns the field that renames the new
/// file.
fn make_new_text_document(test: &TestWindow) -> gtk::Entry {
    test.activate("new-text-document", None);
    let field = name_editor(test);
    settle();
    assert!(dialog_over(test).is_none(), "no dialog asks for a name");
    field
}

/// Escape in `field`, then waits for the rename to end.
fn escape(test: &TestWindow, field: &gtk::Entry) {
    press_in(field, gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(test));
}

/// parity: OPS-002, OPS-010, CMD-004
#[gtk::test]
fn new_text_document_makes_the_file_at_once_and_renames_its_name_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_text_document(&test);

    let created = fixture.path("New Text Document.txt");
    assert!(created.is_file(), "made before any name is typed");
    assert_eq!(
        fs::read(&created).expect("the new file"),
        ox_core::ops::BuiltinTemplate::Text.contents(),
        "the Text document starter's content"
    );
    assert_eq!(test.selected_names(), ["New Text Document.txt"]);
    assert_eq!(field.text(), "New Text Document.txt");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 17)),
        "only the name before .txt is selected"
    );
    field.set_text("Shopping list.txt");
    field.emit_activate();
    wait_until("the rename", || fixture.path("Shopping list.txt").is_file());
    assert!(!created.exists());
    wait_until("the renamed file to be selected", || {
        test.selected_names() == ["Shopping list.txt"]
    });
    assert!(!is_renaming_in_place(&test));
    assert!(dialog_over(&test).is_none());
}

/// Taken names are never overwritten: the next ones are
/// "New Text Document (2).txt" and "New Text Document (3).txt", as in
/// Windows.
///
/// parity: OPS-002, OPS-008
#[gtk::test]
fn taken_names_give_the_next_numbered_text_documents() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("New Text Document.txt"), b"kept").expect("a taken name");
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_text_document(&test);
    assert_eq!(field.text(), "New Text Document (2).txt");
    assert_eq!(field.selection_bounds(), Some((0, 21)), "the name before .txt");
    escape(&test, &field);
    let field = make_new_text_document(&test);
    assert_eq!(field.text(), "New Text Document (3).txt");
    escape(&test, &field);

    assert!(fixture.path("New Text Document (2).txt").is_file());
    assert!(fixture.path("New Text Document (3).txt").is_file());
    assert_eq!(
        fs::read(fixture.path("New Text Document.txt")).expect("the taken one"),
        b"kept",
        "the taken one is left alone"
    );
}

/// Escape ends the rename and keeps the default name.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn escape_keeps_the_name_new_text_document() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Typed but cancelled.txt");

    escape(&test, &field);

    assert!(fixture.path("New Text Document.txt").is_file());
    assert!(!fixture.path("Typed but cancelled.txt").exists());
    assert_eq!(test.selected_names(), ["New Text Document.txt"]);
}

/// Clicking elsewhere, which takes focus from the field, keeps the typed
/// name, as the existing rename in place does.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn leaving_the_field_keeps_the_typed_name() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Ideas.txt");

    test.window.folder_pane().focus_view();

    wait_until("the rename", || fixture.path("Ideas.txt").is_file());
    assert!(!fixture.path("New Text Document.txt").exists());
}

/// A listing of the folder that arrives once the rename has started,
/// such as the one the folder's own change notices ask for, leaves the
/// field open with the name as typed.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn a_listing_that_arrives_during_the_rename_keeps_the_field() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Draft.txt");

    test.activate("refresh", None);
    test.wait_for_listing("the folder listed again");
    wait_for(Duration::from_millis(300));

    assert!(is_renaming_in_place(&test), "the rename goes on");
    let field = name_editor(&test);
    assert_eq!(field.text(), "Draft.txt");
    field.emit_activate();
    wait_until("the rename", || fixture.path("Draft.txt").is_file());
    assert!(!fixture.path("New Text Document.txt").exists());
}

/// Undo takes the new file away again, to the Recycle Bin, as it does a
/// new folder.
///
/// parity: OPS-029
#[gtk::test]
fn undo_takes_the_new_text_document_away() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    escape(&test, &field);

    test.activate("undo", None);

    wait_until("the file to go", || {
        !fixture.path("New Text Document.txt").exists()
    });
}

/// Every other New item still asks first, in the dialog it opened before,
/// with its own suggested name.
///
/// parity: OPS-002
#[gtk::test]
fn the_other_new_items_still_open_their_dialogs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let before = test.names();

    for (action, target, title, name) in [
        ("new-file", None, "New file", "New file"),
        (
            "new-markdown-document",
            None,
            "New from template",
            "New document.md",
        ),
        ("new-csv-file", None, "New from template", "New spreadsheet.csv"),
        ("new-json-file", None, "New from template", "New file.json"),
        ("new-html-document", None, "New from template", "New page.html"),
        ("new-from-template", None, "New from template", "New document.txt"),
        (
            "new-from-user-template",
            Some("text"),
            "New from template",
            "New document.txt",
        ),
    ] {
        test.activate(action, target);
        let dialog = open_dialog(&test);
        assert_eq!(dialog.title_text(), title, "{action}");
        let field = text_field(&dialog);
        assert_eq!(field.text(), name, "{action}");
        assert!(!is_renaming_in_place(&test), "{action}");
        dialog.press("Cancel");
        wait_for_no_dialog(&test);
    }

    assert_eq!(test.names(), before, "nothing was created");
}
