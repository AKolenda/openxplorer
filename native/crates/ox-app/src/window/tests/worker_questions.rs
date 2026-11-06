// SPDX-License-Identifier: AGPL-3.0-only
//! The questions an operation's worker thread asks through a dialog over
//! the window: which button gives the engine which answer.

use std::thread;

use gtk::prelude::*;
use ox_core::transfer::{
    Cancellation, FailedItem, FailureAnswer, FailureCause, MoveByCopyingItem, TransferMode, UnstorableAnswer,
    UnstorableItem, UnstorableReason,
};

use super::file_ops_support::{dialog_over, open_dialog, wait_for_no_dialog};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::window::widget_tree::descendants;

/// Runs `ask` on a worker thread, as the engine does, presses `button` in
/// the dialog it opens over `test`, and returns the worker's answer.
fn answer_on_worker<A: Send + 'static>(
    test: &TestWindow,
    ask: impl FnOnce() -> A + Send + 'static,
    button: &str,
) -> A {
    let worker = thread::spawn(ask);
    open_dialog(test).press(button);
    wait_for_no_dialog(test);
    wait_until("the worker to get its answer", || worker.is_finished());
    worker.join().expect("the worker finishes")
}

/// parity: XFER-028
#[gtk::test]
fn the_unstorable_name_dialog_answers_skip_or_cancel_to_the_worker() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let item = UnstorableItem {
        name: "a:b.txt".into(),
        reason: UnstorableReason::InvalidCharacters,
    };

    for (button, expected) in [
        ("Skip", UnstorableAnswer::Skip),
        ("Cancel", UnstorableAnswer::Cancel),
    ] {
        let asker = test.window.unstorable_asker();
        let question = item.clone();
        let answer = answer_on_worker(&test, move || asker.ask(&question), button);
        assert_eq!(answer, expected, "{button}");
    }
}

/// parity: XFER-011, XFER-013
#[gtk::test]
fn a_move_is_finished_by_copying_only_when_the_user_agrees() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let item = MoveByCopyingItem {
        name: "Photos".into(),
        destination: "USB stick".into(),
    };

    for (button, agrees) in [("Copy, then remove originals", true), ("Cancel", false)] {
        let asker = test.window.move_by_copying_asker();
        let question = item.clone();
        let answer = answer_on_worker(&test, move || asker.ask(&question), button);
        assert_eq!(answer, agrees, "{button}");
    }
}

/// A file the user may not read asks Windows' "File Access Denied", which
/// names the file with its Type, Size and Date modified; Try Again, Skip
/// with "Do this for all current items" and Cancel give the engine Retry,
/// Skip all and Cancel.
///
/// parity: OPS-047
#[gtk::test]
fn file_access_denied_answers_try_again_skip_or_cancel_to_the_worker() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let item = FailedItem {
        mode: TransferMode::Copy,
        name: "Notes 2.txt".into(),
        error: "Error opening file: Permission denied".into(),
        more_items: true,
        cause: FailureCause::AccessDenied,
        uri: fixture.uri_of("Notes 2.txt"),
        destination_folder: Some(fixture.uri_of("Documents")),
        needed: None,
    };

    for (button, for_all, expected) in [
        ("Try Again", false, FailureAnswer::Retry),
        ("Skip", false, FailureAnswer::Skip),
        ("Skip", true, FailureAnswer::SkipAll),
        ("Cancel", false, FailureAnswer::Cancel),
    ] {
        let asker = test.window.failure_asker(&Cancellation::new());
        let question = item.clone();
        let worker = thread::spawn(move || asker.ask(&question));
        let dialog = open_dialog(&test);
        assert_eq!(dialog.title().unwrap_or_default(), "File Access Denied");
        assert_eq!(dialog.title_text(), "File Access Denied");
        assert_eq!(
            dialog.message_text(),
            "You need permission to perform this action"
        );
        let texts = dialog.texts();
        for line in ["Notes 2.txt", "Size: "] {
            assert!(
                texts.iter().any(|text| text.starts_with(line)),
                "{line} in {texts:?}"
            );
        }
        assert!(
            !texts.iter().any(|text| text.contains("Permission denied")),
            "no raw system text: {texts:?}"
        );
        assert_eq!(dialog.button_labels(), ["Try Again", "Skip", "Cancel"]);
        let check = descendants::<gtk::CheckButton>(&dialog)
            .into_iter()
            .next()
            .expect("Do this for all current items");
        assert_eq!(check.label().as_deref(), Some("Do this for all current items"));
        check.set_active(for_all);
        dialog.press(button);
        wait_for_no_dialog(&test);
        wait_until("the worker to get its answer", || worker.is_finished());
        assert_eq!(worker.join().expect("the worker finishes"), expected, "{button}");
    }
}

/// A file that is gone asks "Item Not Found" with where it was; one that
/// is held asks "File In Use"; neither shows the system's text.
///
/// parity: OPS-047
#[gtk::test]
fn a_gone_or_held_file_asks_in_windows_words() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folder = fixture.root().display().to_string();
    for (cause, caption, heading) in [
        (
            FailureCause::NotFound,
            "Item Not Found",
            "Could not find this item",
        ),
        (
            FailureCause::InUse,
            "File In Use",
            "The action can't be completed because the file is open in another program",
        ),
    ] {
        let item = FailedItem {
            mode: TransferMode::Move,
            name: "Gone.txt".into(),
            error: "No such file or directory".into(),
            more_items: false,
            cause,
            uri: fixture.uri_of("Gone.txt"),
            destination_folder: Some(fixture.uri_of("Documents")),
            needed: None,
        };
        let asker = test.window.failure_asker(&Cancellation::new());
        let worker = thread::spawn(move || asker.ask(&item));
        let dialog = open_dialog(&test);
        assert_eq!(dialog.title().unwrap_or_default(), caption);
        assert_eq!(dialog.title_text(), caption);
        assert!(
            dialog.message_text().starts_with(heading),
            "{}",
            dialog.message_text()
        );
        if cause == FailureCause::NotFound {
            assert_eq!(
                dialog.message_text(),
                format!(
                    "{heading}\n\nThis is no longer located in {folder}. Verify the item's location and try again."
                )
            );
        }
        assert!(
            descendants::<gtk::CheckButton>(&dialog).is_empty(),
            "no check box with nothing left"
        );
        dialog.press("Skip");
        wait_for_no_dialog(&test);
        wait_until("the worker to get its answer", || worker.is_finished());
        assert_eq!(worker.join().expect("the worker finishes"), FailureAnswer::Skip);
    }
}

/// A full drive asks in the operation's action, as Windows' "There is
/// not enough space" does, with Try Again and Cancel.
///
/// parity: OPS-047, XFER-028
#[gtk::test]
fn a_full_drive_asks_in_the_progress_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let cancel = Cancellation::new();
    assert!(test.window.claim_transfer_panel(
        crate::window::transfer_panel::TransferKind::Files,
        "Copying…",
        &cancel
    ));
    let item = FailedItem {
        mode: TransferMode::Copy,
        name: "Documents".into(),
        error: "Not enough free space".into(),
        more_items: false,
        cause: FailureCause::NoSpace,
        uri: String::new(),
        destination_folder: Some(fixture.uri_of("Documents")),
        needed: Some(5_000_000),
    };

    for (retry, expected) in [(true, FailureAnswer::Retry), (false, FailureAnswer::Cancel)] {
        let asker = test.window.failure_asker(&cancel);
        let question = item.clone();
        let worker = thread::spawn(move || asker.ask(&question));
        let panel = test.window.transfer_panel().clone();
        wait_until("the action to ask", || panel.asks_about_space());
        let (summary, needed) = panel.space_texts_shown();
        assert!(summary.starts_with("There is not enough space on "), "{summary}");
        assert!(
            needed.ends_with("is needed to copy this item. Delete or move files so you have enough space."),
            "{needed}"
        );
        assert!(dialog_over(&test).is_none(), "no dialog over the window");
        panel.press_space_answer(retry);
        wait_until("the worker to get its answer", || worker.is_finished());
        assert_eq!(worker.join().expect("the worker finishes"), expected);
        assert!(!panel.is_asking(), "the progress shows again");
    }
    test.window.end_operation();
}
