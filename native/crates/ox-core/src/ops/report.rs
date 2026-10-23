// SPDX-License-Identifier: AGPL-3.0-only
//! What the user is told when an operation ends (OPS-023).
//!
//! Ports the end of `runOperation` in `v2.0.0:desktop/ui/app.js`: complete success
//! is a short message, which the window does not show (Windows Explorer
//! says nothing); errors or skipped items open the "Operation result"
//! dialog listing each; a request refused before it started opens
//! "Operation stopped" with the refusal. Unlike `app.js`, a cancelled
//! operation says nothing, as in Windows Explorer, unless a failure was
//! never shown or something needs recovery by hand.

use super::journal::JournalDirection;
use super::undo::UndoRecord;
use crate::transfer::{TransferMode, TransferResult};

/// The title of the dialog that lists what an operation did.
pub const RESULT_TITLE: &str = crate::i18n::message_id("Operation result");

/// The title of the dialog for a request that failed before any item was
/// handled; its text is the error's message.
pub const STOPPED_TITLE: &str = crate::i18n::message_id("Operation stopped");

/// How the end of an operation is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationSummary {
    /// Every item succeeded: a message such as `3 item(s) copied.`, which
    /// the window does not show, as in Windows Explorer.
    Toast(String),
    /// Something went wrong or was skipped: the [`RESULT_TITLE`] dialog
    /// with one line per fact.
    Report(String),
    /// The user cancelled: nothing is said, as in Windows Explorer, also
    /// when items were skipped before or failed in the question the user
    /// answered. A failure no question showed, such as a Recycle Bin item
    /// that could not be restored, and a problem the user must sort out by
    /// hand are still a [`Self::Report`].
    Cancelled,
}

/// The summary of a finished `mode` run, word for word as `app.js`
/// writes it.
pub fn summarize(mode: TransferMode, result: &TransferResult) -> OperationSummary {
    let message = match mode {
        TransferMode::Copy => crate::i18n::message_id("{count} item(s) copied."),
        TransferMode::Move => crate::i18n::message_id("{count} item(s) moved."),
        TransferMode::Delete => crate::i18n::message_id("{count} item(s) permanently deleted."),
        TransferMode::Trash => crate::i18n::message_id("{count} item(s) sent to Trash."),
    };
    summarize_items(message, result)
}

/// The summary of Duplicate, which the Python app did not have:
/// `2 item(s) duplicated.` in the wording of the other toasts.
pub fn summarize_duplicate(result: &TransferResult) -> OperationSummary {
    summarize_items(crate::i18n::message_id("{count} item(s) duplicated."), result)
}

/// The summary of Restore from the Recycle Bin: `2 item(s) restored.`
pub fn summarize_restore(result: &TransferResult) -> OperationSummary {
    summarize_items(crate::i18n::message_id("{count} item(s) restored."), result)
}

/// The summary of Create links, which the Python app did not have:
/// `2 item(s) linked.` in the wording of the other toasts.
pub fn summarize_links(result: &TransferResult) -> OperationSummary {
    summarize_items(crate::i18n::message_id("{count} item(s) linked."), result)
}

/// The summary of a batch rename (OPS-014): `3 item(s) renamed.`
pub fn summarize_batch_rename(result: &TransferResult) -> OperationSummary {
    summarize_items(crate::i18n::message_id("{count} item(s) renamed."), result)
}

/// The summary of an Undo: `Rename undone.` when every step succeeded,
/// otherwise the [`RESULT_TITLE`] report of what happened.
pub fn summarize_undo(record: &UndoRecord, result: &TransferResult) -> OperationSummary {
    summarize_journal_step(JournalDirection::Undo, record.title(), result)
}

/// The summary of an Undo or Redo of the operation titled `title`:
/// `Rename undone.` or `Rename redone.` when every step succeeded,
/// otherwise the [`RESULT_TITLE`] report of what happened, or nothing
/// once cancelled.
pub fn summarize_journal_step(
    direction: JournalDirection,
    title: &str,
    result: &TransferResult,
) -> OperationSummary {
    if !is_complete_success(result) {
        return problems(result);
    }
    let message = match direction {
        JournalDirection::Undo => crate::i18n::message_id("{title} undone."),
        JournalDirection::Redo => crate::i18n::message_id("{title} redone."),
    };
    OperationSummary::Toast(crate::i18n::format_message(message, &[("title", title)]))
}

/// `N item(s) <verb>.` when every item succeeded, otherwise the report,
/// or nothing once cancelled.
fn summarize_items(message: &str, result: &TransferResult) -> OperationSummary {
    if is_complete_success(result) {
        return OperationSummary::Toast(crate::i18n::format_message(
            message,
            &[("count", &result.done.len().to_string())],
        ));
    }
    problems(result)
}

/// What is said about a run that did not fully succeed: once the user
/// cancelled, only the failures no question showed and what needs
/// recovery by hand, otherwise the report. Each presented message
/// silences one failure, so of two failures with the same message the
/// one never asked about is still reported.
fn problems(result: &TransferResult) -> OperationSummary {
    if !result.cancelled {
        return OperationSummary::Report(report_lines(result));
    }
    let mut presented: Vec<&String> = result.presented.iter().collect();
    let unsaid: Vec<&str> = result
        .errors
        .iter()
        .filter(|error| {
            if result.recovery.contains(error) {
                return true;
            }
            let Some(seen) = presented.iter().position(|message| message == error) else {
                return true;
            };
            presented.swap_remove(seen);
            false
        })
        .map(String::as_str)
        .collect();
    if unsaid.is_empty() {
        return OperationSummary::Cancelled;
    }
    OperationSummary::Report(unsaid.join("\n"))
}

/// True when nothing failed, nothing was skipped and nobody cancelled.
fn is_complete_success(result: &TransferResult) -> bool {
    result.errors.is_empty() && result.skipped.is_empty() && !result.cancelled
}

/// `N completed.`, then the skipped count and each error, one per line.
fn report_lines(result: &TransferResult) -> String {
    let mut lines = vec![crate::i18n::format_message(
        "{count} completed.",
        &[("count", &(result.done.len()).to_string())],
    )];
    if !result.skipped.is_empty() {
        lines.push(crate::i18n::format_message(
            "{count} skipped (name already exists).",
            &[("count", &(result.skipped.len()).to_string())],
        ));
    }
    lines.extend(result.errors.iter().cloned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::results::merge_results;

    fn result(done: usize, skipped: usize, errors: &[&str], cancelled: bool) -> TransferResult {
        TransferResult {
            done: vec![String::from("file:///tmp/done"); done],
            skipped: vec![String::from("file:///tmp/skipped"); skipped],
            errors: errors.iter().map(ToString::to_string).collect(),
            cancelled,
            ..TransferResult::default()
        }
    }

    /// One mode and the toast for two successful items.
    struct ToastCase {
        mode: TransferMode,
        toast: &'static str,
    }

    #[test]
    fn complete_success_is_a_toast_naming_the_operation() {
        let cases = [
            ToastCase {
                mode: TransferMode::Copy,
                toast: "2 item(s) copied.",
            },
            ToastCase {
                mode: TransferMode::Move,
                toast: "2 item(s) moved.",
            },
            ToastCase {
                mode: TransferMode::Delete,
                toast: "2 item(s) permanently deleted.",
            },
            ToastCase {
                mode: TransferMode::Trash,
                toast: "2 item(s) sent to Trash.",
            },
        ];
        for case in cases {
            let summary = summarize(case.mode, &result(2, 0, &[], false));

            assert_eq!(summary, OperationSummary::Toast(case.toast.into()));
        }
    }

    #[test]
    fn skips_and_errors_are_listed_line_by_line() {
        let outcome = result(1, 2, &["b.txt: Permission denied"], false);

        let summary = summarize(TransferMode::Copy, &outcome);

        let expected = "1 completed.\n2 skipped (name already exists).\nb.txt: Permission denied";
        assert_eq!(summary, OperationSummary::Report(expected.into()));
    }

    /// Windows Explorer says nothing after Cancel, also when items were
    /// skipped before it or failed in the question the user answered.
    ///
    /// parity: OPS-023
    #[test]
    fn a_cancelled_run_says_nothing() {
        let record = UndoRecord::Rename {
            original_uri: "file:///tmp/a".into(),
            renamed_uri: "file:///tmp/b".into(),
        };
        let denied = "b.txt: Permission denied";
        let cancelled = [
            result(0, 0, &[], true),
            TransferResult {
                presented: vec![denied.to_owned()],
                ..result(1, 2, &[denied], true)
            },
        ];

        for outcome in &cancelled {
            assert_eq!(
                summarize(TransferMode::Copy, outcome),
                OperationSummary::Cancelled
            );
            assert_eq!(summarize_duplicate(outcome), OperationSummary::Cancelled);
            assert_eq!(summarize_undo(&record, outcome), OperationSummary::Cancelled);
        }
    }

    /// Cancel never hides a problem the user must sort out by hand, such
    /// as a kept backup; only those lines are reported.
    ///
    /// parity: OPS-023, XFER-003
    #[test]
    fn a_cancelled_run_still_reports_what_needs_recovery() {
        let denied = "a.txt: Permission denied";
        let backup = "document: Replacement completed, but the prior file remains at file:///tmp/b.";
        let outcome = TransferResult {
            recovery: vec![backup.to_owned()],
            presented: vec![denied.to_owned()],
            ..result(1, 1, &[denied, backup], true)
        };

        let summary = summarize(TransferMode::Copy, &outcome);

        assert_eq!(summary, OperationSummary::Report(backup.into()));
    }

    /// Cancel never hides a failure no question showed, as Restore, a
    /// batch rename and Undo only collect theirs; the failures the user
    /// answered stay unsaid.
    ///
    /// parity: OPS-023
    #[test]
    fn a_cancelled_run_reports_the_failures_no_question_showed() {
        let record = UndoRecord::Rename {
            original_uri: "file:///tmp/a".into(),
            renamed_uri: "file:///tmp/b".into(),
        };
        let answered = "a.txt: Permission denied";
        let unseen = "b.txt: An item named “b.txt” already exists in its original folder.";
        let outcome = TransferResult {
            presented: vec![answered.to_owned()],
            ..result(1, 0, &[answered, unseen], true)
        };

        let report = OperationSummary::Report(unseen.into());
        assert_eq!(summarize_restore(&outcome), report);
        assert_eq!(summarize_batch_rename(&outcome), report);
        assert_eq!(summarize_undo(&record, &outcome), report);
    }

    /// Two failures with the same message, such as a special file of the
    /// same name in two duplicated folders, one answered with "Skip all"
    /// and one left out without asking: Cancel still reports the second,
    /// also once the results of the two runs are added up.
    ///
    /// parity: OPS-023, OPS-047
    #[test]
    fn a_cancelled_run_reports_each_unseen_failure_with_a_seen_message() {
        let pipe = "tree/pipe: Sockets, devices and other special files are not copied.";
        let first = TransferResult {
            presented: vec![pipe.to_owned()],
            ..result(1, 0, &[pipe], false)
        };
        let second = result(0, 0, &[pipe], true);
        let mut total = TransferResult::default();
        merge_results(&mut total, first);
        merge_results(&mut total, second);

        let summary = summarize_duplicate(&total);

        assert_eq!(summary, OperationSummary::Report(pipe.into()));
    }

    #[test]
    fn an_undo_names_the_operation_it_reversed() {
        let record = UndoRecord::Rename {
            original_uri: "file:///tmp/a".into(),
            renamed_uri: "file:///tmp/b".into(),
        };

        let done = summarize_undo(&record, &result(1, 0, &[], false));
        let failed = summarize_undo(&record, &result(0, 0, &["b: gone"], false));

        assert_eq!(done, OperationSummary::Toast("Rename undone.".into()));
        assert_eq!(failed, OperationSummary::Report("0 completed.\nb: gone".into()));
    }

    /// parity: OPS-031, OPS-034
    #[test]
    fn duplicate_restore_and_redo_have_toasts_in_the_same_wording() {
        let two_done = result(2, 0, &[], false);

        let duplicated = summarize_duplicate(&two_done);
        let restored = summarize_restore(&two_done);
        let redone = summarize_journal_step(JournalDirection::Redo, "New folder", &two_done);

        assert_eq!(
            duplicated,
            OperationSummary::Toast("2 item(s) duplicated.".into())
        );
        assert_eq!(restored, OperationSummary::Toast("2 item(s) restored.".into()));
        assert_eq!(redone, OperationSummary::Toast("New folder redone.".into()));
    }
}
