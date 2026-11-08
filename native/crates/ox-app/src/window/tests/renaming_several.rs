// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming several selected items at once in a real window, as Windows
//! Explorer does (OPS-050): F2 edits the focused item's name in place,
//! and the one name typed there goes to every selected item with
//! "(1)", "(2)" and so on, in the order shown, each keeping its own
//! extension.

use std::fs;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    assert_ended_without_a_message, dialog_over, failure_question, is_renaming_in_place, name_editor,
    open_dialog, select_names, text_field, wait_for_no_dialog,
};
use crate::dialog::Dialog;
use crate::test_support::harness::{descendants, settle, wait_until, Fixture, TestWindow};

/// Selects the items called `names` and gives `focused` keyboard focus,
/// as a Ctrl+click on it last does.
fn select_with_focus(test: &TestWindow, names: &[&str], focused: &str) {
    select_names(test, names);
    let pane = test.window.folder_pane();
    let model = pane.model();
    let position = (0..model.n_items())
        .find(|&position| model.name_at(position).as_deref() == Some(focused))
        .expect("the focused item is listed");
    pane.focus_item(position);
    wait_until("the item to have focus", || {
        pane.focused_position() == Some(position)
    });
}

/// How many name fields `test`'s view shows.
fn name_editor_count(test: &TestWindow) -> usize {
    descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
        .iter()
        .filter(|field| field.has_css_class("rename-field"))
        .count()
}

/// Presses Escape in `editor` as far as its own key handler goes.
fn press_escape(editor: &gtk::Entry) {
    use gtk::glib::translate::IntoGlib;
    let keys = editor
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .last()
        .expect("the field has its own keys");
    let modifiers = gdk::ModifierType::empty();
    let handled: bool = keys.emit_by_name(
        "key-pressed",
        &[&gdk::Key::Escape.into_glib(), &0_u32, &modifiers],
    );
    assert!(handled, "the field handled Escape");
}

/// The text of `name` in the fixture.
fn content(fixture: &Fixture, name: &str) -> String {
    fs::read_to_string(fixture.path(name)).unwrap_or_else(|_| panic!("{name} is a readable file"))
}

/// F2 with several items selected edits only the focused item's name;
/// Enter gives every selected item the typed name with a number, from
/// (1) in the order shown, folders first as listed, each file with its
/// own extension. The renamed items are selected without a message, and
/// one Undo renames them all back; Redo renames them again.
///
/// parity: OPS-050, OPS-029
#[gtk::test]
fn several_selected_items_get_the_typed_name_numbered_in_view_order() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("README"), "read me").expect("the file is written");
    fs::write(fixture.path("Map.jpg"), "a map").expect("the file is written");
    fs::write(fixture.path("Documents/inside.txt"), "inside").expect("the file is written");
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    let order = ["Documents", "Map.jpg", "Notes 2.txt", "Notes 10.txt", "README"];
    let shown: Vec<String> = test
        .names()
        .into_iter()
        .filter(|name| order.contains(&name.as_str()))
        .collect();
    assert_eq!(shown, order, "the order shown");
    select_with_focus(&test, &order, "Notes 10.txt");
    fs::write(fixture.path("Notes 10.txt"), "ten").expect("the file is written");

    test.activate("rename", None);
    let field = name_editor(&test);

    assert_eq!(field.text(), "Notes 10.txt", "the focused item is edited");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 8)),
        "the extension stays unselected"
    );
    assert_eq!(name_editor_count(&test), 1);
    assert!(dialog_over(&test).is_none(), "no Rename items dialog");
    field.set_text("Plan.txt");
    field.emit_activate();
    let renamed = [
        "Plan (1)",
        "Plan (2).jpg",
        "Plan (3).txt",
        "Plan (4).txt",
        "Plan (5)",
    ];
    wait_until("the renamed items to be selected", || {
        test.selected_names() == renamed
    });
    assert_ended_without_a_message(&test);
    assert!(!is_renaming_in_place(&test));
    assert_eq!(content(&fixture, "Plan (1)/inside.txt"), "inside");
    assert_eq!(content(&fixture, "Plan (2).jpg"), "a map");
    assert_eq!(content(&fixture, "Plan (4).txt"), "ten");
    assert_eq!(content(&fixture, "Plan (5)"), "read me");
    assert!(fixture.path("Résumé.txt").is_file(), "not selected, so kept");

    test.activate("undo", None);
    wait_until("the whole rename to be undone", || {
        order.iter().all(|name| fixture.path(name).exists())
    });
    assert_ended_without_a_message(&test);
    assert!(renamed.iter().all(|name| !fixture.path(name).exists()));
    test.activate("redo", None);
    wait_until("the whole rename to be redone", || {
        renamed.iter().all(|name| fixture.path(name).exists())
    });
    assert_ended_without_a_message(&test);
    assert_eq!(content(&fixture, "Plan (4).txt"), "ten");
}

/// A dotted name gives a file without an extension one ("README" becomes
/// "Plan.v1 (2)"), so the extension question asks first, as for one item.
/// Cancel keeps the field with the typed name and renames nothing.
///
/// parity: OPS-050, OPS-049
#[gtk::test]
fn a_file_without_an_extension_gaining_one_asks_first() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("README"), "read me").expect("the file is written");
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_with_focus(&test, &["Notes 2.txt", "README"], "Notes 2.txt");
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text("Plan.v1.txt");
    field.emit_activate();
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Change the extension?");
    question.press("Cancel");
    wait_for_no_dialog(&test);
    wait_until("the field to come back", || field.is_sensitive());
    assert!(is_renaming_in_place(&test), "editing goes on");
    assert_eq!(field.text(), "Plan.v1.txt", "the typed name is kept");
    assert!(fixture.path("README").is_file());
    assert!(fixture.path("Notes 2.txt").is_file());

    field.emit_activate();
    open_dialog(&test).press("Change extension");
    wait_until("the rename", || fixture.path("Plan.v1 (2)").is_file());
    assert!(fixture.path("Plan.v1 (1).txt").is_file());
}

/// A number whose name is taken by an item that is not selected is
/// skipped: nothing is overwritten and no message shows.
///
/// parity: OPS-050
#[gtk::test]
fn a_taken_numbered_name_is_skipped() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Plan (2).txt"), "kept").expect("the file is written");
    let test = TestWindow::open(&fixture.uri());
    select_with_focus(&test, &["Notes 2.txt", "Notes 10.txt"], "Notes 2.txt");

    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Plan");
    field.emit_activate();

    wait_until("the renamed items to be selected", || {
        test.selected_names() == ["Plan (1).txt", "Plan (3).txt"]
    });
    assert_ended_without_a_message(&test);
    assert_eq!(content(&fixture, "Plan (2).txt"), "kept");
}

/// Escape ends the rename: no item changes and the selection stays.
///
/// parity: OPS-050
#[gtk::test]
fn escape_renames_none_of_the_selected_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_with_focus(&test, &["Notes 2.txt", "Notes 10.txt"], "Notes 2.txt");

    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Plan");
    press_escape(&field);
    wait_until("the rename to end", || !is_renaming_in_place(&test));
    settle();

    assert!(fixture.path("Notes 2.txt").is_file());
    assert!(fixture.path("Notes 10.txt").is_file());
    assert!(!fixture.path("Plan (1).txt").exists());
    assert_eq!(test.selected_names(), ["Notes 2.txt", "Notes 10.txt"]);
    assert_eq!(test.window.shown_message(), "");
}

/// An item that cannot be renamed, here one that went away while the
/// name was typed, is asked about in the progress window as Windows asks,
/// "Item Not Found", and Skip leaves it out; the others are renamed and
/// stay selected, nothing is said afterwards, and Undo renames them back.
///
/// parity: OPS-050, OPS-023, OPS-047
#[gtk::test]
fn an_item_that_cannot_be_renamed_is_asked_about_and_the_rest_are_renamed() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Map.jpg"), "a map").expect("the file is written");
    let test = TestWindow::open(&fixture.uri());
    select_with_focus(&test, &["Map.jpg", "Notes 2.txt", "Notes 10.txt"], "Map.jpg");

    test.activate("rename", None);
    let field = name_editor(&test);
    fs::remove_file(fixture.path("Notes 2.txt")).expect("the file is removed");
    field.set_text("Plan");
    field.emit_activate();

    let failed = failure_question(&test);
    assert_eq!(failed.caption_shown(), "Item Not Found");
    assert!(
        failed.failure_texts().iter().any(|text| text == "Notes 2.txt"),
        "{:?}",
        failed.failure_texts()
    );
    failed.press_failure("Skip");
    wait_until("the renamed items to be selected", || {
        test.selected_names() == ["Plan (1).jpg", "Plan (2).txt"]
    });
    assert!(dialog_over(&test).is_none(), "nothing is said afterwards");
    assert_eq!(content(&fixture, "Plan (2).txt"), "Synthetic test data\n");

    test.activate("undo", None);
    wait_until("the renamed items to be renamed back", || {
        fixture.path("Map.jpg").is_file() && fixture.path("Notes 10.txt").is_file()
    });
}

/// With one item selected, the name typed is its whole new name, as
/// before: no number is added.
///
/// parity: OPS-009, OPS-050
#[gtk::test]
fn one_selected_item_gets_exactly_the_typed_name() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_with_focus(&test, &["Notes 2.txt"], "Notes 2.txt");

    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Plan.txt");
    field.emit_activate();

    wait_until("the renamed file to be selected", || {
        test.selected_names() == ["Plan.txt"]
    });
    assert_ended_without_a_message(&test);
    assert!(fixture.path("Plan.txt").is_file());
    assert!(!fixture.path("Plan (1).txt").exists());
}

/// When focus is on none of the selected items, the first one's name is
/// edited; when that item is not on screen, the Rename dialog asks for
/// the name instead, and the name still goes to every selected item.
///
/// parity: OPS-050
#[gtk::test]
fn several_items_off_screen_are_named_in_the_rename_dialog() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["file 0398.txt", "file 0399.txt"]);

    test.activate("rename", None);
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Rename");
    let field = text_field(&dialog);
    assert_eq!(field.text(), "file 0398.txt");
    field.set_text("Plan.txt");
    dialog.press("Save");

    wait_until("the renamed files to be selected", || {
        test.selected_names() == ["Plan (1).txt", "Plan (2).txt"]
    });
    assert_ended_without_a_message(&test);
    assert!(!fixture.path("file 0398.txt").exists());
}

/// Save or Enter on the unchanged name in the Rename dialog, which asks
/// when the edited item is not on screen, renames nothing, as Enter on the
/// unchanged name in place does.
///
/// parity: OPS-050
#[gtk::test]
fn the_unchanged_name_in_the_rename_dialog_renames_nothing() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["file 0398.txt", "file 0399.txt"]);

    test.activate("rename", None);
    let dialog = open_dialog(&test);
    assert_eq!(text_field(&dialog).text(), "file 0398.txt");
    dialog.press("Save");
    wait_until("the dialog to close", || !dialog.is_visible());
    settle();

    assert!(!test.window.is_writing_files());
    assert!(fixture.path("file 0398.txt").is_file());
    assert!(fixture.path("file 0399.txt").is_file());
    assert!(!fixture.path("file 0398 (1).txt").exists());
    assert!(!fixture.path("file 0398 (2).txt").exists());
    assert_eq!(test.window.shown_message(), "");
}

/// A folder's name has no extension: a dot typed into it stays part of
/// the name the numbers follow ("Archive.v1 (1)"), also when the edited
/// folder's own name ends with the same dotted part.
///
/// parity: OPS-050
#[gtk::test]
fn a_dotted_name_typed_for_a_folder_is_numbered_whole() {
    let fixture = Fixture::empty();
    fs::create_dir(fixture.path("Project.v1")).expect("the folder is made");
    fs::create_dir(fixture.path("Studio")).expect("the folder is made");
    let test = TestWindow::open(&fixture.uri());
    select_with_focus(&test, &["Project.v1", "Studio"], "Project.v1");

    test.activate("rename", None);
    let field = name_editor(&test);
    assert_eq!(field.selection_bounds(), Some((0, 10)), "a folder's whole name");
    field.set_text("Archive.v1");
    field.emit_activate();

    wait_until("the renamed folders to be selected", || {
        test.selected_names() == ["Archive.v1 (1)", "Archive.v1 (2)"]
    });
    assert!(fixture.path("Archive.v1 (1)").is_dir());
    assert!(fixture.path("Archive.v1 (2)").is_dir());
}

/// A name starting with a dot hides every renamed item while hidden files
/// are not shown: the Rename dialog, which asks when the edited item is
/// not on screen, asks "Rename and hide?" first, as renaming in place
/// does. Cancel keeps the dialog with the typed name; Rename and Hide
/// renames them all.
///
/// parity: OPS-050, OPS-013
#[gtk::test]
fn a_hiding_name_in_the_rename_dialog_asks_first() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["file 0398.txt", "file 0399.txt"]);
    test.activate("rename", None);
    let dialog = open_dialog(&test);
    let field = text_field(&dialog);
    let question = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|window| window.downcast::<Dialog>().ok())
            .find(|shown| shown.is_visible() && shown.title_text() == "Rename and hide?")
    };

    field.set_text(".Plan.txt");
    dialog.press("Save");
    wait_until("the hide question", || question().is_some());
    question().expect("it shows").press("Cancel");
    wait_until("the question to close", || question().is_none());
    settle();

    assert!(dialog.is_visible(), "the Rename dialog stays");
    assert_eq!(field.text(), ".Plan.txt", "the typed name is kept");
    assert!(fixture.path("file 0398.txt").is_file());
    assert!(!fixture.path(".Plan (1).txt").exists());

    dialog.press("Save");
    wait_until("the hide question again", || question().is_some());
    question().expect("it shows").press("Rename and Hide");
    wait_until("the hidden names", || {
        fixture.path(".Plan (1).txt").is_file() && fixture.path(".Plan (2).txt").is_file()
    });
    assert!(!fixture.path("file 0398.txt").exists());
}
