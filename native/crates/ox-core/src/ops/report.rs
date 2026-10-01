// SPDX-License-Identifier: AGPL-3.0-only
//! What the user is told when an operation ends (OPS-023).
//!
//! Ports the end of `runOperation` in `desktop/ui/app.js`: complete success
//! is a short toast; errors, skipped items or a cancellation open the
//! "Operation result" dialog listing each; a request refused before it
//! started opens "Operation stopped" with the refusal.

use super::journal::JournalDirection;
use super::undo::UndoRecord;
use crate::transfer::{TransferMode, TransferResult};

/// The title of the dialog that lists what an operation did.
pub const RESULT_TITLE: &str = "Operation result";

/// The title of the dialog for a request that failed before any item was
/// handled; its text is the error's message.
pub const STOPPED_TITLE: &str = "Operation stopped";

/// How the end of an operation is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationSummary {
    /// Every item succeeded: a toast such as `3 item(s) copied.`
    Toast(String),
    /// Something went wrong, was skipped or was cancelled: the
    /// [`RESULT_TITLE`] dialog with one line per fact.
    Report(String),
}

/// The summary of a finished `mode` run, word for word as `app.js`
/// writes it.
pub fn summarize(mode: TransferMode, result: &TransferResult) -> OperationSummary {
    let verb = match mode {
        TransferMode::Copy => "copied",
        TransferMode::Move => "moved",
        TransferMode::Delete => "permanently deleted",
        TransferMode::Trash => "sent to Trash",
    };
    summarize_items(verb, result)
}

/// The summary of Duplicate, which the Python app did not have:
/// `2 item(s) duplicated.` in the wording of the other toasts.
pub fn summarize_duplicate(result: &TransferResult) -> OperationSummary {
    summarize_items("duplicated", result)
}

/// The summary of Restore from the Recycle Bin: `2 item(s) restored.`
pub fn summarize_restore(result: &TransferResult) -> OperationSummary {
    summarize_items("restored", result)
}

/// The summary of Create links, which the Python app did not have:
/// `2 item(s) linked.` in the wording of the other toasts.
pub fn summarize_links(result: &TransferResult) -> OperationSummary {
    summarize_items("linked", result)
}

/// The summary of a batch rename (OPS-014): `3 item(s) renamed.`
pub fn summarize_batch_rename(result: &TransferResult) -> OperationSummary {
    summarize_items("renamed", result)
}

/// The summary of an Undo: `Rename undone.` when every step succeeded,
/// otherwise the [`RESULT_TITLE`] report of what happened.
pub fn summarize_undo(record: &UndoRecord, result: &TransferResult) -> OperationSummary {
    summarize_journal_step(JournalDirection::Undo, record.title(), result)
}

/// The summary of an Undo or Redo of the operation titled `title`:
/// `Rename undone.` or `Rename redone.` when every step succeeded,
/// otherwise the [`RESULT_TITLE`] report of what happened.
pub fn summarize_journal_step(
    direction: JournalDirection,
    title: &str,
    result: &TransferResult,
) -> OperationSummary {
    if !is_complete_success(result) {
        return OperationSummary::Report(report_lines(result));
    }
    let verb = match direction {
        JournalDirection::Undo => "undone",
        JournalDirection::Redo => "redone",
    };
    OperationSummary::Toast(format!("{title} {verb}."))
}

/// `N item(s) <verb>.` when every item succeeded, otherwise the report.
fn summarize_items(verb: &str, result: &TransferResult) -> OperationSummary {
    if is_complete_success(result) {
        return OperationSummary::Toast(format!("{} item(s) {verb}.", result.done.len()));
    }
    OperationSummary::Report(report_lines(result))
}

/// True when nothing failed, nothing was skipped and nobody cancelled.
fn is_complete_success(result: &TransferResult) -> bool {
    result.errors.is_empty() && result.skipped.is_empty() && !result.cancelled
}

/// `N completed.`, then the skipped count, the cancellation and each
/// error, one per line.
fn report_lines(result: &TransferResult) -> String {
    let mut lines = vec![format!("{} completed.", result.done.len())];
    if !result.skipped.is_empty() {
        lines.push(format!("{} skipped (name already exists).", result.skipped.len()));
    }
    if result.cancelled {
        lines.push(String::from("Cancelled. Completed items remain in place."));
    }
    lines.extend(result.errors.iter().cloned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(done: usize, skipped: usize, errors: &[&str], cancelled: bool) -> TransferResult {
        TransferResult {
            done: vec![String::from("file:///tmp/done"); done],
            skipped: vec![String::from("file:///tmp/skipped"); skipped],
            errors: errors.iter().map(ToString::to_string).collect(),
            cancelled,
            landed: Vec::new(),
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
    fn skips_cancellation_and_errors_are_listed_line_by_line() {
        let outcome = result(1, 2, &["b.txt: Permission denied"], true);

        let summary = summarize(TransferMode::Copy, &outcome);

        let expected = "1 completed.\n2 skipped (name already exists).\n\
                        Cancelled. Completed items remain in place.\nb.txt: Permission denied";
        assert_eq!(summary, OperationSummary::Report(expected.into()));
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
