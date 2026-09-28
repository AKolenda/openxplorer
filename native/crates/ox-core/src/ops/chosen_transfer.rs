// SPDX-License-Identifier: AGPL-3.0-only
//! A copy or move whose name conflicts the user answered one by one
//! (OPS-026, OPS-028).
//!
//! The Python app asked once per paste, "Skip duplicates" or "Replace
//! existing", and ran the whole batch with that policy
//! (`transferWithConflicts` in `desktop/ui/app.js`). The native conflict
//! dialog also offers Keep both and, when "Apply to all" is cleared, an
//! answer per item. The engine takes one policy per run, so the items are
//! grouped by their answer and each group runs on one engine, Skip first,
//! then Keep both, then Replace. Items without a conflict run with Skip,
//! so a name that appears after the check is never overwritten (OPS-027).
//! Every group keeps every safety rule of the engine; a cancellation stops
//! the groups that have not started.

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::results::merge_results;
use super::run_transfer::{gio_transfer_engine, run_on_engine, TransferOutcome, TransferRequest};
use super::undo::UndoRecord;
use crate::transfer::{ConflictPolicy, Progress, TransferMode};

/// One item of the paste and what happens when its name is taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemChoice {
    /// The item's URI.
    pub uri: String,
    /// The user's answer for this item; Skip for an item without a
    /// conflict.
    pub policy: ConflictPolicy,
}

/// A copy or move with an answer per item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenTransfer {
    /// Copy or move.
    pub mode: TransferMode,
    /// The folder the items go into.
    pub destination_folder: String,
    /// The items in the order they were selected, each with its answer.
    pub items: Vec<ItemChoice>,
}

/// The order the groups run in: the safe answers first.
const POLICY_ORDER: [ConflictPolicy; 3] = [
    ConflictPolicy::Skip,
    ConflictPolicy::KeepBoth,
    ConflictPolicy::Replace,
];

/// Runs `request`, one engine run per answer, sending throttled progress
/// to `progress` on the worker thread. The outcome adds up the runs, and
/// its Undo reverses the whole paste as one step, or nothing when a
/// replaced item makes part of it irreversible.
///
/// # Errors
///
/// A group refused before anything changed, as [`run_transfer`] refuses
/// it; the groups that already ran stay done.
///
/// [`run_transfer`]: super::run_transfer
pub async fn run_chosen_transfer(
    request: &ChosenTransfer,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || run_chosen_transfer_blocking(&request, &context, progress)).await
}

/// [`run_chosen_transfer`] on the calling thread.
fn run_chosen_transfer_blocking(
    request: &ChosenTransfer,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    let mut engine = gio_transfer_engine(&context.protection, progress);
    let mut total = TransferOutcome::default();
    let mut undo = CombinedUndo::Nothing;
    for group in groups_by_policy(request) {
        let part = run_on_engine(&mut engine, &group, context)?;
        undo = undo.with(&part);
        merge_results(&mut total.result, part.result);
        total.created.extend(part.created);
        if total.result.cancelled {
            break;
        }
    }
    total.undo = undo.into_record();
    Ok(total)
}

/// One request per answer that some item has, in [`POLICY_ORDER`].
fn groups_by_policy(request: &ChosenTransfer) -> Vec<TransferRequest> {
    POLICY_ORDER
        .into_iter()
        .map(|policy| group_with_policy(request, policy))
        .filter(|group| !group.uris.is_empty())
        .collect()
}

/// The request for the items of `request` answered with `policy`.
fn group_with_policy(request: &ChosenTransfer, policy: ConflictPolicy) -> TransferRequest {
    let uris = request
        .items
        .iter()
        .filter(|item| item.policy == policy)
        .map(|item| item.uri.clone())
        .collect();
    TransferRequest {
        mode: request.mode,
        uris,
        destination_folder: Some(request.destination_folder.clone()),
        policy,
    }
}

/// How the groups that finished items can be undone together.
#[derive(Debug)]
enum CombinedUndo {
    /// No group finished an item yet.
    Nothing,
    /// Every group that finished items can be undone; this record undoes
    /// them all.
    Undoable(UndoRecord),
    /// A group finished items that cannot be undone (Replace), so the
    /// paste is not undone at all rather than half.
    NotUndoable,
}

impl CombinedUndo {
    /// The combination after a group that ended with `part`.
    fn with(self, part: &TransferOutcome) -> Self {
        if part.result.done.is_empty() {
            return self;
        }
        let Some(record) = part.undo.clone() else {
            return CombinedUndo::NotUndoable;
        };
        match self {
            CombinedUndo::Nothing => CombinedUndo::Undoable(record),
            CombinedUndo::Undoable(total) => joined(total, record),
            CombinedUndo::NotUndoable => CombinedUndo::NotUndoable,
        }
    }

    fn into_record(self) -> Option<UndoRecord> {
        match self {
            CombinedUndo::Undoable(record) => Some(record),
            CombinedUndo::Nothing | CombinedUndo::NotUndoable => None,
        }
    }
}

/// `total` and `part`, two records of the same copy or move, as one.
fn joined(total: UndoRecord, part: UndoRecord) -> CombinedUndo {
    match (total, part) {
        (UndoRecord::Copy { mut copies }, UndoRecord::Copy { copies: more }) => {
            copies.extend(more);
            CombinedUndo::Undoable(UndoRecord::Copy { copies })
        }
        (UndoRecord::Move { mut items }, UndoRecord::Move { items: more }) => {
            items.extend(more);
            CombinedUndo::Undoable(UndoRecord::Move { items })
        }
        // One paste has one mode, so its groups never mix records.
        _ => CombinedUndo::NotUndoable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::TransferResult;

    fn choice(uri: &str, policy: ConflictPolicy) -> ItemChoice {
        ItemChoice {
            uri: uri.to_owned(),
            policy,
        }
    }

    fn copied(copies: &[&str]) -> TransferOutcome {
        TransferOutcome {
            result: TransferResult {
                done: copies.iter().map(ToString::to_string).collect(),
                ..TransferResult::default()
            },
            created: Vec::new(),
            undo: Some(UndoRecord::Copy {
                copies: copies.iter().map(ToString::to_string).collect(),
            }),
        }
    }

    #[test]
    fn items_run_grouped_by_answer_with_the_safe_answers_first() {
        let request = ChosenTransfer {
            mode: TransferMode::Copy,
            destination_folder: "file:///dst".into(),
            items: vec![
                choice("file:///a", ConflictPolicy::Replace),
                choice("file:///b", ConflictPolicy::Skip),
                choice("file:///c", ConflictPolicy::KeepBoth),
                choice("file:///d", ConflictPolicy::Skip),
            ],
        };

        let groups = groups_by_policy(&request);

        let order: Vec<(ConflictPolicy, Vec<String>)> = groups
            .into_iter()
            .map(|group| (group.policy, group.uris))
            .collect();
        assert_eq!(
            order,
            [
                (ConflictPolicy::Skip, vec!["file:///b".into(), "file:///d".into()]),
                (ConflictPolicy::KeepBoth, vec!["file:///c".into()]),
                (ConflictPolicy::Replace, vec!["file:///a".into()]),
            ]
        );
    }

    #[test]
    fn a_paste_is_undone_as_one_step_or_not_at_all() {
        let replaced = TransferOutcome {
            undo: None,
            ..copied(&["file:///dst/c"])
        };

        let both = CombinedUndo::Nothing
            .with(&copied(&["file:///dst/a"]))
            .with(&copied(&["file:///dst/b"]));
        let with_replace = CombinedUndo::Nothing
            .with(&copied(&["file:///dst/a"]))
            .with(&replaced);
        let nothing_finished = CombinedUndo::Nothing.with(&TransferOutcome::default());

        let expected = UndoRecord::Copy {
            copies: vec!["file:///dst/a".into(), "file:///dst/b".into()],
        };
        assert_eq!(both.into_record(), Some(expected));
        assert_eq!(with_replace.into_record(), None);
        assert_eq!(nothing_finished.into_record(), None);
    }
}
