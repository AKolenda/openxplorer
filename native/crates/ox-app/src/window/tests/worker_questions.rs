// SPDX-License-Identifier: AGPL-3.0-only
//! The questions an operation's worker thread asks through a dialog over
//! the window: which button gives the engine which answer.

use std::thread;

use ox_core::transfer::{
    FailedItem, FailureAnswer, MoveByCopyingItem, TransferMode, UnstorableAnswer, UnstorableItem,
    UnstorableReason,
};

use super::file_ops_support::{open_dialog, wait_for_no_dialog};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

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

/// The failed-item dialog answers Retry, Skip all or Cancel to the worker.
#[gtk::test]
fn the_failed_item_dialog_answers_retry_skip_all_or_cancel_to_the_worker() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let item = FailedItem {
        mode: TransferMode::Copy,
        name: "report.pdf".into(),
        error: "Permission denied".into(),
        more_items: true,
    };

    for (button, expected) in [
        ("Retry", FailureAnswer::Retry),
        ("Skip all", FailureAnswer::SkipAll),
        ("Cancel", FailureAnswer::Cancel),
    ] {
        let asker = test.window.failure_asker();
        let question = item.clone();
        let answer = answer_on_worker(&test, move || asker.ask(&question), button);
        assert_eq!(answer, expected, "{button}");
    }
}
