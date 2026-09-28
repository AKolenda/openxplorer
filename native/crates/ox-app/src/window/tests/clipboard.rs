// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and Paste through the display's clipboard, and the
//! name-conflict dialog, against `copySelection`, `paste` and
//! `transferWithConflicts` of `desktop/ui/app.js`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::KDE_CUT;

use super::file_ops_support::{is_enabled, open_dialog, select_names, wait_for_no_dialog};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

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

    wait_until("the item to be moved", || {
        fixture.path("Documents/Notes 10.txt").is_file()
    });
    assert!(!fixture.path("Notes 10.txt").exists());
    wait_until("the moved item to leave the clipboard", || {
        !is_enabled(&test, "paste")
    });
    assert_eq!(test.window.shown_message(), "1 item(s) moved.");
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
    assert_eq!(
        dialog.button_labels(),
        ["Cancel", "Skip duplicates", "Keep both", "Replace existing"]
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
    wait_until("the search to filter", || {
        test.window.folder_model().is_searching()
    });

    test.window.paste_from_keyboard();

    wait_until("the refusal", || {
        test.window.shown_message() == "Open the destination folder before pasting."
    });
    assert!(!fixture.path("Notes 2 (copy 2).txt").exists());
}
