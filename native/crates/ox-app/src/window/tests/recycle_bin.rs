// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin in a real window: listing `trash:///`, its menus,
//! Restore and Empty Recycle Bin. New in the native app (OPS-040 to
//! OPS-042), so no Python test is ported here. Every test checks first
//! that the Recycle Bin is the test run's private one.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::TRASH_URI;
use ox_core::ops::list_recycle_bin;
use ox_core::transfer::Cancellation;

use super::file_ops_support::{open_dialog, require_private_trash, select_names};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};
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
    let left = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    assert!(left.is_empty(), "{left:?}");
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
