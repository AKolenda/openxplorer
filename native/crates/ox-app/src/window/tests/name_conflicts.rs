// SPDX-License-Identifier: AGPL-3.0-only
//! The name-conflict questions of a paste or drop, as Windows 11 asks
//! them (OPS-026, OPS-028): "Replace or Skip Files" in the progress
//! window, in place of the action's progress, and the "N File
//! Conflicts" window its "Compare info for both files" or "Let me decide
//! for each file" opens, which blocks only the progress window. Every
//! file is a fictional one of the fixture.

use std::fs;

use gtk::prelude::*;

use super::file_ops_support::{answer_conflicts, dialog_over, is_enabled, select_names, ConflictAnswer};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button as PointerButton, Pointer};
use crate::window::conflict_window::FileConflictWindow;
use crate::window::progress_window::ProgressWindow;

/// Copies the files `names` of Documents, each with other contents than
/// the file of that name in the fixture's folder, then shows that folder.
fn copy_namesakes(fixture: &Fixture, names: &[&str]) -> TestWindow {
    for name in names {
        fs::write(fixture.path(&format!("Documents/{name}")), b"Another file\n")
            .expect("the namesake is written");
    }
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    select_names(&test, names);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri()));
    wait_until("the fixture's folder", || {
        !test.window.is_loading() && test.names().iter().any(|name| name == "Documents")
    });
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test
}

/// Waits for the copy to end and checks that nothing reports the files
/// Skip left alone, as Windows says nothing more about them.
fn assert_no_report_of_skipped_files(test: &TestWindow) {
    wait_until("the copy to end", || !test.window.is_writing_files());
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    assert!(dialog_over(test).is_none(), "no report of the skipped files");
}

/// The progress window, once it shows the question.
pub(super) fn question_window(test: &TestWindow) -> ProgressWindow {
    wait_until("the Replace or Skip Files question", || {
        test.context.progress_window().is_some_and(|window| {
            window.is_visible() && window.title().as_deref() == Some("Replace or Skip Files")
        })
    });
    test.context.progress_window().expect("it shows")
}

/// Whether `widget` and every widget it is in are visible.
pub(super) fn is_shown(widget: &impl IsA<gtk::Widget>) -> bool {
    let mut current = Some(widget.clone().upcast::<gtk::Widget>());
    while let Some(shown) = current {
        if !shown.is_visible() {
            return false;
        }
        current = shown.parent();
    }
    true
}

/// The texts of every shown label inside `widget`.
pub(super) fn texts(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    descendants::<gtk::Label>(widget)
        .into_iter()
        .filter(is_shown)
        .map(|label| label.text().to_string())
        .collect()
}

/// The button inside `widget` that shows `text`.
pub(super) fn button(widget: &impl IsA<gtk::Widget>, text: &str) -> gtk::Button {
    descendants::<gtk::Button>(widget)
        .into_iter()
        .find(|button| is_shown(button) && texts(button).iter().any(|shown| shown == text))
        .unwrap_or_else(|| panic!("a button {text:?} in {:?}", texts(widget)))
}

/// The File Conflict window over `progress`, once it opens.
pub(super) fn conflict_window(progress: &ProgressWindow) -> FileConflictWindow {
    wait_until("the File Conflict window", || {
        progress
            .conflict_window()
            .is_some_and(|window| window.is_visible())
    });
    progress.conflict_window().expect("it shows")
}

/// The check box inside `widget` whose label shows `text`.
fn check(widget: &impl IsA<gtk::Widget>, text: &str) -> gtk::CheckButton {
    descendants::<gtk::CheckButton>(widget)
        .into_iter()
        .find(|check| is_shown(check) && texts(check).iter().any(|shown| shown == text))
        .unwrap_or_else(|| panic!("a check box {text:?} in {:?}", texts(widget)))
}

/// One taken name asks in the progress window, with Windows' singular
/// wording, and nothing over the browsing window, which stays usable;
/// Replace puts the incoming file in place.
///
/// parity: OPS-026, OPS-028
#[gtk::test]
fn one_taken_name_asks_replace_or_skip_in_the_progress_window() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt"]);

    test.activate("paste", None);
    let window = question_window(&test);
    let shown = texts(&window);

    assert!(
        shown.contains(&"Copying 1 item from Documents to Example projects".to_owned()),
        "{shown:?}"
    );
    assert!(
        shown.contains(&"The destination already has a file named \"Notes 2.txt\"".to_owned()),
        "{shown:?}"
    );
    for row in [
        "Replace the file in the destination",
        "Skip this file",
        "Compare info for both files",
    ] {
        let row = button(&window, row);
        assert!(row.is_focusable() && row.can_focus(), "keyboard reachable");
    }
    assert!(
        dialog_over(&test).is_none(),
        "nothing is asked over the browsing window"
    );
    assert!(test.window.is_sensitive());
    assert!(
        GtkWindowExt::focus(&window).is_none() && !window.gets_focus_visible(),
        "no focus ring until the keyboard is used, as in Windows"
    );
    assert!(
        !window.actions()[0].shows_cancel(),
        "alone, the question has no Cancel of its own: the window's close button cancels"
    );

    button(&window, "Replace the file in the destination").emit_clicked();
    wait_until("the file to be replaced", || {
        fs::read(fixture.path("Notes 2.txt")).is_ok_and(|contents| contents == b"Another file\n")
    });
}

/// Several taken names take Windows' plural wording; Skip these files
/// leaves every existing file as it was.
///
/// parity: OPS-026, OPS-028
#[gtk::test]
fn several_taken_names_ask_in_the_plural_and_skip_keeps_them() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("paste", None);
    let window = question_window(&test);
    let shown = texts(&window);

    assert!(
        shown.contains(&"The destination has 2 files with the same names".to_owned()),
        "{shown:?}"
    );
    button(&window, "Replace the files in the destination");
    button(&window, "Let me decide for each file");
    button(&window, "Skip these files").emit_clicked();

    wait_until("the action to end", || test.context.progress_window().is_none());
    assert_no_report_of_skipped_files(&test);
    for name in ["Notes 2.txt", "Notes 10.txt"] {
        assert_eq!(fs::read(fixture.path(name)).unwrap(), b"Synthetic test data\n");
    }
}

/// Closing the window stops the action while it asks, as in Windows:
/// nothing changes.
///
/// parity: OPS-026
#[gtk::test]
fn cancelling_the_question_changes_nothing() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt"]);
    let before = test.names();

    test.activate("paste", None);
    let window = question_window(&test);
    window.close();

    wait_until("the action to end", || test.context.progress_window().is_none());
    test.window.refresh();
    test.wait_for_listing("the folder again");
    assert_eq!(test.names(), before);
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).unwrap(),
        b"Synthetic test data\n"
    );
}

/// "Compare info for both files" opens "1 File Conflict", a window of
/// the progress window that blocks only it. Continue waits until the file
/// has a side; both sides keep both, the copy numbered as Windows does.
///
/// parity: OPS-028, XFER-008
#[gtk::test]
fn the_file_conflict_window_keeps_both_with_a_number() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt"]);

    test.activate("paste", None);
    let progress = question_window(&test);
    button(&progress, "Compare info for both files").emit_clicked();
    let conflicts = conflict_window(&progress);
    let shown = texts(&conflicts);

    assert_eq!(conflicts.title().as_deref(), Some("1 File Conflict"));
    assert_eq!(conflicts.transient_for().as_ref(), Some(progress.upcast_ref()));
    assert!(conflicts.is_modal());
    assert!(
        GtkWindowExt::focus(&conflicts).is_none() && !conflicts.gets_focus_visible(),
        "no focus ring until the keyboard is used, as in Windows"
    );
    let group = conflicts.group().list_windows();
    assert!(group.contains(progress.upcast_ref()));
    assert!(
        !group.contains(test.window.upcast_ref()),
        "the browsing window is not blocked"
    );
    for text in [
        "Which files do you want to keep?",
        "If you select both versions, the copied file will have a number added to its name.",
        "Notes 2.txt",
    ] {
        assert!(shown.contains(&text.to_owned()), "{text:?} in {shown:?}");
    }
    let continue_button = button(&conflicts, "Continue");
    button(&conflicts, "Cancel");
    assert!(!continue_button.is_sensitive(), "no side is chosen yet");
    let (incoming, existing) = conflicts.row_checks(0);
    for side in [&incoming, &existing] {
        assert!(
            gtk::test_accessible_has_property(side, gtk::AccessibleProperty::Label),
            "each side's check box is named"
        );
    }
    incoming.set_active(true);
    assert!(continue_button.is_sensitive(), "one side is a choice");
    existing.set_active(true);
    assert!(continue_button.is_sensitive());
    continue_button.emit_clicked();

    wait_until("both versions", || fixture.path("Notes 2 (2).txt").is_file());
    assert_eq!(
        fs::read(fixture.path("Notes 2 (2).txt")).unwrap(),
        b"Another file\n"
    );
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).unwrap(),
        b"Synthetic test data\n"
    );
}

/// "Let me decide for each file" lists every file; a header check box
/// takes one side for all of them.
///
/// parity: OPS-028
#[gtk::test]
fn a_header_check_box_chooses_one_side_for_every_file() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("paste", None);
    let progress = question_window(&test);
    button(&progress, "Let me decide for each file").emit_clicked();
    let conflicts = conflict_window(&progress);

    assert_eq!(conflicts.title().as_deref(), Some("2 File Conflicts"));
    let shown = texts(&conflicts);
    for text in [
        "Files from Documents",
        "Files already in Example projects",
        "Notes 10.txt",
    ] {
        assert!(shown.contains(&text.to_owned()), "{text:?} in {shown:?}");
    }
    let (header, _) = conflicts.header_checks();
    assert!(gtk::test_accessible_has_property(
        &header,
        gtk::AccessibleProperty::Label
    ));
    header.set_active(true);
    let continue_button = button(&conflicts, "Continue");
    assert!(continue_button.is_sensitive());
    continue_button.emit_clicked();

    wait_until("both files to be replaced", || {
        ["Notes 2.txt", "Notes 10.txt"]
            .iter()
            .all(|name| fs::read(fixture.path(name)).is_ok_and(|contents| contents == b"Another file\n"))
    });
}

/// Clicks the middle of `widget` with the private display's real pointer.
fn click(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    let point = screen_point(pointer, widget, middle, (720, 500));
    pointer.move_to(point.0, point.1);
    pointer.press(PointerButton::Primary);
    pointer.release(PointerButton::Primary);
}

/// Puts the real pointer inside `window`, where its keyboard input and a
/// window manager's close request go.
fn point_into(pointer: &mut Pointer, window: &impl IsA<gtk::Widget>) {
    let inside = screen_point(pointer, window, (40.0, 40.0), (720, 500));
    pointer.move_to(inside.0, inside.1);
}

/// Waits until the File Conflict window over `progress` has closed, then
/// checks that the same question still asks, unanswered, and that the
/// existing file is as it was.
fn back_at_the_question(test: &TestWindow, fixture: &Fixture, progress: &ProgressWindow, how: &str) {
    wait_until(&format!("the File Conflict window to close with {how}"), || {
        progress.conflict_window().is_none()
            && !gtk::Window::list_toplevels()
                .iter()
                .any(|window| window.is::<FileConflictWindow>() && window.is_visible())
    });
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    assert!(test.window.is_writing_files(), "{how}: the copy still waits");
    assert_eq!(
        test.context.progress_window().as_ref(),
        Some(progress),
        "{how}: the same progress window"
    );
    assert_eq!(
        progress.title().as_deref(),
        Some("Replace or Skip Files"),
        "{how}"
    );
    wait_until(&format!("the question to wait again after {how}"), || {
        progress.actions()[0].waits_for_a_row()
    });
    let shown = texts(progress);
    assert!(
        shown.contains(&"The destination already has a file named \"Notes 2.txt\"".to_owned()),
        "{how}: {shown:?}"
    );
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).unwrap(),
        b"Synthetic test data\n",
        "{how}: nothing was written"
    );
}

/// With the real pointer and keyboard, as Windows 11 does: closing the
/// File Conflict window with its close button (X), its Cancel or Escape
/// closes only that window. The copy keeps waiting at the same "Replace
/// or Skip Files" question, unanswered, and Skip still answers it.
///
/// parity: OPS-026, OPS-028
#[gtk::test]
fn closing_the_file_conflict_window_returns_to_the_question() {
    let mut pointer = Pointer::connect();
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt"]);

    test.activate("paste", None);
    let progress = question_window(&test);
    crate::test_support::harness::wait_for_frames(&progress, 4);

    click(&mut pointer, &button(&progress, "Compare info for both files"));
    let conflicts = conflict_window(&progress);
    crate::test_support::harness::wait_for_frames(&conflicts, 4);
    // The close button (X): the desktop's close request, as a window
    // manager sends it when its title bar's X is clicked.
    point_into(&mut pointer, &conflicts);
    pointer.close_window_under_pointer();
    back_at_the_question(&test, &fixture, &progress, "the close button");

    click(&mut pointer, &button(&progress, "Compare info for both files"));
    let conflicts = conflict_window(&progress);
    crate::test_support::harness::wait_for_frames(&conflicts, 4);
    click(&mut pointer, &button(&conflicts, "Cancel"));
    back_at_the_question(&test, &fixture, &progress, "Cancel");

    click(&mut pointer, &button(&progress, "Compare info for both files"));
    let conflicts = conflict_window(&progress);
    crate::test_support::harness::wait_for_frames(&conflicts, 4);
    // The keyboard goes to the window under the pointer.
    point_into(&mut pointer, &conflicts);
    pointer.tap_key(gtk::gdk::Key::Escape);
    back_at_the_question(&test, &fixture, &progress, "Escape");

    crate::test_support::harness::wait_for_frames(&progress, 4);
    click(&mut pointer, &button(&progress, "Skip this file"));
    assert_no_report_of_skipped_files(&test);
    assert_eq!(
        fs::read(fixture.path("Notes 2.txt")).unwrap(),
        b"Synthetic test data\n",
        "Skip left the file as it was"
    );
}

/// "Skip 1 file with the same date and size" shows when two files match,
/// and skips them.
///
/// parity: OPS-028
#[gtk::test]
fn files_with_the_same_date_and_size_can_be_skipped_at_once() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/Notes 2.txt"), b"Synthetic test data\n").unwrap();
    let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
    for name in ["Notes 2.txt", "Documents/Notes 2.txt"] {
        fs::File::options()
            .write(true)
            .open(fixture.path(name))
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri()));
    wait_until("Paste to be enabled", || {
        !test.window.is_loading() && is_enabled(&test, "paste")
    });

    test.activate("paste", None);
    let progress = question_window(&test);
    button(&progress, "Compare info for both files").emit_clicked();
    let conflicts = conflict_window(&progress);
    let same = check(&conflicts, "Skip 1 file with the same date and size");
    same.set_active(true);
    let continue_button = button(&conflicts, "Continue");
    assert!(continue_button.is_sensitive(), "the skipped file needs no side");
    continue_button.emit_clicked();

    wait_until("the action to end", || test.context.progress_window().is_none());
    assert_no_report_of_skipped_files(&test);
    assert!(!fixture.path("Notes 2 (2).txt").exists());
}

/// Replace keeps the replaced file in the Recycle Bin, and the journal
/// records it: Undo Copy takes the copy away and brings the replaced file
/// back, and Redo replaces it again.
///
/// parity: OPS-028, OPS-029, OPS-031
#[gtk::test]
fn undo_brings_back_the_file_a_paste_replaced() {
    let fixture = Fixture::standard();
    let test = copy_namesakes(&fixture, &["Notes 2.txt"]);

    test.activate("paste", None);
    answer_conflicts(&test, ConflictAnswer::Replace);
    wait_until("the file to be replaced", || {
        fs::read(fixture.path("Notes 2.txt")).is_ok_and(|contents| contents == b"Another file\n")
    });
    wait_until("Undo Copy", || is_enabled(&test, "undo"));

    test.activate("undo", None);
    wait_until("the replaced file to come back", || {
        fs::read(fixture.path("Notes 2.txt")).is_ok_and(|contents| contents == b"Synthetic test data\n")
    });
    wait_until("Redo Copy", || is_enabled(&test, "redo"));
    test.activate("redo", None);
    wait_until("the file to be replaced again", || {
        fs::read(fixture.path("Notes 2.txt")).is_ok_and(|contents| contents == b"Another file\n")
    });
    assert_eq!(
        fs::read(fixture.path("Documents/Notes 2.txt")).unwrap(),
        b"Another file\n",
        "the source stays"
    );
}
