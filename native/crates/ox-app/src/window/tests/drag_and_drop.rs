// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files out of and dropping files onto a real window, against
//! `makeFileDraggable` in `v2.0.0:desktop/ui/app.js` and
//! `v2.0.0:desktop/native_file_drop.py`. The drag and drop gestures themselves
//! need a pointer, which the private display has none of; these tests run
//! what the drag source and the drop target call.

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::file_ops_support::{dialog_over, open_dialog, select_names};
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::window::file_drag::DraggedItems;
use crate::window::file_drop::{DragOrigin, DropAction, DropDestination};

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

/// parity: DND-009, DND-011, OPS-027
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

/// parity: DND-009, DND-011, OPS-026
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
    assert!(taken);
    super::file_ops_support::answer_conflicts(&test, super::file_ops_support::ConflictAnswer::KeepBoth);
    wait_until("the second copy", || {
        std::fs::read_dir(fixture.path("Documents"))
            .expect("the folder lists")
            .count()
            == 2
    });
}

/// A drop onto a folder that went away since it was listed cannot be
/// checked for conflicts: "Could not check destination" says so and
/// nothing is copied.
///
/// parity: OPS-026
#[gtk::test]
fn a_drop_whose_folder_went_away_reports_the_failed_check() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let documents = test.position_of("Documents");
    std::fs::remove_dir_all(fixture.path("Documents")).expect("the fixture is ours");

    let taken = test
        .window
        .drop_files(&[source.uri_of("Notes 2.txt")], Some(documents), DropAction::Copy);
    let dialog = open_dialog(&test);

    assert!(taken);
    assert_eq!(dialog.title_text(), "Could not check destination");
    assert!(!fixture.path("Documents").exists(), "nothing was copied");
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
    wait_until("the search to filter", || test.window.is_searching());
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

/// A drop while a dialog shows is refused, as the Python app refused it;
/// the in-window dialogs leave the tabs usable, so a drop could reach
/// the window then.
///
/// parity: DND-013
#[gtk::test]
fn a_drop_while_a_dialog_shows_is_refused() {
    let fixture = Fixture::standard();
    let source = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.window.show_result_dialog("Operation stopped", "Done.");
    wait_until("the dialog", || test.window.dialog_layer().shown().is_some());

    let taken = test.window.drop_files(
        &[source.uri_of("Documents")],
        Some(test.position_of("Documents")),
        DropAction::Copy,
    );

    assert!(!taken);
    assert_eq!(
        test.window.shown_message(),
        "Close the dialog and finish the current operation before dropping files."
    );
    assert!(!fixture.path("Documents/Documents").exists());
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

/// parity: DND-006
#[gtk::test]
fn no_drag_starts_while_a_menu_or_a_dialog_is_open() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let notes = test.position_of("Notes 2.txt");

    test.window.right_click(Some(notes));
    let with_menu = test.window.drag_content_for(notes);
    test.window.context_menu().popdown();
    super::file_ops_support::select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.activate("rename-items", None);
    let dialog = open_dialog(&test);
    let with_dialog = test.window.drag_content_for(notes);
    dialog.press("Cancel");

    assert!(with_menu.is_none());
    assert!(with_dialog.is_none());
    wait_until("drags to start again", || {
        test.window.drag_content_for(notes).is_some()
    });
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
    let link = fixture.path("Documents/Notes 2.txt - Shortcut");
    wait_until("the link", || link.symlink_metadata().is_ok());
    assert_eq!(
        std::fs::read_link(&link).expect("a symbolic link"),
        source.path("Notes 2.txt")
    );
    wait_until("the new link to be selected", || {
        test.selected_names() == ["Notes 2.txt - Shortcut"]
    });
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// An Alt drop from another app asks with the drop menu, a copy in bold
/// as a plain drop of another app's items copies; Cancel does nothing and
/// Move here moves.
///
/// parity: DND-018
#[gtk::test]
fn an_alt_drop_asks_with_the_drop_menu_and_runs_the_answer() {
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let dropped = [source.uri_of("Notes 2.txt")];

    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::OtherApp));
    let menu = test.window.drop_menu();
    wait_until("the drop menu", || menu.is_mapped());
    let labels = menu.row_labels();
    let bold = menu.emphasised_labels();
    test.activate("drop-choice", Some("cancel"));
    // Choosing an entry closes the menu; the action alone does not, and
    // no drop is taken while a menu is open.
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    wait_for(Duration::from_millis(100));
    let cancelled_kept_source = source.path("Notes 2.txt").is_file();
    let cancelled_made_nothing = !fixture.path("Documents/Notes 2.txt").exists();
    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::OtherApp));
    wait_until("the drop menu", || menu.is_mapped());
    test.activate("drop-choice", Some("move"));

    assert_eq!(
        labels,
        ["Copy here", "Move here", "Create shortcuts here", "-", "Cancel"]
    );
    assert_eq!(bold, ["Copy here"]);
    assert!(cancelled_kept_source && cancelled_made_nothing);
    wait_until("the move", || fixture.path("Documents/Notes 2.txt").is_file());
    wait_until("the original to go", || !source.path("Notes 2.txt").exists());
}

/// A right-button drag of this app's own items within their drive asks
/// with Windows Explorer's menu, Move here in bold; closing it unanswered,
/// as Escape does, does nothing, and Create shortcuts here makes `Notes 2.txt - Shortcut`.
///
/// parity: DND-027
#[gtk::test]
fn a_right_drag_drop_asks_and_creates_shortcuts_here() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let dropped = [fixture.uri_of("Notes 2.txt")];

    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::ThisApp));
    let menu = test.window.drop_menu();
    wait_until("the drop menu", || menu.is_mapped());
    let labels = menu.row_labels();
    let bold = menu.emphasised_labels();
    // Escape closes a popover as this does: the menu closes unanswered.
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    wait_for(Duration::from_millis(100));
    let escape_made_nothing = std::fs::read_dir(fixture.path("Documents"))
        .expect("the folder is read")
        .filter_map(Result::ok)
        .all(|entry| !entry.file_name().to_string_lossy().contains("Notes 2"));
    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::ThisApp));
    wait_until("the drop menu", || menu.is_mapped());
    menu.click_row("Create shortcuts here");

    assert_eq!(
        labels,
        ["Copy here", "Move here", "Create shortcuts here", "-", "Cancel"]
    );
    assert_eq!(bold, ["Move here"]);
    assert!(escape_made_nothing, "Escape does nothing");
    let shortcut = fixture.path("Documents/Notes 2.txt - Shortcut");
    wait_until("the shortcut", || shortcut.symlink_metadata().is_ok());
    assert_eq!(
        std::fs::read_link(&shortcut).expect("a symbolic link"),
        fixture.path("Notes 2.txt")
    );
    assert!(fixture.path("Notes 2.txt").is_file(), "the item stays");
}

/// The right-drag menu is text only, as in Windows Explorer: no glyph on
/// any row, the text where the other menus' text is. No tooltip shows
/// while it is open, and tooltips come back once it closes.
///
/// parity: DND-027
#[gtk::test]
fn the_right_drag_menu_is_text_only_and_hides_tooltips_while_open() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let dropped = [fixture.uri_of("Notes 2.txt")];
    let sidebar_row = |label: &str| {
        let index = test
            .window
            .sidebar()
            .labels()
            .iter()
            .position(|shown| shown == label);
        index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| test.window.sidebar().list().row_at_index(index))
            .expect("the sidebar shows the row")
    };
    let tooltip_before = sidebar_row("Documents").has_tooltip();

    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::ThisApp));
    let menu = test.window.drop_menu();
    // Mapped is not laid out: the labels are measured once they have a
    // width.
    wait_until("the drop menu to be laid out", || {
        let labels: Vec<gtk::Label> = menu
            .rows()
            .iter()
            .filter_map(|row| {
                crate::test_support::harness::descendants::<gtk::Label>(row)
                    .into_iter()
                    .next()
            })
            .collect();
        menu.is_mapped() && !labels.is_empty() && labels.iter().all(|label| label.width() > 0)
    });
    let glyphs: usize = menu
        .rows()
        .iter()
        .map(|row| crate::test_support::harness::descendants::<gtk::Image>(row).len())
        .sum();
    let text_starts: Vec<f32> = menu
        .rows()
        .iter()
        .filter_map(|row| {
            crate::test_support::harness::descendants::<gtk::Label>(row)
                .into_iter()
                .next()
                .map(|label| (row.clone(), label))
        })
        .filter_map(|(row, label)| label.compute_point(&row, &gtk::graphene::Point::zero()))
        .map(|point| point.x())
        .collect();
    let tooltip_while_open = sidebar_row("Documents").has_tooltip();
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    let tooltip_after = sidebar_row("Documents").has_tooltip();

    assert_eq!(glyphs, 0, "no glyphs, as in Windows Explorer");
    assert!(
        text_starts.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.5)
            && text_starts.first().is_some_and(|x| *x > 16.0),
        "the text keeps the glyph column's indent: {text_starts:?}"
    );
    assert!(
        tooltip_before && tooltip_after,
        "tooltips are back once it closes"
    );
    assert!(!tooltip_while_open, "no tooltip while the menu is open");
}

/// The window shows no tooltip while its items are dragged, once the drag
/// has started, and its tooltips come back once the drag ends.
///
/// parity: DND-027
#[gtk::test]
fn tooltips_are_off_while_items_are_dragged() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let row = test
        .window
        .sidebar()
        .list()
        .row_at_index(1)
        .expect("the sidebar has rows");
    let before = row.has_tooltip();

    test.window
        .drag_content_for(test.position_of("Notes 2.txt"))
        .expect("the item can be dragged");
    test.window.show_file_drag_feedback();
    wait_until("the tooltips to go off", || !row.has_tooltip());
    test.window.end_file_drag();

    assert!(before, "the row has a tooltip");
    assert!(row.has_tooltip(), "it is back once the drag ends");
}

/// Every window of the app turns its tooltips off while items are
/// dragged, not only the one they come from, as a drag can hover over
/// another window's rows; all come back once the drag ends.
///
/// parity: DND-027
#[gtk::test]
fn every_window_turns_its_tooltips_off_while_items_are_dragged() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    let row_of = |test: &TestWindow| {
        test.window
            .sidebar()
            .list()
            .row_at_index(1)
            .expect("the sidebar has rows")
    };
    let (row, other_row) = (row_of(&test), row_of(&other));
    let before = other_row.has_tooltip();

    test.window
        .drag_content_for(test.position_of("Notes 2.txt"))
        .expect("the item can be dragged");
    test.window.show_file_drag_feedback();
    wait_until("this window's tooltips to go off", || !row.has_tooltip());
    wait_until("the other window's tooltips to go off", || {
        !other_row.has_tooltip()
    });
    test.window.end_file_drag();

    assert!(before, "the other window's row has a tooltip");
    assert!(row.has_tooltip() && other_row.has_tooltip(), "both come back");
}

/// A right-button drag dropped in the folder it comes from asks, as in
/// Windows Explorer: Copy here and Create shortcuts here, but no Move
/// here, which would do nothing, and nothing in bold, as a plain drop
/// there does nothing. Copy here makes `Notes 2 - Copy.txt`.
///
/// parity: DND-027
#[gtk::test]
fn a_right_drop_in_its_own_folder_asks_and_copy_here_makes_a_copy() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let dropped = [fixture.uri_of("Notes 2.txt")];

    test.window
        .drop_files(&dropped, None, DropAction::Ask(DragOrigin::ThisApp));
    let menu = test.window.drop_menu();
    wait_until("the drop menu", || menu.is_mapped());
    let labels = menu.row_labels();
    let bold = menu.emphasised_labels();
    menu.click_row("Copy here");

    assert_eq!(labels, ["Copy here", "Create shortcuts here", "-", "Cancel"]);
    assert!(bold.is_empty(), "{bold:?}");
    let copy = fixture.path("Notes 2 - Copy.txt");
    wait_until("the copy", || copy.is_file());
    assert_eq!(
        std::fs::read(&copy).expect("the copy"),
        std::fs::read(fixture.path("Notes 2.txt")).expect("the item")
    );
    wait_until("the copy to be selected", || {
        test.selected_names() == ["Notes 2 - Copy.txt"]
    });
}

/// While items are dragged, the dragged items themselves take no drop: a
/// dragged folder dropped onto itself is dropped on the folder shown, as
/// in Windows Explorer, so a right-button drag asks there.
///
/// parity: DND-011, DND-027
#[gtk::test]
fn a_dragged_folder_is_no_drop_target_while_it_is_dragged() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let documents = test.position_of("Documents");

    let before = test.window.folder_view_destination(Some(documents));
    test.window
        .drag_content_for(documents)
        .expect("the folder can be dragged");
    test.window.show_file_drag_feedback();
    let while_dragged = test.window.folder_view_destination(Some(documents));
    test.window.end_file_drag();
    let after = test.window.folder_view_destination(Some(documents));

    let into_documents = Some(DropDestination::Folder(fixture.uri_of("Documents")));
    assert_eq!(before, into_documents);
    assert_eq!(while_dragged, Some(DropDestination::Folder(fixture.uri())));
    assert_eq!(after, into_documents);
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
