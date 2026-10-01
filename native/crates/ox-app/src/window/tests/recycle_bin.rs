// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin in a real window: listing `trash:///`, its menus,
//! Restore and Empty Recycle Bin. New in the native app (OPS-040 to
//! OPS-042), so no Python test is ported here. Every test checks first
//! that the Recycle Bin is the test run's private one.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::TRASH_URI;
use ox_core::ops::{list_recycle_bin, JournalDirection};
use ox_core::settings::{PreferencesUpdate, Settings};
use ox_core::transfer::Cancellation;

use super::file_ops_support::{
    dialog_over, is_enabled, open_dialog, require_private_trash, select_names, wait_for_no_dialog,
};
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::file_drop::DropAction;

/// Moves `path` to the Trash, as another file manager would.
fn trash(path: &Path) {
    gio::File::for_path(path)
        .trash(None::<&gio::Cancellable>)
        .expect("the private Trash takes the file");
}

/// Panics unless everything in the Recycle Bin came from this test run's
/// temporary folders, so emptying it can delete nothing of the user's
/// (the Trash of another mounted volume is listed too).
fn require_only_test_items_in_the_recycle_bin() {
    let items = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    let temp = std::env::temp_dir();
    for item in items {
        let from_test = item
            .original_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&temp));
        assert!(
            from_test,
            "the Recycle Bin holds an item from outside the test: {item:?}"
        );
    }
}

/// parity: OPS-040, OPS-041
#[gtk::test]
fn the_recycle_bin_lists_trashed_items_and_restore_puts_them_back() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Budget for restoring.txt");
    let original = fixture.path("Budget for restoring.txt");
    trash(&original);
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        test.names().contains(&"Budget for restoring.txt".to_owned())
    });
    select_names(&test, &["Budget for restoring.txt"]);

    test.window
        .right_click(test.window.folder_model().first_selected());
    let labels = test.window.context_menu().row_labels();
    test.window.context_menu().popdown();
    test.activate("restore", None);

    assert_eq!(labels, ["Restore", "Delete permanently", "-", "Properties"]);
    wait_until("the file to be back", || original.is_file());
    wait_until("the toast", || {
        test.window.shown_message() == "1 item(s) restored."
    });
}

/// parity: OPS-043
#[gtk::test]
fn delete_in_the_recycle_bin_deletes_only_the_chosen_items_for_good() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Delete me for good.txt");
    fixture.write("Keep me trashed.txt");
    trash(&fixture.path("Delete me for good.txt"));
    trash(&fixture.path("Keep me trashed.txt"));
    let test = TestWindow::open(TRASH_URI);
    let is_listed = |name: &str| test.names().contains(&name.to_owned());
    wait_until("the trashed files to be listed", || {
        is_listed("Delete me for good.txt") && is_listed("Keep me trashed.txt")
    });
    select_names(&test, &["Delete me for good.txt"]);

    test.activate("trash", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete permanently?");
    dialog.press("Delete permanently");
    wait_until("the item to leave the Recycle Bin", || {
        !is_listed("Delete me for good.txt")
    });
    assert!(is_listed("Keep me trashed.txt"));
    assert!(
        !fixture.path("Delete me for good.txt").exists(),
        "nothing was restored"
    );
}

/// parity: OPS-040, OPS-042
#[gtk::test]
fn empty_recycle_bin_asks_then_deletes_everything_in_it() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old draft.txt");
    trash(&fixture.path("Old draft.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || !test.names().is_empty());

    test.window.right_click(None);
    let labels = test.window.context_menu().row_labels();
    test.window.context_menu().popdown();
    test.activate("empty-recycle-bin", None);
    let dialog = open_dialog(&test);

    assert_eq!(labels, ["Empty Recycle Bin", "Refresh"]);
    assert_eq!(dialog.title_text(), "Empty Recycle Bin?");
    assert_eq!(dialog.button_labels(), ["Cancel", "Empty Recycle Bin"]);
    dialog.press("Empty Recycle Bin");
    wait_until("the Recycle Bin to be empty", || test.names().is_empty());
    wait_until("the empty Recycle Bin text", || {
        test.window.folder_pane().empty_page().title() == "Recycle Bin is empty"
    });
    let left = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    assert!(left.is_empty(), "{left:?}");
}

/// With "Ask before emptying the Recycle Bin" off, Empty Recycle Bin
/// empties it at once.
///
/// parity: SET-010
#[gtk::test]
fn empty_recycle_bin_asks_nothing_when_the_settings_say_so() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old note.txt");
    trash(&fixture.path("Old note.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(TRASH_URI);
    let update = PreferencesUpdate {
        confirm_empty_trash: Some(false),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        !test.context.settings_data().preferences.confirm_empty_trash
    });
    wait_until("the trashed file to be listed", || !test.names().is_empty());

    test.activate("empty-recycle-bin", None);
    wait_until("the Recycle Bin to be empty", || test.names().is_empty());
    assert!(dialog_over(&test).is_none(), "nothing asked");
}

/// The sidebar's Recycle Bin shows whether it is full and empties it
/// from any folder; Recent files sits beside it.
///
/// parity: SIDE-025, SIDE-026
#[gtk::test]
fn the_sidebar_recycle_bin_shows_it_is_full_and_empties_from_anywhere() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old draft.txt");
    trash(&fixture.path("Old draft.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let bin_tooltip = || {
        let index = sidebar.labels().iter().position(|label| label == "Recycle Bin");
        let index = i32::try_from(index.expect("the sidebar shows the Recycle Bin")).unwrap_or(0);
        let row = sidebar.list().row_at_index(index).expect("its row");
        row.tooltip_text().map(String::from).unwrap_or_default()
    };
    assert!(sidebar.labels().contains(&"Recent files".to_owned()));
    // Earlier tests in this run may have left items of their own.
    wait_until("the full Recycle Bin", || {
        bin_tooltip().ends_with(" item") || bin_tooltip().ends_with(" items")
    });

    wait_for_frames(&test.window, 3);
    let menu = sidebar.right_click_row("Recycle Bin");
    let labels = menu.row_labels();
    let empty = menu.row("Empty Recycle Bin");
    menu.popdown();
    assert_eq!(labels[..3], ["Open", "Open in new tab", "Open in new window"]);
    assert!(empty.is_sensitive(), "something to empty");
    test.activate("empty-trash", None);
    open_dialog(&test).press("Empty Recycle Bin");

    wait_until("the empty Recycle Bin", || bin_tooltip() == "Recycle Bin · Empty");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "the folder stays");
}

/// parity: OPS-045
#[gtk::test]
fn items_dropped_on_the_recycle_bin_go_to_the_trash() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Drop me in the bin.txt");
    let test = TestWindow::open(TRASH_URI);
    test.wait_for_listing("the Recycle Bin");

    let taken = test.window.drop_files(
        &[fixture.uri_of("Drop me in the bin.txt")],
        None,
        DropAction::Copy,
    );
    assert!(taken, "{}", test.window.shown_message());
    open_dialog(&test).press("Move to Trash");

    wait_until("the dropped file to be listed", || {
        test.names().contains(&"Drop me in the bin.txt".to_owned())
    });
    assert!(!fixture.path("Drop me in the bin.txt").exists());
}

/// parity: OPS-046, DND-018
#[gtk::test]
fn items_dragged_out_of_the_recycle_bin_are_moved_into_the_folder() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Bring me back here.txt");
    trash(&fixture.path("Bring me back here.txt"));
    let bin = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        bin.names().contains(&"Bring me back here.txt".to_owned())
    });
    select_names(&bin, &["Bring me back here.txt"]);
    let position = bin.window.folder_model().first_selected().expect("selected");
    assert!(
        bin.window.drag_content_for(position).is_some(),
        "Recycle Bin items can be dragged"
    );
    let trashed = bin.window.folder_model().selected_uris();
    assert!(
        !bin.window.drop_files(&trashed, None, DropAction::Copy),
        "not back into the Recycle Bin"
    );

    let documents = TestWindow::open(&fixture.uri_of("Documents"));
    assert!(documents.window.drop_files(&trashed, None, DropAction::Copy));

    let moved = fixture.path("Documents").join("Bring me back here.txt");
    wait_until("the item to be moved into Documents", || moved.is_file());
    assert!(
        !fixture.path("Bring me back here.txt").exists(),
        "moved, not restored"
    );
}

/// Undo asks before it moves a copy that changed after the copy to the
/// Recycle Bin; Cancel keeps the copy and the Undo step.
///
/// parity: OPS-030
#[gtk::test]
fn undoing_a_copy_that_changed_since_asks_and_cancel_keeps_it() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    let copy = fixture.path("Documents/Notes 2.txt");
    wait_until("the copy", || test.window.shown_message() == "1 item(s) copied.");
    let edited = std::fs::File::options()
        .write(true)
        .open(&copy)
        .expect("the copy exists");
    let a_minute_ahead = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    edited.set_modified(a_minute_ahead).expect("the copy is ours");

    test.activate("undo", None);
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Undo copy?");
    assert_eq!(
        question.message_text(),
        "“Notes 2.txt” was changed after it was copied. Undo moves it to the Recycle Bin anyway?"
    );
    question.press("Cancel");
    wait_for_no_dialog(&test);

    assert!(copy.is_file(), "the copy stays");
    assert_eq!(test.window.journal_label(JournalDirection::Undo), "Undo: Copy");
}
