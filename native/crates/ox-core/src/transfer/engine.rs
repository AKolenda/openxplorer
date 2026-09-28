// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer orchestration. Port of `TransferEngine.run` and
//! `_run_items` in `desktop/operations.py`; see the module documentation for
//! the rules. The request is validated in `request.rs`, the settings of the
//! run are in `batch.rs`, the destination name is chosen in `conflicts.rs`,
//! and the copy of one item (staging, publishing, device checks) is in
//! `staged_copy.rs`.

use std::fmt;
use std::time::Duration;

use super::batch::{Batch, ItemAction, Removal};
use super::cancellation::Cancellation;
use super::commit::commit_replace;
use super::conflicts::Placement;
use super::containment::guard_destination;
use super::error::TransferError;
use super::guard::{check_write_tree, SourceChange};
use super::labels::{completed_label, item_label};
use super::node::{Node, NodeFactory, NodeKind, WriteGuard};
use super::relisting::SourceFolders;
use super::request::{destination_folder, distinct_items};
use super::staged_copy::{ItemStaging, StagedCopy};
use super::staging::{discard_stage, leftover_report};
use super::types::{ConflictPolicy, Operation, Progress, TransferMode, TransferResult};

/// Receives the progress of a run for the transfer panel.
type ProgressCallback = Box<dyn FnMut(Progress) + Send>;

/// Waits between device cleanup attempts; tests record the delay instead.
type SleepCallback = Box<dyn Fn(Duration) + Send + Sync>;

/// Runs copy, move, Trash and delete operations over [`Node`]s.
pub struct TransferEngine {
    factory: NodeFactory,
    /// Named after `self.emit` of the Python engine.
    emit: ProgressCallback,
    write_guard: Option<Box<WriteGuard>>,
    sleep: SleepCallback,
}

impl fmt::Debug for TransferEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferEngine")
            .field("has_write_guard", &self.write_guard.is_some())
            .finish_non_exhaustive()
    }
}

/// What one run has done so far.
#[derive(Default)]
struct RunState {
    result: TransferResult,
    /// Folders that moves took items from, relisted at the end (MTP).
    moved_from: SourceFolders,
}

/// How an item that did not fail ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemOutcome {
    /// Copied, moved, trashed or deleted.
    Done,
    /// Left alone: its name is taken and the policy is Skip, or it was
    /// moved into the folder it is already in.
    Skipped,
}

/// A selected item, resolved and inspected without following links.
struct SelectedItem {
    node: Box<dyn Node>,
    kind: NodeKind,
}

impl TransferEngine {
    /// An engine resolving URIs with `factory`, without progress reports or
    /// a write guard.
    pub fn new(factory: NodeFactory) -> Self {
        Self {
            factory,
            emit: Box::new(|_| {}),
            write_guard: None,
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Receives progress for the transfer panel.
    #[must_use]
    pub fn with_progress(mut self, emit: impl FnMut(Progress) + Send + 'static) -> Self {
        self.emit = Box::new(emit);
        self
    }

    /// Rejects writes into protected locations such as snapshot folders.
    #[must_use]
    pub fn with_write_guard(
        mut self,
        guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.write_guard = Some(Box::new(guard));
        self
    }

    /// Replaces the delay used between device cleanup retries (tests).
    #[must_use]
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// Runs `operation` over `uris`. A caller holding the protocol names of
    /// a request builds the operation with [`Operation::from_request`].
    ///
    /// Everything after the request was accepted is reported per item in
    /// the result.
    ///
    /// # Errors
    ///
    /// Invalid requests, before anything is changed: no items or too many,
    /// an empty destination, a destination that is not a folder, or a
    /// failure or cancellation while the destination is checked.
    pub fn run(
        &mut self,
        operation: Operation<'_>,
        uris: &[String],
        cancel: &Cancellation,
    ) -> Result<TransferResult, TransferError> {
        let uris = distinct_items(uris)?;
        let result = match operation {
            Operation::Copy {
                destination_folder: folder_uri,
                policy,
            }
            | Operation::Move {
                destination_folder: folder_uri,
                policy,
            } => {
                let folder = destination_folder(&self.factory, folder_uri, cancel)?;
                let placement = Placement {
                    mode: operation.mode(),
                    policy,
                    destination_folder: folder.as_ref(),
                    cancel,
                };
                self.run_items(ItemAction::Transfer(placement), &uris, cancel)
            }
            Operation::Trash => {
                let trash = ItemAction::Remove(Removal::Trash);
                self.run_items(trash, &uris, cancel)
            }
            Operation::Delete => {
                let permanent_delete = ItemAction::Remove(Removal::PermanentDelete);
                self.run_items(permanent_delete, &uris, cancel)
            }
        };
        Ok(result)
    }

    /// Runs `action` over the distinct `uris` of an accepted request, then
    /// relists the folders that moves took items from.
    fn run_items(&mut self, action: ItemAction<'_>, uris: &[&str], cancel: &Cancellation) -> TransferResult {
        let batch = Batch {
            action,
            cancel,
            total: uris.len(),
        };
        let mut state = RunState::default();
        for (index, uri) in uris.iter().enumerate() {
            self.run_item(&batch, index, uri, &mut state);
            if state.result.cancelled {
                break;
            }
        }
        // XFER-025: MTP keeps resolving a moved item's OLD path to the
        // object until that folder is listed again.
        state.moved_from.refresh_all();
        (self.emit)(Progress {
            label: completed_label(state.result.done.len()),
            fraction: 1.0,
        });
        state.result
    }

    /// Runs one top-level item and records its outcome. Staging this item
    /// created is removed afterwards, whatever happened; a leftover is
    /// reported with its exact location.
    fn run_item(&mut self, batch: &Batch, index: usize, uri: &str, state: &mut RunState) {
        let mut staging = ItemStaging::default();
        let outcome = self.process_item(batch, index, uri, &mut state.moved_from, &mut staging);
        match outcome {
            Ok(ItemOutcome::Skipped) => state.result.skipped.push(uri.to_owned()),
            Ok(ItemOutcome::Done) => {
                state.result.done.push(uri.to_owned());
                // A published copy leaves its private folder empty. Failing
                // to remove it is reported, but the copy stays done.
                if let Err(error) = staging.remove_empty_folder() {
                    self.record_failure(batch, uri, &error, &mut state.result);
                }
            }
            Err(error) => self.record_failure(batch, uri, &error, &mut state.result),
        }
        self.discard_leftover_stage(staging, &mut state.result);
    }

    /// Copies, moves, trashes or deletes one top-level item.
    fn process_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        moved_from: &mut SourceFolders,
        staging: &mut ItemStaging,
    ) -> Result<ItemOutcome, TransferError> {
        let selected = self.start_item(batch, index, uri)?;
        match &batch.action {
            ItemAction::Remove(removal) => {
                self.remove(*removal, selected.node.as_ref(), batch.cancel)?;
                Ok(ItemOutcome::Done)
            }
            ItemAction::Transfer(placement) => {
                self.transfer(batch, placement, &selected, moved_from, staging)
            }
        }
    }

    /// Resolves the selected item at `uri` and announces it on the progress
    /// panel.
    fn start_item(&mut self, batch: &Batch, index: usize, uri: &str) -> Result<SelectedItem, TransferError> {
        batch.cancel.check()?;
        let node = (self.factory)(uri)?;
        // XFER-019: a root has no name to copy, move or trash it under.
        if node.parent().is_none() {
            return Err(TransferError::failed(
                "Filesystem roots cannot be copied, moved or trashed as items.",
            ));
        }
        let kind = node.info(Some(batch.cancel))?.kind;
        (self.emit)(Progress {
            label: item_label(batch.mode(), &node.display_name(), index + 1, batch.total),
            fraction: batch.start_fraction(index),
        });
        Ok(SelectedItem { node, kind })
    }

    /// Trashes or permanently deletes one user-selected item.
    fn remove(
        &self,
        removal: Removal,
        source: &dyn Node,
        cancel: &Cancellation,
    ) -> Result<(), TransferError> {
        // XFER-020: a protected descendant anywhere in the tree stops the
        // whole item before anything is removed.
        check_write_tree(self.guard(), source, None, cancel, SourceChange::Changed)?;
        match removal {
            // XFER-014: the backend reports an error rather than falling
            // back to a permanent delete.
            Removal::Trash => source.trash(cancel),
            // XFER-015: reached only after the user confirmed a permanent
            // delete. The backend re-checks every item with the write guard.
            Removal::PermanentDelete => source.delete_tree(cancel, self.guard()),
        }
    }

    /// Copies or moves one selected item into the destination folder of
    /// `placement`, under the name its conflict policy chooses.
    fn transfer(
        &mut self,
        batch: &Batch,
        placement: &Placement,
        selected: &SelectedItem,
        moved_from: &mut SourceFolders,
        staging: &mut ItemStaging,
    ) -> Result<ItemOutcome, TransferError> {
        let source = selected.node.as_ref();
        if selected.kind == NodeKind::Directory {
            guard_destination(source, placement.destination_folder)?;
        }
        let Some(destination) = placement.destination_for(source, selected.kind)? else {
            return Ok(ItemOutcome::Skipped);
        };
        let destination = destination.as_ref();
        // XFER-020: check every affected path before changing this
        // top-level item, because a writable parent can contain protected
        // backup descendants.
        let source_change = batch.source_change();
        check_write_tree(
            self.guard(),
            source,
            Some(destination),
            batch.cancel,
            source_change,
        )?;
        if placement.mode == TransferMode::Move {
            moved_from.remember(source);
            self.move_item(source, destination, placement.policy, batch.cancel)?;
        } else {
            self.copy_item(placement, selected, destination, staging)?;
        }
        Ok(ItemOutcome::Done)
    }

    /// Moves one item. XFER-011: backends never degrade a move to
    /// copy-then-delete (`NO_FALLBACK_FOR_MOVE`); Replace is an explicit
    /// user choice and still never degrades.
    fn move_item(
        &self,
        source: &dyn Node,
        destination: &dyn Node,
        policy: ConflictPolicy,
        cancel: &Cancellation,
    ) -> Result<(), TransferError> {
        if policy == ConflictPolicy::Replace {
            commit_replace(source, destination, cancel, self.guard(), None)
        } else {
            source.move_native(destination, Some(cancel))
        }
    }

    /// Copies one item into the destination folder of `placement` through
    /// private staging; `staging` receives the staging as soon as it
    /// exists.
    fn copy_item(
        &mut self,
        placement: &Placement,
        selected: &SelectedItem,
        destination: &dyn Node,
        staging: &mut ItemStaging,
    ) -> Result<(), TransferError> {
        let copy = StagedCopy {
            source: selected.node.as_ref(),
            source_kind: selected.kind,
            destination_folder: placement.destination_folder,
            destination,
            policy: placement.policy,
            cancel: placement.cancel,
            guard: self.write_guard.as_deref(),
            emit: &mut *self.emit,
        };
        copy.run(staging)
    }

    /// Adds a failed item to `result`, unless the user's cancellation
    /// explains it.
    fn record_failure(&self, batch: &Batch, uri: &str, error: &TransferError, result: &mut TransferResult) {
        // Only the user's own cancellation counts: a backend reporting
        // "cancelled" by itself is an ordinary error.
        let cancelled = error.is_cancelled() || batch.cancel.is_cancelled();
        result.cancelled |= cancelled;
        // XFER-003 and XFER-010: cancellation must not hide the location of
        // a retained backup or another problem that requires manual
        // recovery.
        if !cancelled || matches!(error, TransferError::RecoveryRequired(_)) {
            result.errors.push(format!("{}: {error}", self.display_name(uri)));
        }
    }

    /// Removes the staging an item left behind. XFER-003: a leftover is
    /// reported with its exact location.
    fn discard_leftover_stage(&self, staging: ItemStaging, result: &mut TransferResult) {
        let Some(stage) = staging.stage else {
            return;
        };
        let root = stage.root();
        if let Err(problem) = discard_stage(root, staging.created, staging.place, &*self.sleep) {
            result.errors.push(leftover_report(root, staging.place, &problem));
        }
    }

    /// The write guard, when the app installed one.
    fn guard(&self) -> Option<&WriteGuard> {
        self.write_guard.as_deref()
    }

    /// The item's name for error messages, or the URI when it cannot be
    /// resolved.
    fn display_name(&self, uri: &str) -> String {
        match (self.factory)(uri) {
            Ok(node) => node.display_name(),
            Err(_) => uri.to_string(),
        }
    }
}
