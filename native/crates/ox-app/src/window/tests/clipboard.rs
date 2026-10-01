// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and Paste through the display's clipboard, and the
//! name-conflict dialog, against `copySelection`, `paste` and
//! `transferWithConflicts` of `v2.0.0:desktop/ui/app.js`: the refusals, the
//! dimming of cut items and what a clipboard manager may keep. What other
//! applications put on the clipboard is in `clipboard_interop`.

use std::fs;
use std::os::unix::net::UnixListener;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{CUSTOM, GNOME, KDE_CUT, URI_LIST};

use super::file_ops_support::{is_enabled, open_dialog, press_shortcut, select_names, wait_for_no_dialog};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

/// The modifier of Ctrl+C, Ctrl+X and Ctrl+V.
const CONTROL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;

/// The toast after copying one item.
const ONE_ITEM_COPIED: &str = "1 item(s) copied — ready to paste in another window.";

/// Whether the views show the item called `name` dimmed as cut.
fn is_shown_cut(test: &TestWindow, name: &str) -> bool {
    let index = test
        .names()
        .iter()
        .position(|shown| shown == name)
        .expect("the item is listed");
    let position = u32::try_from(index).expect("a listing has fewer than u32::MAX items");
    test.window.folder_pane().owners().is_shown_cut(position) == Some(true)
}

/// Shows the Documents folder of `fixture` in `test`, and waits until the
/// clipboard's files enable Paste there.
fn show_documents(test: &TestWindow, fixture: &Fixture) {
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(test, "paste"));
}

/// A window on `fixture` with `names` put on the clipboard by `command`
/// ("copy" or "cut"), then showing the fixture's Documents folder.
fn clipboard_then_documents(fixture: &Fixture, command: &str, names: &[&str]) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, names);
    test.activate(command, None);
    show_documents(&test, fixture);
    test
}

/// parity: CLIP-001, CLIP-003, CLIP-004, OPS-023, OPS-027
#[gtk::test]
fn copy_then_paste_copies_into_the_folder_shown_and_selects_the_copy() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.activate("copy", None);
    let copied = test.window.shown_message();
    show_documents(&test, &fixture);
    test.activate("paste", None);

    assert_eq!(copied, "1 item(s) copied — ready to paste in another window.");
    wait_until("the copy to be selected", || {
        test.selected_names() == ["Notes 2.txt"]
    });
    assert!(fixture.path("Documents/Notes 2.txt").is_file());
    assert!(fixture.path("Notes 2.txt").is_file(), "a copy keeps its source");
    assert_eq!(test.window.shown_message(), "1 item(s) copied.");
}

/// parity: CLIP-002, CLIP-006, CLIP-008
#[gtk::test]
fn cut_then_paste_moves_the_items_and_empties_the_cut() {
    let fixture = Fixture::standard();
    let test = clipboard_then_documents(&fixture, "cut", &["Notes 10.txt"]);

    test.activate("paste", None);

    // Paste is also disabled while the move runs, and the file is in
    // place before the move is reported, so the report comes first.
    wait_until("the move to be reported", || {
        test.window.shown_message() == "1 item(s) moved."
    });
    assert!(fixture.path("Documents/Notes 10.txt").is_file());
    assert!(!fixture.path("Notes 10.txt").exists());
    wait_until("the moved item to leave the clipboard", || {
        !is_enabled(&test, "paste")
    });
}

/// parity: CLIP-005, CLIP-009, CLIP-016
#[gtk::test]
fn text_on_the_clipboard_is_no_file_list_and_disables_paste() {
    let fixture = Fixture::standard();
    let test = clipboard_then_documents(&fixture, "copy", &["Notes 2.txt"]);

    test.window.clipboard().set_text(&fixture.uri_of("Notes 10.txt"));

    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
}

/// parity: OPS-026, OPS-028, XFER-008
#[gtk::test]
fn pasting_onto_a_taken_name_asks_and_keep_both_keeps_both() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.activate("paste", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Items already exist");
    let expected_start = format!("1 matching name(s) in {}\n\n", fixture.root().display());
    assert!(
        dialog.message_text().starts_with(&expected_start),
        "{}",
        dialog.message_text()
    );
    // The copy is of the existing item itself, which may not replace
    // itself.
    assert_eq!(
        dialog.button_labels(),
        ["Cancel", "Skip duplicates", "Keep both", "Rename"]
    );
    assert!(
        descendants::<gtk::CheckButton>(&dialog).is_empty(),
        "one conflict needs no Apply to all"
    );
    dialog.press("Keep both");
    wait_until("the second copy to be selected", || {
        test.selected_names()
            .first()
            .is_some_and(|name| name.starts_with("Notes 2 (copy"))
    });
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// parity: OPS-026
#[gtk::test]
fn cancelling_the_conflict_dialog_changes_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    let names_before = test.names();

    test.activate("paste", None);
    open_dialog(&test).press("Cancel");
    wait_for_no_dialog(&test);

    test.window.refresh();
    test.wait_for_listing("the folder again");
    assert_eq!(test.names(), names_before);
}

/// parity: OPS-028
#[gtk::test]
fn clearing_apply_to_all_asks_about_each_conflict_in_turn() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.activate("paste", None);
    let first = open_dialog(&test);
    let apply_to_all = descendants::<gtk::CheckButton>(&first)
        .into_iter()
        .next()
        .expect("several conflicts offer Apply to all");
    assert_eq!(apply_to_all.label().as_deref(), Some("Apply to all 2 items"));
    assert!(apply_to_all.is_active());
    apply_to_all.set_active(false);
    first.press("Skip duplicates");
    wait_until("the second question", || {
        super::file_ops_support::dialog_over(&test)
            .is_some_and(|dialog| dialog != first && dialog.message_text().starts_with("1 matching"))
    });
    open_dialog(&test).press("Keep both");

    wait_until("the answered copy to be selected", || {
        test.selected_names()
            .first()
            .is_some_and(|name| name.starts_with("Notes 10 (copy"))
    });
    let copies: Vec<String> = test
        .names()
        .into_iter()
        .filter(|name| name.contains("(copy"))
        .collect();
    assert_eq!(copies.len(), 1, "Notes 2.txt was skipped: {copies:?}");
    let report = open_dialog(&test);
    assert_eq!(
        report.title_text(),
        "Operation result",
        "the skipped item is reported"
    );
    report.press("OK");
}

/// parity: OPS-028
#[gtk::test]
fn rename_in_the_conflict_dialog_copies_under_the_typed_name() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.activate("paste", None);
    let dialog = open_dialog(&test);
    let new_name = descendants::<gtk::Entry>(&dialog)
        .into_iter()
        .next()
        .expect("the dialog has a New name field");
    assert_eq!(
        new_name.text(),
        "Notes 2 (copy 2).txt",
        "a free name is suggested"
    );
    new_name.set_text("Notes 10.txt");
    dialog.press("Rename");
    wait_until("the taken name to be refused", || dialog.error_text().is_some());
    new_name.set_text("Renamed notes.txt");
    dialog.press("Rename");

    wait_until("the renamed copy to be selected", || {
        test.selected_names() == ["Renamed notes.txt"]
    });
    assert_eq!(
        fs::read(fixture.path("Renamed notes.txt")).expect("the copy exists"),
        fs::read(fixture.path("Notes 2.txt")).expect("the original stays")
    );
}

/// Marks the file at `path` as last modified an hour ago.
fn make_an_hour_old(path: &std::path::Path) {
    let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let file = fs::File::options()
        .write(true)
        .open(path)
        .expect("the fixture is ours");
    file.set_modified(an_hour_ago).expect("the fixture is ours");
}

/// "Replace older" replaces only the files whose existing copy is older;
/// Cancel has the focus, so a reflexive Enter changes nothing.
///
/// parity: OPS-028
#[gtk::test]
fn replace_older_replaces_only_the_older_existing_files() {
    let fixture = Fixture::standard();
    for name in ["Documents/Notes 2.txt", "Documents/Notes 10.txt"] {
        fs::write(fixture.path(name), b"existing").expect("the fixture is ours");
    }
    make_an_hour_old(&fixture.path("Documents/Notes 2.txt"));
    make_an_hour_old(&fixture.path("Notes 10.txt"));
    let test = clipboard_then_documents(&fixture, "copy", &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("paste", None);
    let dialog = open_dialog(&test);
    let focused = GtkWindowExt::focus(&dialog).and_downcast::<gtk::Button>();
    assert_eq!(
        focused.and_then(|button| button.label()).as_deref(),
        Some("Cancel")
    );
    dialog.press("Replace older");
    wait_until("the report of one copied and one skipped", || {
        super::file_ops_support::dialog_over(&test).is_some_and(|report| {
            report != dialog && report.message_text().starts_with("1 completed.\n1 skipped")
        })
    });
    let report = super::file_ops_support::dialog_over(&test).expect("the report");
    report.press("OK");

    assert_eq!(
        fs::read(fixture.path("Documents/Notes 2.txt")).expect("the older copy was replaced"),
        b"Synthetic test data\n"
    );
    assert_eq!(
        fs::read(fixture.path("Documents/Notes 10.txt")).expect("the newer copy stays"),
        b"existing"
    );
}

/// The bytes the clipboard of `test`'s window offers as `mime_type`.
fn clipboard_bytes(test: &TestWindow, mime_type: &str) -> Vec<u8> {
    let clipboard = test.window.clipboard();
    glib::MainContext::default().block_on(async {
        let (stream, _) = clipboard
            .read_future(&[mime_type], glib::Priority::DEFAULT)
            .await
            .expect("the clipboard offers the format");
        let bytes = stream
            .read_bytes_future(4096, glib::Priority::DEFAULT)
            .await
            .expect("the payload reads");
        bytes.to_vec()
    })
}

/// parity: CLIP-004, CLIP-007
#[gtk::test]
fn a_cut_offers_every_format_and_the_kde_marker_under_its_real_mime_type() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.activate("cut", None);

    let formats = test.window.clipboard().formats();
    for mime_type in [
        "application/x-winspace-files",
        "x-special/gnome-copied-files",
        "text/uri-list",
        "application/x-kde-cutselection",
    ] {
        assert!(formats.contain_mime_type(mime_type), "{mime_type}");
    }
    assert_eq!(KDE_CUT, "application/x-kde-cutselection");
    assert_eq!(clipboard_bytes(&test, KDE_CUT), b"1");
    let gnome = String::from_utf8(clipboard_bytes(&test, "x-special/gnome-copied-files")).expect("UTF-8");
    assert_eq!(gnome, format!("cut\n{}", fixture.uri_of("Notes 2.txt")));
}

/// parity: CLIP-005, CLIP-006, CLIP-007
#[gtk::test]
fn a_cut_from_dolphin_pastes_as_a_move() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let uri_list = format!("{}\r\n", fixture.uri_of("Notes 2.txt"));
    let dolphin_cut = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(uri_list.into_bytes())),
        gdk::ContentProvider::for_bytes(KDE_CUT, &glib::Bytes::from_static(b"1")),
    ]);

    test.window
        .clipboard()
        .set_content(Some(&dolphin_cut))
        .expect("the test owns the clipboard");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);

    wait_until("the item to be moved", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    assert!(!fixture.path("Notes 2.txt").exists(), "a Dolphin cut is a move");
}

/// parity: CLIP-011
#[gtk::test]
fn paste_during_a_search_asks_to_open_the_destination_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.window.search_box().entry().set_text("Notes");
    wait_until("the search to filter", || test.window.is_searching());

    test.window.paste_from_keyboard();

    wait_until("the refusal", || {
        test.window.shown_message() == "Open the destination folder before pasting."
    });
    assert!(!fixture.path("Notes 2 (copy 2).txt").exists());
}

/// parity: CLIP-002, CLIP-009, LOOK-014
#[gtk::test]
fn a_cut_dims_its_items_in_both_views_until_the_clipboard_changes() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 10.txt"]);

    test.activate("cut", None);

    wait_until("the cut row to be dimmed", || is_shown_cut(&test, "Notes 10.txt"));
    assert!(!is_shown_cut(&test, "Notes 2.txt"), "only the cut item is dimmed");
    test.activate("view", Some("large"));
    wait_until("the cut tile to be dimmed", || {
        is_shown_cut(&test, "Notes 10.txt")
    });
    test.window.clipboard().set_text("Quarterly plan");
    wait_until("the dimming to end", || !is_shown_cut(&test, "Notes 10.txt"));
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    assert!(!is_shown_cut(&test, "Notes 10.txt"), "a copy dims nothing");
}

/// parity: CLIP-001, CLIP-011
#[gtk::test]
fn ctrl_c_on_an_item_that_is_no_file_or_folder_asks_to_open_the_share() {
    let fixture = Fixture::standard();
    let _socket = UnixListener::bind(fixture.path("studio.sock")).expect("a socket in the fixture");
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["studio.sock"]);

    press_shortcut(&test, gdk::Key::c, CONTROL);

    assert_eq!(
        test.window.shown_message(),
        "Open the share first, then select its files or folders."
    );
    assert!(!is_enabled(&test, "copy"), "the Copy command stays off");
}

/// parity: CLIP-002, CLIP-011
#[gtk::test]
fn ctrl_x_in_a_previous_version_is_refused_and_ctrl_c_copies() {
    let fixture = Fixture::standard();
    let snapshot = fixture.path(".snapshot/Monday");
    fs::create_dir_all(&snapshot).expect("a snapshot folder in the fixture");
    fs::write(snapshot.join("Plan.txt"), b"Synthetic test data\n").expect("a file in the snapshot");
    let test = TestWindow::open(&fixture.uri_of(".snapshot/Monday"));
    select_names(&test, &["Plan.txt"]);

    press_shortcut(&test, gdk::Key::x, CONTROL);
    let cut_message = test.window.shown_message();
    press_shortcut(&test, gdk::Key::c, CONTROL);

    assert_eq!(
        cut_message,
        "Previous versions are read-only. Use Restore a copy."
    );
    assert_eq!(test.window.shown_message(), ONE_ITEM_COPIED);
}

/// `onKey` copies even while an operation runs, where the Copy button is
/// off.
///
/// parity: CLIP-001
#[gtk::test]
fn ctrl_c_copies_while_an_operation_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    let _operation = test
        .window
        .begin_operation("Copy: Notes 10.txt (1/1)")
        .expect("no other operation runs");

    press_shortcut(&test, gdk::Key::c, CONTROL);

    let message = test.window.shown_message();
    test.window.end_operation();
    assert_eq!(message, ONE_ITEM_COPIED);
}

/// GTK stores a local clipboard's storable formats with the clipboard
/// manager when the application quits, as `gtk_clipboard_set_can_store`
/// asked GTK 3 to.
///
/// parity: CLIP-010
#[gtk::test]
fn every_published_format_is_offered_for_a_clipboard_manager_to_keep() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);

    test.activate("copy", None);

    let clipboard = test.window.clipboard();
    assert!(clipboard.is_local(), "the window owns the clipboard");
    let content = clipboard.content().expect("a local clipboard has content");
    let storable = content.storable_formats();
    for mime_type in [CUSTOM, GNOME, URI_LIST, KDE_CUT] {
        assert!(storable.contain_mime_type(mime_type), "{mime_type}");
    }
}
