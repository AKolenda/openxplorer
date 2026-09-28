// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files out of and dropping files onto a real window, against
//! `makeFileDraggable` in `desktop/ui/app.js` and
//! `desktop/native_file_drop.py`. The drag and drop gestures themselves
//! need a pointer, which the private display has none of; these tests run
//! what the drag source and the drop target call.

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{open_dialog, select_names};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// The position of the item called `name` in `test`'s view.
fn position_of(test: &TestWindow, name: &str) -> u32 {
    let model = test.window.folder_model();
    (0..model.n_items())
        .find(|position| model.name_at(*position).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} is listed"))
}

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
        .drag_content_for(position_of(&test, "Notes 2.txt"))
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
        .drag_content_for(position_of(&test, "Résumé.txt"))
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

    let taken = test.window.drop_files(&dropped, None);

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
        Some(position_of(&test, "Documents")),
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
        .drop_files(&["https://example.com/a.txt".to_owned()], None);
    let link_message = test.window.shown_message();
    let operation = test.window.begin_operation("Preparing copy…");
    let busy = test.window.drop_files(&[fixture.uri_of("Notes 2.txt")], None);
    let busy_message = test.window.shown_message();
    test.window.end_operation();
    test.window.search_box().entry().set_text("Notes");
    wait_until("the search to filter", || {
        test.window.folder_model().is_searching()
    });
    let searching = test.window.drop_files(&[fixture.uri_of("Notes 2.txt")], None);

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

    let content = test.window.drag_content_for(position_of(&test, "Notes 2.txt"));
    test.window.end_operation();

    assert!(operation.is_some());
    assert!(content.is_none());
    let after = test.window.drag_content_for(position_of(&test, "Notes 2.txt"));
    assert!(after.is_some(), "a drag starts again once the operation ended");
}
