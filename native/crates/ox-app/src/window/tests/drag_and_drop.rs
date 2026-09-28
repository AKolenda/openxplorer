// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files out of and dropping files onto a real window, against
//! `makeFileDraggable` in `desktop/ui/app.js` and
//! `desktop/native_file_drop.py`. The drag and drop gestures themselves
//! need a pointer, which the private display has none of; these tests run
//! what the drag source and the drop target call.

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::file_ops_support::{dialog_over, open_dialog, select_names};
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::window::file_drag::DraggedItems;
use crate::window::file_drop::DropAction;

/// The URIs a drag's content offers as a file list.
fn offered_uris(content: &gdk::ContentProvider) -> Vec<String> {
    let value = content
        .value(gdk::FileList::static_type())
        .expect("a drag offers a file list");
    let files = value.get::<gdk::FileList>().expect("the value is a file list");
    files.files().iter().map(|file| file.uri().to_string()).collect()
}

/// parity: DND-001, DND-002
#[gtk::test]
fn dragging_a_selected_item_carries_the_whole_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Documents"]);

    let content = test
        .window
        .drag_content_for(test.position_of("Notes 2.txt"))
        .expect("the selection can be dragged");

    let mut offered = offered_uris(&content);
    offered.sort();
    let mut expected = vec![fixture.uri_of("Documents"), fixture.uri_of("Notes 2.txt")];
    expected.sort();
    assert_eq!(offered, expected);
    // Other apps receive it as the text/uri-list GTK serializes it to.
    let served = content.formats().union_serialize_mime_types();
    assert!(served.contain_mime_type("text/uri-list"));
}

/// parity: DND-002
#[gtk::test]
fn dragging_an_unselected_item_selects_and_carries_only_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    let content = test
        .window
        .drag_content_for(test.position_of("Résumé.txt"))
        .expect("a file can be dragged");

    assert_eq!(offered_uris(&content), [fixture.uri_of("Résumé.txt")]);
    assert_eq!(test.selected_names(), ["Résumé.txt"]);
}

/// parity: DND-009, DND-011
#[gtk::test]
fn files_dropped_on_blank_space_are_copied_into_the_folder_shown() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let dropped = [source.uri_of("Notes 2.txt"), source.uri_of("Résumé.txt")];

    let taken = test.window.drop_files(&dropped, None, DropAction::Copy);

    assert!(taken);
    wait_until("the copies to be selected", || test.selected_names().len() == 2);
    assert!(fixture.path("Documents/Notes 2.txt").is_file());
    assert!(fixture.path("Documents/Résumé.txt").is_file());
    assert!(source.path("Notes 2.txt").is_file(), "a drop is a copy");
}

/// parity: DND-009, DND-011
#[gtk::test]
fn files_dropped_on_a_folder_go_into_that_folder_after_the_conflict_check() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    fixture.write("Documents/Notes 2.txt");
    let test = TestWindow::open(&fixture.uri());

    let taken = test.window.drop_files(
        &[source.uri_of("Notes 2.txt")],
        Some(test.position_of("Documents")),
        DropAction::Copy,
    );
    let dialog = open_dialog(&test);

    assert!(taken);
    assert_eq!(dialog.title_text(), "Items already exist");
    dialog.press("Keep both");
    wait_until("the second copy", || {
        std::fs::read_dir(fixture.path("Documents"))
            .expect("the folder lists")
            .count()
            == 2
    });
}

/// parity: DND-012, DND-013
#[gtk::test]
fn links_and_drops_during_a_search_or_an_operation_are_refused() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let link = test
        .window
        .drop_files(&["https://example.com/a.txt".to_owned()], None, DropAction::Copy);
    let link_message = test.window.shown_message();
    let operation = test.window.begin_operation("Preparing copy…");
    let busy = test
        .window
        .drop_files(&[fixture.uri_of("Notes 2.txt")], None, DropAction::Copy);
    let busy_message = test.window.shown_message();
    test.window.end_operation();
    test.window.search_box().entry().set_text("Notes");
    wait_until("the search to filter", || {
        test.window.folder_model().is_searching()
    });
    let searching = test
        .window
        .drop_files(&[fixture.uri_of("Notes 2.txt")], None, DropAction::Copy);

    assert!(operation.is_some());
    assert!(!link && !busy && !searching);
    assert_eq!(link_message, "Drop files or folders, rather than links or text.");
    assert_eq!(
        busy_message,
        "Finish the current operation before dropping files."
    );
    assert_eq!(
        test.window.shown_message(),
        "Open a writable destination folder before dropping files."
    );
}

/// parity: DND-006
#[gtk::test]
fn no_drag_starts_while_a_file_operation_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");

    let content = test.window.drag_content_for(test.position_of("Notes 2.txt"));
    test.window.end_operation();

    assert!(operation.is_some());
    assert!(content.is_none());
    let after = test.window.drag_content_for(test.position_of("Notes 2.txt"));
    assert!(after.is_some(), "a drag starts again once the operation ended");
}

/// parity: DND-007
#[gtk::test]
fn a_drag_fades_its_items_and_pauses_clicks_until_shortly_after_it_ends() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Documents"]);
    let notes = test.position_of("Notes 2.txt");
    let owners = || test.window.folder_pane().owners();

    test.window
        .drag_content_for(notes)
        .expect("the selection can be dragged");
    test.window
        .show_file_drag_feedback()
        .expect("the drag was prepared");
    let faded_while_dragging = owners().is_shown_dragged(notes);
    let paused_while_dragging = test.window.are_item_clicks_paused();
    test.window.end_file_drag();
    let faded_after = owners().is_shown_dragged(notes);
    let paused_just_after = test.window.are_item_clicks_paused();
    wait_for(Duration::from_millis(450));

    assert_eq!(faded_while_dragging, Some(true));
    assert!(paused_while_dragging);
    assert_eq!(faded_after, Some(false));
    assert!(paused_just_after, "clicks stay paused 400 ms after the drag");
    assert!(!test.window.are_item_clicks_paused());
    assert_eq!(
        owners().is_shown_dragged(test.position_of("Résumé.txt")),
        Some(false)
    );
}

/// parity: DND-010
#[gtk::test]
fn an_openxplorer_window_receives_the_items_own_addresses() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);

    let content = test
        .window
        .drag_content_for(test.position_of("Résumé.txt"))
        .expect("a file can be dragged");

    let own = content
        .value(DraggedItems::static_type())
        .expect("the drag offers its own items to this process");
    let own = own.get::<DraggedItems>().expect("the value is the dragged items");
    let listed = test.window.folder_model().selected_items();
    assert_eq!(own.0, [listed[0].entry().uri.clone()]);
    assert_eq!(offered_uris(&content), own.0, "a local file is its own address");
}

/// parity: DND-017
#[gtk::test]
fn a_shift_drop_moves_the_items_into_the_folder() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let taken = test.window.drop_files(
        &[source.uri_of("Notes 2.txt"), source.uri_of("Résumé.txt")],
        Some(test.position_of("Documents")),
        DropAction::Move,
    );

    assert!(taken);
    wait_until("the items to move", || {
        fixture.path("Documents/Résumé.txt").is_file()
    });
    wait_until("the originals to go", || !source.path("Résumé.txt").exists());
    assert!(fixture.path("Documents/Notes 2.txt").is_file());
    assert!(!source.path("Notes 2.txt").exists());
}

/// parity: DND-017
#[gtk::test]
fn moving_items_into_the_folder_they_are_in_changes_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let names_before = test.names();

    let taken = test
        .window
        .drop_files(&[fixture.uri_of("Notes 2.txt")], None, DropAction::Move);
    wait_for(Duration::from_millis(100));

    assert!(taken);
    assert!(dialog_over(&test).is_none(), "no conflict to ask about");
    assert!(test.window.imp().file_operations.borrow().is_idle());
    assert_eq!(test.names(), names_before);
}

/// parity: DND-019
#[gtk::test]
fn a_ctrl_shift_drop_creates_links_to_the_items() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));

    let taken = test
        .window
        .drop_files(&[source.uri_of("Notes 2.txt")], None, DropAction::Link);

    assert!(taken);
    let link = fixture.path("Documents/Notes 2.txt");
    wait_until("the link", || link.symlink_metadata().is_ok());
    assert_eq!(
        std::fs::read_link(&link).expect("a symbolic link"),
        source.path("Notes 2.txt")
    );
    wait_until("the report", || {
        test.window.shown_message() == "1 item(s) linked."
    });
    wait_until("the new link to be selected", || {
        test.selected_names() == ["Notes 2.txt"]
    });
}

/// parity: DND-018
#[gtk::test]
fn an_alt_drop_asks_with_the_drop_menu_and_runs_the_answer() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let dropped = [source.uri_of("Notes 2.txt")];

    test.window.drop_files(&dropped, None, DropAction::Ask);
    let menu = test.window.drop_menu();
    wait_until("the drop menu", || menu.is_mapped());
    let labels = menu.row_labels();
    test.activate("drop-choice", Some("cancel"));
    wait_for(Duration::from_millis(100));
    let cancelled_kept_source = source.path("Notes 2.txt").is_file();
    let cancelled_made_nothing = !fixture.path("Documents/Notes 2.txt").exists();
    test.window.drop_files(&dropped, None, DropAction::Ask);
    test.activate("drop-choice", Some("move"));

    assert_eq!(
        labels,
        ["Copy here", "Move here", "Create links here", "-", "Cancel"]
    );
    assert!(cancelled_kept_source && cancelled_made_nothing);
    wait_until("the move", || fixture.path("Documents/Notes 2.txt").is_file());
    wait_until("the original to go", || !source.path("Notes 2.txt").exists());
}

/// parity: DND-011
#[gtk::test]
fn a_folder_dropped_onto_itself_is_refused() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let taken = test.window.drop_files(
        &[fixture.uri_of("Documents")],
        Some(test.position_of("Documents")),
        DropAction::Copy,
    );

    assert!(!taken);
    assert_eq!(
        test.window.shown_message(),
        "A folder cannot be copied into itself."
    );
}
