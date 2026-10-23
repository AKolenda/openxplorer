// SPDX-License-Identifier: AGPL-3.0-only
//! Paste (Ctrl+V), and copies and moves into a folder with the
//! name-conflict check (CLIP-003, OPS-026, OPS-027, OPS-028, OPS-036).
//!
//! Ports `paste` and `transferWithConflicts` of `v2.0.0:desktop/ui/app.js`.
//! Paste reads the desktop's clipboard again first. Before anything is
//! copied, the destination is checked for every incoming name; without a
//! conflict the copy starts at once with the Skip policy, so a name that
//! appears after the check is still never overwritten. With conflicts the
//! dialog asks ([`super::conflict_dialog`]); items without a conflict
//! keep the Skip policy whatever the answers. A copy of an item into its
//! own folder is kept as "name - Copy" without asking, as Windows Explorer
//! does: the only name it meets there is its own. An item moved (cut and
//! pasted) into its own folder stays where it is, without asking, as in
//! Windows Explorer; the other items move. The check and the dialog
//! count as the running operation, so a second paste cannot start
//! meanwhile. After a move, the moved items leave a cut clipboard.

use std::collections::HashMap;

use gtk::subclass::prelude::*;
use ox_core::clipboard::{ClipboardFiles, ClipboardMode};
use ox_core::location::{parent_location, same_location};
use ox_core::ops::{
    find_conflicts, run_chosen_transfer, starting_label, ChosenTransfer, ItemChoice, OpsError,
    TransferOutcome, TransferRequest,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use super::conflict_dialog::ConflictAnswer;
use super::running::FinishedOperation;
use super::unfinished::mark_unfinished;
use crate::dialog;
use crate::search::changed_folders;
use crate::window::BrowserWindow;

/// The title of the dialog shown when the destination cannot be checked.
const CHECK_FAILED_TITLE: &str = crate::i18n::message_id("Could not check destination");

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

impl IncomingItems {
    /// The items that a move would really move: an item moved into the
    /// folder it is already in stays where it is, without asking, as in
    /// Windows Explorer. `None` when nothing is left to move.
    fn without_items_already_there(mut self) -> Option<Self> {
        if self.mode == TransferMode::Move {
            let destination = self.destination_folder.clone();
            self.uris.retain(|uri| {
                !parent_location(uri).is_some_and(|parent| same_location(&parent, &destination))
            });
        }
        (!self.uris.is_empty()).then_some(self)
    }
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
    /// `into`, a selected folder (Dolphin's "Paste into folder", CMD-019),
    /// or without one into the folder shown (`paste`).
    pub(crate) async fn paste(&self, into: Option<String>) {
        let clipboard = self.refresh_file_clipboard().await;
        if into.is_none() && self.is_searching() {
            self.show_message(ox_core::i18n::gettext_static(
                "Open the destination folder before pasting.",
            ));
            return;
        }
        // Ctrl+V pastes without the Paste button, which a folder that
        // could not be read disables: it takes nothing either.
        if into.is_none() && self.shown_folder_is_unreadable() {
            self.show_message(ox_core::i18n::gettext_static("This location is unavailable"));
            return;
        }
        let Some(destination_folder) = into.or_else(|| self.current_uri()) else {
            return;
        };
        self.paste_files(clipboard, destination_folder).await;
    }

    /// Paste into the folder at `uri`, from the folder tree's menu
    /// (SIDE-028), as Dolphin's "Paste" on a folder pastes into it.
    pub(crate) async fn paste_into(&self, uri: &str) {
        let clipboard = self.refresh_file_clipboard().await;
        self.paste_files(clipboard, uri.to_owned()).await;
    }

    /// Copies or moves the clipboard's `files` into `destination_folder`
    /// where it is writable.
    async fn paste_files(&self, clipboard: Option<ClipboardFiles>, destination_folder: String) {
        let Some(files) = clipboard else {
            return;
        };
        if !self
            .imp()
            .locations
            .borrow()
            .is_writable_location(&destination_folder)
        {
            self.show_message(ox_core::i18n::gettext_static("This folder is read-only."));
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
        self.transfer_checked(incoming, true).await
    }

    /// [`Self::transfer_with_conflicts`] for a move that is one step of a
    /// larger operation, which Undo cannot reverse on its own: the items
    /// of an extraction into an existing folder come from a private folder
    /// that is removed afterwards, so the journal does not record it.
    pub(crate) async fn transfer_without_undo(&self, incoming: IncomingItems) -> Option<TransferOutcome> {
        self.transfer_checked(incoming, false).await
    }

    /// Checks, asks, runs and reports `incoming`; the journal records it
    /// when `undoable`.
    async fn transfer_checked(&self, incoming: IncomingItems, undoable: bool) -> Option<TransferOutcome> {
        let incoming = incoming.without_items_already_there()?;
        let origin = self.current_uri();
        let answers = self.plan_transfer(&incoming).await?;
        let mode = incoming.mode;
        let plan = TransferPlan::new(incoming, &answers);
        let outcome = self.run_plan(&plan).await?;
        let finished = outcome.clone().map(|outcome| {
            let finished = FinishedOperation::of_transfer(mode, outcome);
            if undoable {
                finished
            } else {
                FinishedOperation {
                    undo: None,
                    ..finished
                }
            }
        });
        self.conclude_operation_in(finished, origin.as_deref()).await;
        outcome.ok()
    }

    /// The conflict check and the dialog, as the window's planning step:
    /// the answers about the conflicting items, or `None` when nothing
    /// may run.
    async fn plan_transfer(&self, incoming: &IncomingItems) -> Option<Vec<ConflictAnswer>> {
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            if operations.is_busy() || operations.jobs.len() >= super::jobs::MAX_JOBS {
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
                dialog::show_message(
                    self,
                    ox_core::i18n::gettext_static(CHECK_FAILED_TITLE),
                    &error.to_string(),
                )
                .await;
                return None;
            }
        };
        // An item copied into its own folder meets only its own name:
        // Keep both, without asking. The others are asked about.
        let (own, others): (Vec<String>, Vec<String>) = conflicts.into_iter().partition(|uri| {
            incoming.mode == TransferMode::Copy
                && parent_location(uri)
                    .is_some_and(|parent| same_location(&parent, &incoming.destination_folder))
        });
        // The names those copies will take are not offered to the others.
        let mut reserved = Vec::new();
        for uri in &own {
            let planned =
                super::conflict_rename::suggested_name(uri, &incoming.destination_folder, &reserved).await;
            reserved.push(planned);
        }
        let mut answers: Vec<ConflictAnswer> = own
            .into_iter()
            .map(|uri| ConflictAnswer {
                uri,
                policy: ConflictPolicy::KeepBoth,
                rename_to: None,
            })
            .collect();
        if others.is_empty() {
            return Some(answers);
        }
        let destination = self
            .imp()
            .locations
            .borrow()
            .display_location(&incoming.destination_folder);
        let asked = self
            .ask_about_conflicts(&others, &incoming.destination_folder, &destination, reserved)
            .await?;
        answers.extend(asked);
        Some(answers)
    }

    /// Runs `plan` as a transfer job with independent progress and cancellation.
    /// Returns `None` when `begin_transfer` cannot admit the job.
    async fn run_plan(&self, plan: &TransferPlan) -> Option<Result<TransferOutcome, OpsError>> {
        match plan {
            TransferPlan::Uniform(request) => self.run_request(request).await,
            TransferPlan::PerItem(chosen) => {
                let uris: Vec<String> = chosen.items.iter().map(|item| item.uri.clone()).collect();
                let context = self.begin_transfer(
                    starting_label(chosen.mode),
                    &uris,
                    Some(&chosen.destination_folder),
                )?;
                let progress = self.progress_reporter(&context.cancel);
                let mark = mark_unfinished(Some(&chosen.destination_folder));
                let outcome = run_chosen_transfer(chosen, &context, progress).await;
                drop(mark);
                self.end_transfer(&context.cancel);
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

    /// A move leaves out the items already in the destination folder; a
    /// copy keeps them, to make "name - Copy".
    ///
    /// parity: OPS-026
    #[test]
    fn a_move_leaves_out_the_items_already_in_the_folder() {
        let items = |mode| IncomingItems {
            mode,
            uris: vec![
                "file:///tmp/target/a.txt".to_owned(),
                "file:///tmp/other/b.txt".to_owned(),
            ],
            destination_folder: "file:///tmp/target/".to_owned(),
        };
        let moved = items(TransferMode::Move).without_items_already_there();
        assert_eq!(
            moved.map(|moved| moved.uris),
            Some(vec!["file:///tmp/other/b.txt".to_owned()])
        );
        assert_eq!(
            items(TransferMode::Copy).without_items_already_there(),
            Some(items(TransferMode::Copy))
        );
        let only_there = IncomingItems {
            uris: vec!["file:///tmp/target/a.txt".to_owned()],
            ..items(TransferMode::Move)
        };
        assert_eq!(only_there.without_items_already_there(), None);
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
