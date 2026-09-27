// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer orchestration. Port of `TransferEngine.run` and
//! `_run_items` in `desktop/operations.py`; see the module documentation for
//! the rules. The destination name is chosen in `conflicts.rs`, and the copy
//! of one item (staging, publishing, device checks) is in `staged_copy.rs`.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use super::commit::commit_replace;
use super::conflicts::Placement;
use super::error::TransferError;
use super::guard::{check_write_tree, guard_destination, SourceChange};
use super::labels::{completed_label, item_label};
use super::node::{Cancellation, Node, NodeFactory, NodeKind, WriteGuard};
use super::relisting::SourceFolders;
use super::staged_copy::{StageSlot, StagedCopy};
use super::staging::{discard_stage, leftover_report};
use super::types::{progress_fraction, ConflictPolicy, Progress, TransferMode, TransferResult};

/// The most items one run accepts.
pub const MAX_ITEMS: usize = 100_000;

type Emit = Box<dyn FnMut(Progress) + Send>;
type Sleep = Box<dyn Fn(Duration) + Send + Sync>;

/// Runs copy, move, Trash and delete operations over [`Node`]s.
pub struct TransferEngine {
    factory: NodeFactory,
    emit: Emit,
    write_guard: Option<Box<WriteGuard>>,
    sleep: Sleep,
}

impl fmt::Debug for TransferEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferEngine")
            .field("has_write_guard", &self.write_guard.is_some())
            .finish_non_exhaustive()
    }
}

/// The settings shared by every item of one run.
struct Batch<'a> {
    mode: TransferMode,
    policy: ConflictPolicy,
    /// The destination folder; `None` for Trash and delete.
    dest_dir: Option<&'a dyn Node>,
    cancel: &'a Cancellation,
    /// The number of distinct items.
    total: usize,
}

impl Batch<'_> {
    /// The progress shown when item `index` starts. Trash and delete have
    /// no byte progress, so the batch position is the only honest fraction
    /// to show for them.
    fn start_fraction(&self, index: usize) -> f64 {
        if self.mode.is_removal() {
            progress_fraction(index as u64, self.total as u64)
        } else {
            0.0
        }
    }

    /// How a copy or move into `dest_dir` names its items.
    fn placement<'a>(&'a self, dest_dir: &'a dyn Node) -> Placement<'a> {
        Placement {
            mode: self.mode,
            policy: self.policy,
            dest_dir,
            cancel: self.cancel,
        }
    }

    /// Move, Trash and delete change their sources; a copy keeps them.
    fn source_change(&self) -> SourceChange {
        match self.mode {
            TransferMode::Move | TransferMode::Trash | TransferMode::Delete => SourceChange::Changed,
            TransferMode::Copy => SourceChange::Kept,
        }
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

    /// Runs one operation over `uris`. `target` is the destination folder
    /// for copies and moves and is ignored for Trash and delete.
    ///
    /// Everything after the request was accepted is reported per item in
    /// the result.
    ///
    /// # Errors
    ///
    /// Invalid requests, before anything is changed: no items or too many,
    /// no destination, a destination that is not a folder, or a failure or
    /// cancellation while the destination is checked.
    pub fn run(
        &mut self,
        mode: TransferMode,
        uris: &[String],
        target: Option<&str>,
        policy: ConflictPolicy,
        cancel: &Cancellation,
    ) -> Result<TransferResult, TransferError> {
        if uris.is_empty() || uris.len() > MAX_ITEMS {
            return Err(TransferError::failed("Select between 1 and 100,000 items."));
        }
        let uris = deduplicate(uris);
        let dest_dir = self.destination_folder(mode, target, cancel)?;
        let batch = Batch {
            mode,
            policy,
            dest_dir: dest_dir.as_deref(),
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
        // MTP keeps resolving a moved item's OLD path to the object until
        // that folder is listed again. Relist once per source folder.
        state.moved_from.refresh_all();
        (self.emit)(Progress {
            label: completed_label(state.result.done.len()),
            fraction: 1.0,
        });
        Ok(state.result)
    }

    /// The destination folder of a copy or move; `None` for Trash and
    /// delete.
    fn destination_folder(
        &self,
        mode: TransferMode,
        target: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn Node>>, TransferError> {
        if mode.is_removal() {
            return Ok(None);
        }
        let Some(target) = target.filter(|target| !target.is_empty()) else {
            return Err(TransferError::failed("Choose a destination folder."));
        };
        let dest_dir = (self.factory)(target)?;
        if !dest_dir.is_directory(Some(cancel))? {
            return Err(TransferError::failed("The destination is not a folder."));
        }
        Ok(Some(dest_dir))
    }

    /// Runs one top-level item and records its outcome. Staging this item
    /// created is removed afterwards, whatever happened; a leftover is
    /// reported with its exact location.
    fn run_item(&mut self, batch: &Batch, index: usize, uri: &str, state: &mut RunState) {
        let mut slot = StageSlot::default();
        let outcome = self.process_item(batch, index, uri, &mut state.moved_from, &mut slot);
        match outcome {
            Ok(ItemOutcome::Skipped) => state.result.skipped.push(uri.to_owned()),
            Ok(ItemOutcome::Done) => {
                state.result.done.push(uri.to_owned());
                // A published copy leaves its private folder empty. Failing
                // to remove it is reported, but the copy stays done.
                if let Err(error) = slot.remove_empty_folder() {
                    self.record_failure(batch, uri, &error, &mut state.result);
                }
            }
            Err(error) => self.record_failure(batch, uri, &error, &mut state.result),
        }
        self.discard_leftover_stage(slot, &mut state.result);
    }

    /// Copies, moves, trashes or deletes one top-level item.
    fn process_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        moved_from: &mut SourceFolders,
        slot: &mut StageSlot,
    ) -> Result<ItemOutcome, TransferError> {
        let selected = self.start_item(batch, index, uri)?;
        if batch.mode.is_removal() {
            self.remove(batch.mode, selected.node.as_ref(), batch.cancel)?;
            return Ok(ItemOutcome::Done);
        }
        self.transfer(batch, &selected, moved_from, slot)
    }

    /// Copies or moves one selected item into the destination folder, under
    /// the name its conflict policy chooses.
    fn transfer(
        &mut self,
        batch: &Batch,
        selected: &SelectedItem,
        moved_from: &mut SourceFolders,
        slot: &mut StageSlot,
    ) -> Result<ItemOutcome, TransferError> {
        let source = selected.node.as_ref();
        let dest_dir = batch
            .dest_dir
            .ok_or_else(|| TransferError::failed("Choose a destination folder."))?;
        if selected.kind == NodeKind::Directory {
            guard_destination(source, dest_dir)?;
        }
        let placement = batch.placement(dest_dir);
        let Some(destination) = placement.destination_for(source, selected.kind)? else {
            return Ok(ItemOutcome::Skipped);
        };
        let destination = destination.as_ref();
        // Check every affected path before changing this top-level item: a
        // writable parent can contain protected backup descendants.
        let source_change = batch.source_change();
        check_write_tree(
            self.guard(),
            source,
            Some(destination),
            batch.cancel,
            source_change,
        )?;
        if batch.mode == TransferMode::Move {
            moved_from.remember(source);
            self.move_item(source, destination, batch.policy, batch.cancel)?;
        } else {
            self.copy_item(batch, selected, dest_dir, destination, slot)?;
        }
        Ok(ItemOutcome::Done)
    }

    /// Resolves the selected item at `uri` and announces it on the progress
    /// panel.
    fn start_item(&mut self, batch: &Batch, index: usize, uri: &str) -> Result<SelectedItem, TransferError> {
        batch.cancel.check()?;
        let node = (self.factory)(uri)?;
        if node.parent().is_none() {
            return Err(TransferError::failed(
                "Filesystem roots cannot be copied, moved or trashed as items.",
            ));
        }
        let kind = node.info(Some(batch.cancel))?.kind;
        (self.emit)(Progress {
            label: item_label(batch.mode, &node.display_name(), index + 1, batch.total),
            fraction: batch.start_fraction(index),
        });
        Ok(SelectedItem { node, kind })
    }

    /// Trash or permanently delete one user-selected item.
    fn remove(
        &self,
        mode: TransferMode,
        source: &dyn Node,
        cancel: &Cancellation,
    ) -> Result<(), TransferError> {
        // A protected descendant anywhere in the tree stops the whole item
        // before anything is removed.
        check_write_tree(self.guard(), source, None, cancel, SourceChange::Changed)?;
        match mode {
            // Trash never falls back to a permanent delete; the backend
            // reports an error instead.
            TransferMode::Trash => source.trash(cancel),
            // Reached only after the user confirmed a permanent delete. The
            // backend re-checks every item with the write guard.
            _ => source.delete_tree(cancel, self.guard()),
        }
    }

    /// Moves one item. Backends never degrade a move to copy-then-delete
    /// (`NO_FALLBACK_FOR_MOVE`); Replace is an explicit user choice and still
    /// never degrades.
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

    /// Copies one item through private staging; `slot` receives the staging
    /// as soon as it exists.
    fn copy_item(
        &mut self,
        batch: &Batch,
        selected: &SelectedItem,
        dest_dir: &dyn Node,
        destination: &dyn Node,
        slot: &mut StageSlot,
    ) -> Result<(), TransferError> {
        let copy = StagedCopy {
            source: selected.node.as_ref(),
            source_kind: selected.kind,
            dest_dir,
            destination,
            policy: batch.policy,
            cancel: batch.cancel,
            guard: self.write_guard.as_deref(),
            emit: &mut *self.emit,
        };
        copy.run(slot)
    }

    /// Adds a failed item to `result`, unless the user's cancellation
    /// explains it.
    fn record_failure(&self, batch: &Batch, uri: &str, error: &TransferError, result: &mut TransferResult) {
        // Only the user's own cancellation counts: a backend reporting
        // "cancelled" by itself is an ordinary error.
        let cancelled = error.is_cancelled() || batch.cancel.is_cancelled();
        result.cancelled |= cancelled;
        // Cancellation must not hide the location of a retained backup or
        // another problem that requires manual recovery.
        if !cancelled || matches!(error, TransferError::RecoveryRequired(_)) {
            result.errors.push(format!("{}: {error}", self.display_name(uri)));
        }
    }

    /// Removes the staging an item left behind; a leftover is reported with
    /// its exact location.
    fn discard_leftover_stage(&self, slot: StageSlot, result: &mut TransferResult) {
        let Some(stage) = slot.stage else {
            return;
        };
        let root = stage.root();
        if let Err(problem) = discard_stage(root, slot.created, slot.place, &*self.sleep) {
            result.errors.push(leftover_report(root, slot.place, &problem));
        }
    }

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

/// The URIs in their original order, each once.
fn deduplicate(uris: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    uris.iter()
        .filter(|uri| seen.insert(uri.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `test_unknown_operation_rejected` in
    /// `desktop/tests/test_operations.py`. Only the parsing can be tested:
    /// an unknown name never becomes a [`TransferMode`] or
    /// [`ConflictPolicy`], so the engine cannot be asked to run one.
    ///
    /// parity: XFER-019
    #[test]
    fn protocol_names_round_trip_and_unknown_ones_are_refused() {
        for mode in [
            TransferMode::Copy,
            TransferMode::Move,
            TransferMode::Trash,
            TransferMode::Delete,
        ] {
            assert_eq!(mode.as_str().parse::<TransferMode>(), Ok(mode));
        }
        for policy in [
            ConflictPolicy::Skip,
            ConflictPolicy::Replace,
            ConflictPolicy::KeepBoth,
        ] {
            assert_eq!(policy.as_str().parse::<ConflictPolicy>(), Ok(policy));
        }
        assert_eq!(
            "erase".parse::<TransferMode>(),
            Err(TransferError::failed("Unknown operation."))
        );
        assert_eq!(
            "overwrite".parse::<ConflictPolicy>(),
            Err(TransferError::failed(
                "Choose Skip duplicates, Keep both, or Replace existing."
            ))
        );
    }

    /// parity: XFER-019
    #[test]
    fn duplicates_are_dropped_in_order() {
        let uris: Vec<String> = ["b", "a", "b", "c", "a"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(deduplicate(&uris), ["b", "a", "c"]);
    }
}
