// SPDX-License-Identifier: AGPL-3.0-only
//! Paste (Ctrl+V), and copies and moves into a folder with the
//! name-conflict check (CLIP-003, OPS-026, OPS-027, OPS-028, OPS-036).
//!
//! Ports `paste` and `transferWithConflicts` of `desktop/ui/app.js`.
//! Paste reads the desktop's clipboard again first. Before anything is
//! copied, the destination is checked for every incoming name; without a
//! conflict the copy starts at once with the Skip policy, so a name that
//! appears after the check is still never overwritten. With conflicts the
//! dialog asks ([`super::conflict_dialog`]); items without a conflict
//! keep the Skip policy whatever the answers. The check and the dialog
//! count as the running operation, so a second paste cannot start
//! meanwhile. After a move, the moved items leave a cut clipboard.

use std::collections::HashMap;

use gtk::subclass::prelude::*;
use ox_core::clipboard::ClipboardMode;
use ox_core::ops::{
    find_conflicts, run_chosen_transfer, starting_label, ChosenTransfer, ItemChoice, OpsError,
    TransferOutcome, TransferRequest,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use super::conflict_dialog::ConflictAnswer;
use super::running::FinishedOperation;
use super::unfinished::mark_unfinished;
use crate::search::changed_folders;
use crate::window::dialog;
use crate::window::BrowserWindow;

/// The title of the dialog shown when the destination cannot be checked.
const CHECK_FAILED_TITLE: &str = "Could not check destination";

/// A copy or move of some items into one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncomingItems {
    /// Copy or move.
    pub(crate) mode: TransferMode,
    /// The items, in the order they were selected.
    pub(crate) uris: Vec<String>,
    /// The folder they go into.
    pub(crate) destination_folder: String,
}

/// How the items of `incoming` run, given the answers about their
/// conflicts: one request when every item has the same policy, otherwise
/// a request with an answer per item (items without a conflict: Skip).
#[derive(Debug, Clone, PartialEq, Eq)]
enum TransferPlan {
    /// Every item with one policy.
    Uniform(TransferRequest),
    /// An answer per item.
    PerItem(ChosenTransfer),
}

impl TransferPlan {
    /// The plan for `incoming` with `answers`.
    fn new(incoming: IncomingItems, answers: &[ConflictAnswer]) -> Self {
        let by_uri: HashMap<&str, &ConflictAnswer> = answers
            .iter()
            .map(|answer| (answer.uri.as_str(), answer))
            .collect();
        let policy_of = |uri: &String| {
            by_uri
                .get(uri.as_str())
                .map_or(ConflictPolicy::Skip, |answer| answer.policy)
        };
        let rename_of = |uri: &String| {
            by_uri
                .get(uri.as_str())
                .and_then(|answer| answer.rename_to.clone())
        };
        let first_policy = incoming.uris.first().map_or(ConflictPolicy::Skip, policy_of);
        let is_uniform = incoming
            .uris
            .iter()
            .all(|uri| policy_of(uri) == first_policy && rename_of(uri).is_none());
        if is_uniform {
            return TransferPlan::Uniform(TransferRequest {
                mode: incoming.mode,
                uris: incoming.uris,
                destination_folder: Some(incoming.destination_folder),
                policy: first_policy,
            });
        }
        let items = incoming
            .uris
            .iter()
            .map(|uri| ItemChoice {
                uri: uri.clone(),
                policy: policy_of(uri),
                rename_to: rename_of(uri),
            })
            .collect();
        TransferPlan::PerItem(ChosenTransfer {
            mode: incoming.mode,
            destination_folder: incoming.destination_folder,
            items,
        })
    }
}

impl BrowserWindow {
    /// Paste: reads the clipboard, then copies or moves its items into
    /// the folder shown (`paste`).
    pub(crate) async fn paste(&self) {
        let clipboard = self.refresh_file_clipboard().await;
        if self.is_searching() {
            self.show_message("Open the destination folder before pasting.");
            return;
        }
        let Some(files) = clipboard else {
            return;
        };
        let Some(destination_folder) = self.current_uri() else {
            return;
        };
        if !self
            .imp()
            .locations
            .borrow()
            .is_writable_location(&destination_folder)
        {
            return;
        }
        let mode = match files.mode() {
            ClipboardMode::Copy => TransferMode::Copy,
            ClipboardMode::Cut => TransferMode::Move,
        };
        let incoming = IncomingItems {
            mode,
            uris: files.uris().to_vec(),
            destination_folder,
        };
        let Some(outcome) = self.transfer_with_conflicts(incoming).await else {
            return;
        };
        if mode == TransferMode::Move && !outcome.result.done.is_empty() {
            self.consume_cut(&files, &outcome.result.done).await;
        }
    }

    /// Checks `incoming` for name conflicts, asks about them, then runs the
    /// copy or move and reports it (`transferWithConflicts`). Returns the
    /// outcome of a run that finished; `None` when another operation runs,
    /// the check failed or the user cancelled.
    pub(crate) async fn transfer_with_conflicts(&self, incoming: IncomingItems) -> Option<TransferOutcome> {
        let answers = self.plan_transfer(&incoming).await?;
        let mode = incoming.mode;
        let plan = TransferPlan::new(incoming, &answers);
        let outcome = self.run_plan(&plan).await?;
        let finished = outcome
            .clone()
            .map(|outcome| FinishedOperation::of_transfer(mode, outcome));
        self.conclude_operation(finished).await;
        outcome.ok()
    }

    /// The conflict check and the dialog, as the window's planning step:
    /// the answers about the conflicting items, or `None` when nothing
    /// may run.
    async fn plan_transfer(&self, incoming: &IncomingItems) -> Option<Vec<ConflictAnswer>> {
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            if operations.is_busy() {
                return None;
            }
            operations.planning = true;
        }
        self.update_file_commands();
        let answers = self.check_and_ask(incoming).await;
        self.imp().file_operations.borrow_mut().planning = false;
        self.update_file_commands();
        answers
    }

    /// Finds the conflicts of `incoming` and asks about them; a failed
    /// check opens "Could not check destination".
    async fn check_and_ask(&self, incoming: &IncomingItems) -> Option<Vec<ConflictAnswer>> {
        let checked =
            find_conflicts(&incoming.uris, &incoming.destination_folder, &Cancellation::new()).await;
        let conflicts = match checked {
            Ok(conflicts) => conflicts,
            Err(error) => {
                dialog::show_message(self, CHECK_FAILED_TITLE, &error.to_string()).await;
                return None;
            }
        };
        if conflicts.is_empty() {
            return Some(Vec::new());
        }
        let destination = self
            .imp()
            .locations
            .borrow()
            .display_location(&incoming.destination_folder);
        self.ask_about_conflicts(&conflicts, &incoming.destination_folder, &destination)
            .await
    }

    /// Runs `plan` as the window's one operation; `None` when another runs.
    async fn run_plan(&self, plan: &TransferPlan) -> Option<Result<TransferOutcome, OpsError>> {
        match plan {
            TransferPlan::Uniform(request) => self.run_request(request).await,
            TransferPlan::PerItem(chosen) => {
                let context = self.begin_operation(starting_label(chosen.mode))?;
                let progress = self.progress_reporter(&context.cancel);
                let mark = mark_unfinished(Some(&chosen.destination_folder));
                let outcome = run_chosen_transfer(chosen, &context, progress).await;
                drop(mark);
                self.end_operation();
                let items = chosen.items.iter().map(|item| item.uri.as_str());
                let changed = changed_folders([chosen.destination_folder.as_str()], items);
                self.context().search_cache().folders_written(changed);
                Some(outcome)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(uris: &[&str]) -> IncomingItems {
        IncomingItems {
            mode: TransferMode::Copy,
            uris: uris.iter().map(ToString::to_string).collect(),
            destination_folder: "file:///tmp/target".to_owned(),
        }
    }

    fn answer(uri: &str, policy: ConflictPolicy) -> ConflictAnswer {
        ConflictAnswer {
            uri: uri.to_owned(),
            policy,
            rename_to: None,
        }
    }

    /// parity: OPS-027
    #[test]
    fn without_conflicts_everything_copies_with_skip() {
        let plan = TransferPlan::new(incoming(&["file:///a", "file:///b"]), &[]);

        let TransferPlan::Uniform(request) = plan else {
            panic!("one policy for every item");
        };
        assert_eq!(request.policy, ConflictPolicy::Skip);
        assert_eq!(request.uris, ["file:///a", "file:///b"]);
        assert_eq!(request.destination_folder.as_deref(), Some("file:///tmp/target"));
    }

    /// parity: OPS-026
    #[test]
    fn one_answer_for_every_item_is_one_request_with_that_policy() {
        let answers = [answer("file:///a", ConflictPolicy::Replace)];

        let plan = TransferPlan::new(incoming(&["file:///a"]), &answers);

        let TransferPlan::Uniform(request) = plan else {
            panic!("one policy for every item");
        };
        assert_eq!(request.policy, ConflictPolicy::Replace);
    }

    /// parity: OPS-027, OPS-028
    #[test]
    fn items_without_a_conflict_keep_skip_beside_answered_ones() {
        let answers = [answer("file:///a", ConflictPolicy::KeepBoth)];

        let plan = TransferPlan::new(incoming(&["file:///a", "file:///b"]), &answers);

        let TransferPlan::PerItem(chosen) = plan else {
            panic!("an answer per item");
        };
        let policies: Vec<ConflictPolicy> = chosen.items.iter().map(|item| item.policy).collect();
        assert_eq!(policies, [ConflictPolicy::KeepBoth, ConflictPolicy::Skip]);
    }
}
