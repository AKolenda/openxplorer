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
use crate::window::progress_window::ProgressWindow;
use crate::window::transfer_panel::{TransferKind, TransferPanel};

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

/// Asks `item` on a worker thread, as the engine does, in the action of
/// the operation `cancel` stops, and returns the worker and that action
/// once the question shows there.
fn ask_in_action(
    test: &TestWindow,
    cancel: &Cancellation,
    item: &FailedItem,
) -> (thread::JoinHandle<FailureAnswer>, TransferPanel) {
    let asker = test.window.failure_asker(cancel);
    let question = item.clone();
    let worker = thread::spawn(move || asker.ask(&question));
    let panel = test.window.transfer_panel().clone();
    wait_until("the action to ask", || panel.asks_about_failure());
    (worker, panel)
}

/// Starts an operation that `cancel` stops, whose action asks.
fn start_operation(test: &TestWindow) -> Cancellation {
    let cancel = Cancellation::new();
    assert!(test
        .window
        .claim_transfer_panel(TransferKind::Files, "Copying…", &cancel));
    cancel
}

/// Waits for the worker's answer, and checks that the action shows its
/// progress again.
fn answer_of(worker: thread::JoinHandle<FailureAnswer>, panel: &TransferPanel) -> FailureAnswer {
    wait_until("the worker to get its answer", || worker.is_finished());
    assert!(!panel.is_asking(), "the progress shows again");
    worker.join().expect("the worker finishes")
}

/// A file the user may not read asks Windows' "File Access Denied" in the
/// progress window, in place of the action's progress, naming the file
/// with its Type, Size and Date modified; Try Again, Skip with "Do this
/// for all current items" and Cancel give the engine Retry, Skip all and
/// Cancel. No dialog opens over the window.
///
/// parity: OPS-047
#[gtk::test]
fn file_access_denied_answers_try_again_skip_or_cancel_to_the_worker() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let cancel = start_operation(&test);
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
        let (worker, panel) = ask_in_action(&test, &cancel, &item);
        assert_eq!(panel.caption_shown(), "File Access Denied");
        assert_eq!(
            ProgressWindow::of(&test.context).title().as_deref(),
            Some("1 Interrupted Action")
        );
        let texts = panel.failure_texts();
        for line in [
            "You need permission to perform this action",
            "Notes 2.txt",
            "Size: ",
            "Do this for all current items",
        ] {
            assert!(
                texts.iter().any(|text| text.starts_with(line)),
                "{line} in {texts:?}"
            );
        }
        assert!(
            !texts.iter().any(|text| text.contains("Permission denied")),
            "no raw system text: {texts:?}"
        );
        assert_eq!(panel.failure_button_labels(), ["Try Again", "Skip", "Cancel"]);
        assert!(
            !panel.failure_text_is_selectable(),
            "plain text, nothing selected"
        );
        assert!(
            panel.failure_focus_is_on_first_button(),
            "the keyboard on the first button"
        );
        assert!(!panel.failure_shows_shield());
        assert!(dialog_over(&test).is_none(), "no dialog over the window");
        assert!(panel.set_failure_for_all(for_all));
        panel.press_failure(button);
        assert_eq!(answer_of(worker, &panel), expected, "{button}");
    }
    test.window.end_operation();
}

/// A destination folder the user may not write to asks Windows'
/// "Destination Folder Access Denied" about the folder in the progress
/// window, with Continue and its administrator shield, Skip and Cancel.
/// Continue answers Continue, and with "Do this for all current items"
/// Continue for all; a folder the administrator check refuses (this
/// test's own) is answered without any password prompt, so the engine
/// says why. The progress window's More details shows the source and the
/// destination. Tests run as a normal user: root may write anywhere.
///
/// parity: OPS-047
#[gtk::test]
fn destination_folder_access_denied_offers_continue_with_the_shield() {
    let fixture = Fixture::standard();
    let locked = Locked::new(fixture.path("Documents"));
    let test = TestWindow::open(&fixture.uri());
    let cancel = start_operation(&test);
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

    for (for_all, expected) in [(false, FailureAnswer::Elevate), (true, FailureAnswer::ElevateAll)] {
        let (worker, panel) = ask_in_action(&test, &cancel, &item);
        assert_eq!(panel.caption_shown(), "Destination Folder Access Denied");
        let texts = panel.failure_texts();
        for line in [
            "You'll need to provide administrator permission to copy to this folder",
            "Documents",
            "Source",
            "Destination",
        ] {
            assert!(texts.iter().any(|text| text == line), "{line} in {texts:?}");
        }
        assert!(
            texts.iter().any(|text| text.ends_with("Notes 2.txt")),
            "{texts:?}"
        );
        assert_eq!(panel.failure_button_labels(), ["Continue", "Skip", "Cancel"]);
        assert!(
            !panel.failure_text_is_selectable(),
            "plain text, nothing selected"
        );
        assert!(
            panel.failure_focus_is_on_first_button(),
            "the keyboard on the first button"
        );
        assert!(panel.failure_shows_shield());
        let window = ProgressWindow::of(&test.context);
        assert_eq!(panel.failure_details_shown(), window.shows_details());
        window.press_details();
        assert_eq!(
            panel.failure_details_shown(),
            window.shows_details(),
            "the window's More details"
        );
        assert!(dialog_over(&test).is_none(), "no dialog over the window");
        assert!(panel.set_failure_for_all(for_all));
        panel.press_failure("Continue");
        assert_eq!(answer_of(worker, &panel), expected);
    }
    test.window.end_operation();
    drop(locked);
}

/// An item to delete from a folder the user may not change asks Windows'
/// "File Access Denied", "You'll need to provide administrator permission
/// to delete this file", with Continue and its shield. In a delete run
/// Continue answers Continue, with "Do this for all current items" for
/// all. The Recycle Bin has no administrator path, so in a Recycle Bin
/// run Continue first asks in the same place whether to delete the file
/// permanently, also when the settings say not to confirm deletes: No
/// asks the first question again, Yes answers Continue for this item
/// alone. This test's folder fails the administrator check,
/// so no password is asked for.
///
/// parity: OPS-047, OPS-018
#[gtk::test]
fn an_item_in_a_folder_only_an_administrator_may_change_offers_continue() {
    let fixture = Fixture::standard();
    fixture.write("Documents/tool.cfg");
    let locked = Locked::new(fixture.path("Documents"));
    let test = TestWindow::open(&fixture.uri());
    let cancel = start_operation(&test);
    let item = |mode| FailedItem {
        mode,
        name: "tool.cfg".into(),
        error: "Error removing file: Permission denied".into(),
        more_items: true,
        cause: FailureCause::AccessDenied,
        uri: fixture.uri_of("Documents/tool.cfg"),
        destination_folder: None,
        needed: None,
    };

    for (for_all, expected) in [(false, FailureAnswer::Elevate), (true, FailureAnswer::ElevateAll)] {
        let (worker, panel) = ask_in_action(&test, &cancel, &item(TransferMode::Delete));
        assert_eq!(panel.caption_shown(), "File Access Denied");
        let texts = panel.failure_texts();
        assert!(
            texts
                .iter()
                .any(|text| text == "You'll need to provide administrator permission to delete this file"),
            "{texts:?}"
        );
        assert!(texts.iter().any(|text| text == "tool.cfg"), "{texts:?}");
        assert_eq!(panel.failure_button_labels(), ["Continue", "Skip", "Cancel"]);
        assert!(
            !panel.failure_text_is_selectable(),
            "plain text, nothing selected"
        );
        assert!(
            panel.failure_focus_is_on_first_button(),
            "the keyboard on the first button"
        );
        assert!(panel.failure_shows_shield());
        assert!(panel.set_failure_for_all(for_all));
        panel.press_failure("Continue");
        assert_eq!(answer_of(worker, &panel), expected);
    }

    test.context
        .remember_preferences(ox_core::settings::PreferencesUpdate {
            confirm_delete: Some(false),
            ..ox_core::settings::PreferencesUpdate::default()
        });
    wait_until("the setting to apply", || {
        !test.context.settings_data().preferences.confirm_delete
    });
    let (worker, panel) = ask_in_action(&test, &cancel, &item(TransferMode::Trash));
    assert!(panel.set_failure_for_all(true));
    panel.press_failure("Continue");
    wait_until("the permanent delete question", || {
        panel.asks_about_failure() && panel.caption_shown() == "Delete File"
    });
    let texts = panel.failure_texts();
    assert!(
        texts.iter().any(|text| text
            == "This file can't be moved to the Recycle Bin. Do you want to permanently delete it?"),
        "{texts:?}"
    );
    assert_eq!(panel.failure_button_labels(), ["Yes", "No"]);
    panel.press_failure("No");
    wait_until("the first question again", || {
        panel.asks_about_failure() && panel.caption_shown() == "File Access Denied"
    });
    panel.press_failure("Continue");
    wait_until("the permanent delete question", || {
        panel.asks_about_failure() && panel.caption_shown() == "Delete File"
    });
    panel.press_failure("Yes");
    assert_eq!(
        answer_of(worker, &panel),
        FailureAnswer::Elevate,
        "for this item alone"
    );
    test.window.end_operation();
    drop(locked);
}

/// A file that is gone asks "Item Not Found" with where it was; one that
/// is held asks "File In Use"; neither shows the system's text, and both
/// ask in the progress window.
///
/// parity: OPS-047
#[gtk::test]
fn a_gone_or_held_file_asks_in_windows_words() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let cancel = start_operation(&test);
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
        let (worker, panel) = ask_in_action(&test, &cancel, &item);
        assert_eq!(panel.caption_shown(), caption);
        let texts = panel.failure_texts();
        assert!(texts[0].starts_with(heading), "{texts:?}");
        if cause == FailureCause::NotFound {
            assert_eq!(
                texts[0],
                format!(
                    "{heading}\n\nThis is no longer located in {folder}. Verify the item's location and try again."
                )
            );
        }
        assert!(!panel.set_failure_for_all(true), "no check box with nothing left");
        assert!(dialog_over(&test).is_none(), "no dialog over the window");
        panel.press_failure("Skip");
        assert_eq!(answer_of(worker, &panel), FailureAnswer::Skip);
    }
    test.window.end_operation();
}

/// Takes write permission from a folder, and gives it back when dropped,
/// also when the test fails, so the fixture can be removed.
struct Locked(std::path::PathBuf);

impl Locked {
    fn new(folder: std::path::PathBuf) -> Self {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).expect("lock the folder");
        Self(folder)
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
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
    assert!(test
        .window
        .claim_transfer_panel(TransferKind::Files, "Copying…", &cancel));
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
