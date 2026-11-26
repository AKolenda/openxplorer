// SPDX-License-Identifier: AGPL-3.0-only
//! A folder renamed in place to the name of another folder in its folder
//! asks Windows 11's "Confirm Folder Replace" question (checked in Windows
//! 11 26200) and, on Yes, is merged into that folder: its items move in,
//! a file whose name is taken there asks Replace or Skip as a move does,
//! and the emptied folder goes. One Undo takes the merge back; nothing is
//! lost on the way (OPS-008).

use std::fs;

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{
    answer_conflicts, dialog_over, is_renaming_in_place, name_editor, open_dialog, select_names,
    wait_for_no_dialog, ConflictAnswer,
};
use crate::dialog::Dialog;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Pointer};

/// `Alpha` holding `a.txt`, `shared.txt` and `Sub/c.txt`, and `Beta`
/// holding `b.txt`, `shared.txt` and `Sub/d.txt`, with fictional text.
fn two_folders(with_shared: bool) -> Fixture {
    let fixture = Fixture::empty();
    for (folder, own, sub) in [("Alpha", "a.txt", "c.txt"), ("Beta", "b.txt", "d.txt")] {
        fs::create_dir_all(fixture.path(&format!("{folder}/Sub"))).expect("a folder");
        fs::write(fixture.path(&format!("{folder}/{own}")), folder).expect("a file");
        fs::write(fixture.path(&format!("{folder}/Sub/{sub}")), folder).expect("a file");
        if with_shared {
            fs::write(fixture.path(&format!("{folder}/shared.txt")), folder).expect("a file");
        }
    }
    fixture
}

/// The text of `path` in `fixture`.
fn text(fixture: &Fixture, path: &str) -> String {
    fs::read_to_string(fixture.path(path)).unwrap_or_default()
}

/// Renames the folder Alpha in place to "Beta" with F2 and Enter on the
/// real keyboard, and returns the question that asks.
fn rename_alpha_to_beta(test: &TestWindow, pointer: &mut Pointer) -> Dialog {
    select_names(test, &["Alpha"]);
    test.window.folder_pane().focus_view();
    wait_for_frames(&test.window, 3);
    let inside = screen_point(pointer, &test.window, (40.0, 40.0), (200, 200));
    pointer.move_to(inside.0, inside.1);
    pointer.tap_key(gdk::Key::F2);
    let field = name_editor(test);
    wait_until("the field takes the keyboard", || {
        GtkWindowExt::focus(&test.window).is_some_and(|focus| focus.is_ancestor(&field))
    });
    field.set_text("Beta");
    pointer.tap_key(gdk::Key::Return);
    open_dialog(test)
}

/// The visible texts of `dialog`'s labels.
fn texts_of(dialog: &Dialog) -> Vec<String> {
    descendants::<gtk::Label>(dialog)
        .iter()
        .filter(|label| label.is_visible())
        .map(|label| label.text().to_string())
        .collect()
}

/// The question names the folder already there and asks, in Windows 11's
/// words, whether to merge the renamed folder into it, showing each
/// folder with its date created; No goes back to editing the typed name
/// and changes nothing.
///
/// parity: OPS-008
#[gtk::test]
fn a_folder_renamed_to_a_taken_folder_name_asks_to_merge() {
    let fixture = two_folders(false);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));

    let question = rename_alpha_to_beta(&test, &mut pointer);
    let shown = texts_of(&question);
    let title = question.title_text();
    question.press("No");
    wait_for_no_dialog(&test);

    assert_eq!(title, "Confirm Folder Replace");
    let all = shown.join("\n");
    for line in [
        "This destination already contains a folder named 'Beta'.",
        "If any files have the same names, you will be asked if you want to replace those files.",
        "Do you still want to merge this folder",
        "with this one?",
    ] {
        assert!(all.contains(line), "{line:?} in {shown:?}");
    }
    let beta = shown
        .iter()
        .position(|text| text == "Beta")
        .expect("Beta is shown");
    let alpha = shown
        .iter()
        .position(|text| text == "Alpha")
        .expect("Alpha is shown");
    assert!(beta < alpha, "Beta first, then Alpha: {shown:?}");
    let dates = shown
        .iter()
        .filter(|text| text.starts_with("Date created: "))
        .count();
    assert_eq!(dates, 2, "each folder's date created: {shown:?}");
    assert!(is_renaming_in_place(&test), "No goes back to editing");
    assert_eq!(name_editor(&test).text(), "Beta");
    assert!(fixture.path("Alpha/a.txt").is_file() && fixture.path("Beta/b.txt").is_file());
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(&test));
}

/// Yes merges Alpha into Beta: its items and its subfolder's items move
/// in, the emptied Alpha goes, Beta is selected; one Undo makes Alpha
/// again with everything in it, and Redo merges again.
///
/// parity: OPS-008
#[gtk::test]
fn yes_merges_the_folders_and_one_undo_takes_it_back() {
    let fixture = two_folders(false);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));

    let question = rename_alpha_to_beta(&test, &mut pointer);
    question.press("Yes");
    wait_until("the merge", || {
        fixture.path("Beta/a.txt").is_file() && !fixture.path("Alpha").exists()
    });
    wait_until("Beta to be selected", || test.selected_names() == ["Beta"]);

    assert_eq!(text(&fixture, "Beta/a.txt"), "Alpha");
    assert_eq!(text(&fixture, "Beta/Sub/c.txt"), "Alpha");
    assert_eq!(text(&fixture, "Beta/Sub/d.txt"), "Beta");
    assert!(dialog_over(&test).is_none());

    wait_until("Undo Rename", || {
        super::file_ops_support::is_enabled(&test, "undo")
    });
    test.activate("undo", None);
    wait_until("the undo", || fixture.path("Alpha/a.txt").is_file());
    wait_until("the subfolder back", || fixture.path("Alpha/Sub/c.txt").is_file());
    assert!(!fixture.path("Beta/a.txt").exists());
    assert!(!fixture.path("Beta/Sub/c.txt").exists());
    assert_eq!(text(&fixture, "Beta/b.txt"), "Beta");
    assert_eq!(text(&fixture, "Beta/Sub/d.txt"), "Beta");

    wait_until("Redo", || super::file_ops_support::is_enabled(&test, "redo"));
    test.activate("redo", None);
    wait_until("the redo", || {
        fixture.path("Beta/a.txt").is_file() && !fixture.path("Alpha").exists()
    });
}

/// A file whose name is taken in Beta asks Replace or Skip, as a move
/// does; Skip keeps both files where they are, so Alpha stays with the
/// skipped file in it, and nothing is lost.
///
/// parity: OPS-008
#[gtk::test]
fn skipping_a_taken_file_keeps_it_in_the_renamed_folder() {
    let fixture = two_folders(true);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));

    let question = rename_alpha_to_beta(&test, &mut pointer);
    question.press("Yes");
    answer_conflicts(&test, ConflictAnswer::Skip);
    wait_until("the rest of the merge", || fixture.path("Beta/a.txt").is_file());
    wait_until("the job to end", || !test.window.is_writing_files());

    assert_eq!(text(&fixture, "Beta/shared.txt"), "Beta", "the file there stays");
    assert_eq!(
        text(&fixture, "Alpha/shared.txt"),
        "Alpha",
        "the skipped file stays"
    );
    assert_eq!(text(&fixture, "Beta/Sub/c.txt"), "Alpha");
}

/// Replace for a taken file sends the file already in Beta to the
/// Recycle Bin and moves Alpha's in; Alpha, now empty, goes. One Undo
/// brings back both: Alpha with its file, and Beta's own file from the
/// Recycle Bin, so the merge loses nothing.
///
/// parity: OPS-008
#[gtk::test]
fn replacing_a_taken_file_is_undone_with_both_files_back() {
    super::file_ops_support::require_private_trash();
    let fixture = two_folders(true);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));

    let question = rename_alpha_to_beta(&test, &mut pointer);
    question.press("Yes");
    answer_conflicts(&test, ConflictAnswer::Replace);
    wait_until("the merge", || {
        text(&fixture, "Beta/shared.txt") == "Alpha" && !fixture.path("Alpha").exists()
    });

    wait_until("Undo Rename", || {
        super::file_ops_support::is_enabled(&test, "undo")
    });
    test.activate("undo", None);
    wait_until("both files back", || {
        text(&fixture, "Alpha/shared.txt") == "Alpha" && text(&fixture, "Beta/shared.txt") == "Beta"
    });
    assert!(fixture.path("Alpha/a.txt").is_file());
    assert!(fixture.path("Alpha/Sub/c.txt").is_file());
    assert!(!fixture.path("Beta/a.txt").exists());
}

/// When the emptied Alpha cannot be removed for another reason than an
/// item left in it, here the folder it is in taking no changes, the
/// window says so once the items are moved, rather than taking the merge
/// as done in full; Alpha stays, empty, and nothing is lost.
///
/// parity: OPS-008
#[gtk::test]
fn a_merged_folder_that_cannot_be_removed_is_said() {
    use std::os::unix::fs::PermissionsExt;

    /// Gives the folder its permissions back, also when the test fails.
    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }

    let fixture = two_folders(false);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));

    let question = rename_alpha_to_beta(&test, &mut pointer);
    let _unlock = Unlock(fixture.path(""));
    fs::set_permissions(fixture.path(""), fs::Permissions::from_mode(0o555)).expect("mode");
    if fs::write(fixture.path("probe"), "").is_ok() {
        question.press("No");
        return; // Running as root, which removes anything.
    }
    question.press("Yes");
    wait_until("the items moved", || {
        fixture.path("Beta/a.txt").is_file() && fixture.path("Beta/Sub/c.txt").is_file()
    });
    wait_until("the message", || {
        test.window
            .shown_message()
            .starts_with("The items were moved, but the folder ")
    });

    let message = test.window.shown_message().to_string();
    assert!(message.contains("Alpha"), "{message}");
    assert!(message.contains("Access is denied."), "{message}");
    assert!(fixture.path("Alpha").is_dir(), "Alpha stays");
    assert_eq!(text(&fixture, "Beta/a.txt"), "Alpha");
}
