// SPDX-License-Identifier: AGPL-3.0-only
//! New folder in a real window, as Windows Explorer makes one: the folder
//! is created at once as "New folder", or "New folder (2)" and so on when
//! that is taken, then selected and renamed in place.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    dialog_over, is_enabled, is_renaming_in_place, name_editor, require_private_trash,
};
use crate::test_support::harness::{settle, wait_until, Fixture, TestWindow};

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

/// Runs New folder and returns the field that renames the new folder.
fn make_new_folder(test: &TestWindow) -> gtk::Entry {
    test.activate("new-folder", None);
    settle();
    assert!(dialog_over(test).is_none(), "no dialog asks for a name");
    name_editor(test)
}

/// parity: OPS-001, OPS-010, CMD-004
#[gtk::test]
fn new_folder_makes_the_folder_at_once_and_renames_it_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_folder(&test);

    assert!(
        fixture.path("New folder").is_dir(),
        "made before any name is typed"
    );
    assert_eq!(test.selected_names(), ["New folder"]);
    assert_eq!(field.text(), "New folder");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 10)),
        "the whole name is selected"
    );
    field.set_text("Projects");
    field.emit_activate();
    wait_until("the rename", || fixture.path("Projects").is_dir());
    assert!(!fixture.path("New folder").exists());
    wait_until("the renamed folder to be selected", || {
        test.selected_names() == ["Projects"]
    });
}

/// A taken "New folder" is never overwritten: the next one is
/// "New folder (2)", as in Windows.
///
/// parity: OPS-001, OPS-008
#[gtk::test]
fn a_taken_name_gives_the_next_numbered_folder() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("New folder")).expect("the folder is made");
    fixture.write("New folder/keep.txt");
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_folder(&test);

    assert_eq!(field.text(), "New folder (2)");
    assert!(fixture.path("New folder (2)").is_dir());
    assert!(
        fixture.path("New folder/keep.txt").is_file(),
        "the taken one is left alone"
    );
}

/// Escape ends the rename and keeps the name "New folder".
///
/// parity: OPS-001
#[gtk::test]
fn escape_keeps_the_name_new_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_folder(&test);

    press_in(&field, gdk::Key::Escape);

    wait_until("the rename to end", || !is_renaming_in_place(&test));
    assert!(fixture.path("New folder").is_dir());
    assert_eq!(test.selected_names(), ["New folder"]);
}

/// Undo takes the new folder away again, to the Recycle Bin.
///
/// parity: OPS-029
#[gtk::test]
fn undo_takes_the_new_folder_away() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_folder(&test);
    press_in(&field, gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(&test));

    test.activate("undo", None);

    wait_until("the folder to go", || !fixture.path("New folder").exists());
}

/// New folder works in an empty folder and in the icon views too.
///
/// parity: OPS-001, OPS-010
#[gtk::test]
fn new_folder_renames_in_place_in_an_empty_folder_and_in_icon_views() {
    let fixture = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_folder(&test);
    assert_eq!(field.text(), "New folder");
    press_in(&field, gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(&test));

    test.activate("view", Some("large"));
    let field = make_new_folder(&test);
    assert_eq!(field.text(), "New folder (2)");
}

/// Where the folder cannot be made, nothing asks for a name and nothing
/// is made: New is disabled there, or, when it runs, the toast says why.
/// (Disabling New in a folder you cannot write to is its own fix; this
/// holds with or without it.)
///
/// parity: OPS-001
#[gtk::test]
fn a_folder_that_cannot_take_a_new_folder_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("Locked")).expect("the folder is made");
    let test = TestWindow::open(&fixture.uri_of("Locked"));
    std::fs::set_permissions(fixture.path("Locked"), std::fs::Permissions::from_mode(0o555))
        .expect("the folder is ours");

    let enabled = is_enabled(&test, "new-folder");
    test.activate("new-folder", None);
    if enabled {
        wait_until("the refusal", || !test.window.shown_message().is_empty());
    }
    settle();
    let shown = test.window.shown_message().to_string();
    std::fs::set_permissions(fixture.path("Locked"), std::fs::Permissions::from_mode(0o755))
        .expect("the folder is ours");

    assert!(dialog_over(&test).is_none(), "no dialog");
    assert!(!is_renaming_in_place(&test));
    assert!(!fixture.path("Locked/New folder").exists());
    assert!(!enabled || !shown.is_empty(), "when New runs, the toast says why");
}

/// A new folder that sorts far down a long list, with folders not kept
/// first, is scrolled to and still renamed in place, not in a dialog.
///
/// parity: OPS-001, OPS-010
#[gtk::test]
fn a_new_folder_far_down_the_list_is_scrolled_to_and_renamed_in_place() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().model().set_folders_first(false);
    settle();

    let field = make_new_folder(&test);

    assert_eq!(field.text(), "New folder");
    assert!(dialog_over(&test).is_none(), "no Rename dialog either");
}
