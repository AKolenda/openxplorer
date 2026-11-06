// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and Paste through the display's clipboard, against `copySelection`, `paste` and
//! `transferWithConflicts` of `v2.0.0:desktop/ui/app.js`: the refusals, the
//! dimming of cut items and what a clipboard manager may keep. What other
//! applications put on the clipboard is in `clipboard_interop`; the
//! name-conflict questions are in `name_conflicts`.

use std::fs;
use std::os::unix::net::UnixListener;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{CUSTOM, GNOME, KDE_CUT, TEXT, URI_LIST};
use ox_core::settings::ContextMenu;

use super::context_menus::choose_menu_style;
use super::file_ops_support::{
    answer_conflicts, is_enabled, open_dialog, press_shortcut, select_names, wait_for_no_dialog,
    ConflictAnswer,
};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// The modifier of Ctrl+C, Ctrl+X and Ctrl+V.
const CONTROL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;

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

    assert_eq!(copied, "", "copying shows no message, as Windows Explorer");
    wait_until("the copy to be selected", || {
        test.selected_names() == ["Notes 2.txt"]
    });
    assert!(fixture.path("Documents/Notes 2.txt").is_file());
    assert!(fixture.path("Notes 2.txt").is_file(), "a copy keeps its source");
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// The menu of one selected folder pastes into that folder, not into
/// the folder shown.
///
/// parity: CMD-019
#[gtk::test]
fn paste_into_folder_pastes_into_the_selected_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The classic menu, which this test reads; Windows 11's compact
    // menu is the default.
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    select_names(&test, &["Documents"]);
    wait_until("Paste into folder to be enabled", || {
        is_enabled(&test, "paste-into")
    });
    test.window
        .right_click(Some(super::context_menus::position_of(&test, "Documents")));
    let menu = test.window.context_menu();

    menu.row("Paste into folder").emit_activate();

    wait_until("the copy in Documents", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the folder shown stays"
    );
}

/// parity: CLIP-002, CLIP-006, CLIP-008
#[gtk::test]
fn cut_then_paste_moves_the_items_and_empties_the_cut() {
    let fixture = Fixture::standard();
    let test = clipboard_then_documents(&fixture, "cut", &["Notes 10.txt"]);

    test.activate("paste", None);

    wait_until("the moved item", || {
        fixture.path("Documents/Notes 10.txt").is_file()
    });
    super::file_ops_support::assert_ended_without_a_message(&test);
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

/// Pasting a copy into the folder it came from makes "Notes 2 - Copy.txt"
/// at once, as Windows Explorer does: the item's own name is not a
/// conflict to ask about.
///
/// parity: OPS-026, XFER-008
#[gtk::test]
fn pasting_a_copy_into_its_own_folder_makes_a_copy_without_asking() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.activate("paste", None);

    wait_until("the copy, or a question", || {
        fixture.path("Notes 2 - Copy.txt").is_file() || super::file_ops_support::dialog_over(&test).is_some()
    });
    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing is asked"
    );
    wait_until("the copy to be selected", || {
        test.selected_names() == ["Notes 2 - Copy.txt"]
    });
    assert_eq!(
        fs::read(fixture.path("Notes 2 - Copy.txt")).expect("copied"),
        fs::read(fixture.path("Notes 2.txt")).expect("kept")
    );
}

/// The names in `folder`, sorted.
fn names_in(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder is readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Lets the main loop run for a while, long enough for a paste to have
/// asked or moved something.
fn wait_a_moment() {
    let waited = std::rc::Rc::new(std::cell::Cell::new(false));
    let done = std::rc::Rc::clone(&waited);
    glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || done.set(true));
    wait_until("a moment", || waited.get());
}

/// Cutting a file and pasting it back into its own folder does nothing,
/// as in Windows Explorer: no "Items already exist" question, and the
/// file stays where it is, unchanged.
///
/// parity: OPS-026, CLIP-003
#[gtk::test]
fn pasting_a_cut_item_into_its_own_folder_does_nothing() {
    let fixture = Fixture::standard();
    let before = names_in(fixture.root());
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("cut", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.activate("paste", None);
    wait_a_moment();

    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing is asked"
    );
    assert_eq!(names_in(fixture.root()), before, "nothing moved or copied");
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).expect("kept"),
        b"Synthetic test data\n"
    );
    wait_until("the window to be free again", || is_enabled(&test, "paste"));
}

/// Moving items into a folder some of them are already in, as a cut
/// from another app may hold items of several folders, moves the others
/// and leaves those where they are, without asking.
///
/// parity: OPS-026
#[gtk::test]
fn a_move_into_the_folder_an_item_is_in_moves_only_the_others() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/Plan.txt"), b"Plan\n").expect("a file in Documents");
    let test = TestWindow::open(&fixture.uri());

    let incoming = crate::window::file_ops::IncomingItems {
        mode: ox_core::transfer::TransferMode::Move,
        uris: vec![
            fixture.uri_of("Notes 2.txt"),
            fixture.uri_of("Documents/Plan.txt"),
        ],
        destination_folder: fixture.uri(),
    };
    let window = test.window.clone();
    glib::spawn_future_local(async move {
        window.transfer_with_conflicts(incoming).await;
    });

    wait_until("Plan.txt to move in", || fixture.path("Plan.txt").is_file());
    wait_a_moment();
    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing is asked"
    );
    assert!(!fixture.path("Documents/Plan.txt").exists());
    assert!(fixture.path("Notes 2.txt").is_file(), "Notes 2.txt stays");
    assert!(!fixture.path("Notes 2 - Copy.txt").exists());
}

/// One drop brings a file into its own folder and another file of the
/// same name. The first becomes "Notes 2 - Copy.txt" without asking, as
/// Windows names a copy in its own folder; the second is asked about, and
/// keeping both numbers it "Notes 2 (2).txt", as Windows' File Conflict
/// window does.
///
/// parity: OPS-026, OPS-028, XFER-008
#[gtk::test]
fn an_automatic_copy_and_a_kept_both_copy_take_their_own_names() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/Notes 2.txt"), b"Another file\n").expect("the namesake is written");
    let test = TestWindow::open(&fixture.uri());

    let taken = test.window.drop_files(
        &[
            fixture.uri_of("Notes 2.txt"),
            fixture.uri_of("Documents/Notes 2.txt"),
        ],
        None,
        crate::window::file_drop::DropAction::Copy,
    );
    assert!(taken);
    answer_conflicts(&test, ConflictAnswer::KeepBoth);

    wait_until("both copies", || {
        fixture.path("Notes 2 - Copy.txt").is_file() && fixture.path("Notes 2 (2).txt").is_file()
    });
    assert_eq!(
        fs::read(fixture.path("Notes 2 - Copy.txt")).expect("the automatic copy"),
        b"Synthetic test data\n"
    );
    assert_eq!(
        fs::read(fixture.path("Notes 2 (2).txt")).expect("the kept copy"),
        b"Another file\n"
    );
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).expect("the existing file stays"),
        b"Synthetic test data\n"
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
        "text/plain;charset=utf-8",
        "application/x-kde-cutselection",
    ] {
        assert!(formats.contain_mime_type(mime_type), "{mime_type}");
    }
    assert_eq!(KDE_CUT, "application/x-kde-cutselection");
    assert_eq!(clipboard_bytes(&test, KDE_CUT), b"1");
    let gnome = String::from_utf8(clipboard_bytes(&test, "x-special/gnome-copied-files")).expect("UTF-8");
    assert_eq!(gnome, format!("cut\n{}", fixture.uri_of("Notes 2.txt")));
    // The text is the item's path unquoted, as Nautilus publishes it, so a
    // terminal pastes a path a shell reads as one argument.
    let text = String::from_utf8(clipboard_bytes(&test, TEXT)).expect("UTF-8");
    let path = fixture.path("Notes 2.txt").to_string_lossy().into_owned();
    assert_eq!(text, path);
}

/// Several items are one path per line in the text format, without quotes
/// and without a trailing newline.
///
/// parity: CLIP-004
#[gtk::test]
fn a_copy_of_several_items_publishes_their_paths_one_per_line() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let names = ["Notes 2.txt", "Résumé.txt"];
    select_names(&test, &names);

    test.activate("copy", None);

    let text = String::from_utf8(clipboard_bytes(&test, TEXT)).expect("UTF-8");
    let paths: Vec<String> = names
        .iter()
        .map(|name| fixture.path(name).to_string_lossy().into_owned())
        .collect();
    assert_eq!(text, paths.join("\n"));
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
    assert!(!fixture.path("Notes 2 - Copy.txt").exists());
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
    let copied = String::from_utf8(clipboard_bytes(&test, URI_LIST)).expect("a URI list");
    assert!(copied.contains("Plan.txt"), "Ctrl+C copies: {copied}");
    assert_eq!(test.window.shown_message(), cut_message, "and adds no message");
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
    let copied = String::from_utf8(clipboard_bytes(&test, URI_LIST)).expect("a URI list");
    assert!(copied.contains("Notes%202.txt"), "Ctrl+C copies: {copied}");
    assert_eq!(message, "", "copying shows no message");
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
    for mime_type in [CUSTOM, GNOME, URI_LIST, TEXT, KDE_CUT] {
        assert!(storable.contain_mime_type(mime_type), "{mime_type}");
    }
}

/// Copies `Alpha.txt` and `Beta.txt` of the fixture's folder, the file
/// called `unreadable` without read permission, and starts pasting them
/// into Documents: the copy stops at that file with the failed-item
/// dialog, which this returns. Tests run as a normal user: root reads the
/// file anyway.
fn paste_with_an_unreadable_file(fixture: &Fixture, unreadable: &str) -> (TestWindow, crate::dialog::Dialog) {
    use std::os::unix::fs::PermissionsExt;

    fixture.write("Alpha.txt");
    fixture.write("Beta.txt");
    fs::set_permissions(fixture.path(unreadable), fs::Permissions::from_mode(0o000))
        .expect("take read permission away");
    let test = clipboard_then_documents(fixture, "copy", &["Alpha.txt", "Beta.txt"]);
    test.activate("paste", None);
    let failed = open_dialog(&test);
    assert!(
        failed.button_labels().contains(&"Try Again".to_owned()),
        "the failed-item dialog"
    );
    (test, failed)
}

/// Cancel in the failed-item dialog just stops, as in Windows Explorer:
/// no "Operation result" follows, the item copied before stays, and
/// Ctrl+Z takes that copy back.
///
/// parity: OPS-023, OPS-029
#[gtk::test]
fn cancel_in_the_failed_item_dialog_just_stops_and_undo_takes_back_what_was_copied() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    let (test, failed) = paste_with_an_unreadable_file(&fixture, "Beta.txt");

    failed.press("Cancel");
    wait_until("the finished copy to be selected", || {
        test.selected_names() == ["Alpha.txt"]
    });

    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing is reported after Cancel"
    );
    assert!(!fixture.path("Documents/Beta.txt").exists());
    test.activate("undo", None);
    wait_until("Undo to take the copy back", || {
        !fixture.path("Documents/Alpha.txt").exists()
    });
    assert!(fixture.path("Alpha.txt").is_file(), "the original stays");
}

/// The progress panel's Cancel stops a copy without a report, also after
/// an item was skipped.
///
/// parity: OPS-019, OPS-023
#[gtk::test]
fn the_panels_cancel_stops_a_copy_without_a_report() {
    let fixture = Fixture::standard();
    let (test, failed) = paste_with_an_unreadable_file(&fixture, "Alpha.txt");

    test.activate("cancel-operation", None);
    failed.press("Skip");
    wait_for_no_dialog(&test);
    wait_until("the copy to end", || !test.window.is_writing_files());
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));

    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing is reported after Cancel"
    );
    assert!(!fixture.path("Documents/Beta.txt").exists(), "the copy stopped");
}
