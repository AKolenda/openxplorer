// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer orchestration. Port of `TransferEngine.run` and
//! `_run_items` in `desktop/operations.py`; see the module documentation for
//! the rules. The copy of one item (staging, publishing, device checks) is
//! in `staged_copy.rs`.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::time::Duration;

use super::commit::commit_replace;
use super::error::TransferError;
use super::guard::{check_write_tree, guard_destination};
use super::labels::{completed_label, item_label};
use super::names::{child_node, new_copy_name};
use super::node::{Cancellation, Node, NodeFactory, NodeKind, WriteGuard};
use super::staged_copy::{StageSlot, StagedCopy};
use super::staging::discard_stage;
use super::types::{ConflictPolicy, Progress, TransferMode, TransferResult};

/// The most items one run accepts.
pub const MAX_ITEMS: usize = 100_000;

/// "Keep both" gives up once `(copy N)` would pass this number.
const MAX_COPY_NUMBER: u32 = 10_000;

type Emit = Box<dyn FnMut(Progress) + Send>;
type AssertWritable = Box<dyn Fn(&str) -> Result<(), TransferError> + Send + Sync>;
type Sleep = Box<dyn Fn(Duration) + Send + Sync>;

/// Runs copy, move, Trash and delete operations over [`Node`]s.
pub struct TransferEngine {
    factory: NodeFactory,
    emit: Emit,
    assert_writable: Option<AssertWritable>,
    sleep: Sleep,
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

impl TransferEngine {
    /// An engine resolving URIs with `factory`, without progress reports or
    /// a write guard.
    pub fn new(factory: NodeFactory) -> Self {
        Self {
            factory,
            emit: Box::new(|_| {}),
            assert_writable: None,
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Receives progress for the transfer panel.
    pub fn with_progress(mut self, emit: impl FnMut(Progress) + Send + 'static) -> Self {
        self.emit = Box::new(emit);
        self
    }

    /// Rejects writes into protected locations such as snapshot folders.
    pub fn with_write_guard(
        mut self,
        guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.assert_writable = Some(Box::new(guard));
        self
    }

    /// Replaces the delay used between device cleanup retries (tests).
    pub fn with_sleep(mut self, sleep: impl Fn(std::time::Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// Runs one operation over `uris`. `target` is the destination folder
    /// for copies and moves and is ignored for Trash and delete.
    ///
    /// Invalid requests (no items, no destination, a destination that is not
    /// a folder, or a cancellation before the destination could be checked)
    /// return an error before anything is changed. Everything after that is
    /// reported per item in the result.
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
        let removal = mode.is_removal();
        let dest_dir = match target.filter(|target| !target.is_empty()) {
            Some(target) if !removal => Some((self.factory)(target)?),
            _ => None,
        };
        if !removal && dest_dir.is_none() {
            return Err(TransferError::failed("Choose a destination folder."));
        }
        if let Some(dest_dir) = &dest_dir {
            if !dest_dir.is_directory(Some(cancel))? {
                return Err(TransferError::failed("The destination is not a folder."));
            }
        }
        let batch = Batch {
            mode,
            policy,
            dest_dir: dest_dir.as_deref(),
            cancel,
            total: uris.len(),
        };
        let mut result = TransferResult::default();
        let mut moved_from = SourceFolders::default();
        for (index, uri) in uris.iter().enumerate() {
            self.run_item(&batch, index, uri, &mut result, &mut moved_from);
            if result.cancelled {
                break;
            }
        }
        // MTP keeps resolving a moved item's OLD path to the object until
        // that folder is listed again. Relist once per source folder.
        moved_from.refresh_all();
        (self.emit)(Progress {
            label: completed_label(result.done.len()),
            fraction: 1.0,
        });
        Ok(result)
    }

    /// Runs one top-level item and records its outcome. Staging this item
    /// created is removed afterwards, whatever happened; a leftover is
    /// reported with its exact location.
    fn run_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        result: &mut TransferResult,
        moved_from: &mut SourceFolders,
    ) {
        let mut slot = StageSlot::default();
        let outcome = self.process_item(batch, index, uri, result, moved_from, &mut slot);
        if let Err(error) = outcome {
            // Only the user's own cancellation counts: a backend reporting
            // "cancelled" by itself is an ordinary error.
            let cancelled = error.is_cancelled() || batch.cancel.is_cancelled();
            result.cancelled |= cancelled;
            // Cancellation must not hide the location of a retained backup
            // or another problem that requires manual recovery.
            if !cancelled || matches!(error, TransferError::RecoveryRequired(_)) {
                result.errors.push(format!("{}: {error}", self.display_name(uri)));
            }
        }
        if let Some(stage) = slot.stage.take() {
            let root = stage.root();
            if let Some(problem) = discard_stage(root, slot.device, &*self.sleep) {
                let what = if slot.device { "item" } else { "folder" };
                result.errors.push(format!(
                    "Incomplete staging {what} left at {}. Inspect it before removing it. {problem}",
                    root.uri()
                ));
            }
        }
    }

    fn process_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        result: &mut TransferResult,
        moved_from: &mut SourceFolders,
        slot: &mut StageSlot,
    ) -> Result<(), TransferError> {
        let cancel = batch.cancel;
        cancel.check()?;
        let source = (self.factory)(uri)?;
        if source.parent().is_none() {
            return Err(TransferError::failed(
                "Filesystem roots cannot be copied, moved or trashed as items.",
            ));
        }
        let info = source.info(Some(cancel))?;
        // Trash and delete have no byte progress, so the batch position is
        // the only honest fraction to show for them.
        let fraction = if batch.mode.is_removal() {
            index as f64 / batch.total as f64
        } else {
            0.0
        };
        (self.emit)(Progress {
            label: item_label(batch.mode, &source.display_name(), index + 1, batch.total),
            fraction,
        });
        if batch.mode.is_removal() {
            self.remove(batch.mode, source.as_ref(), cancel)?;
            result.done.push(uri.to_string());
            return Ok(());
        }
        let dest_dir = batch
            .dest_dir
            .ok_or_else(|| TransferError::failed("Choose a destination folder."))?;
        let is_directory = info.kind == NodeKind::Directory;
        if is_directory {
            guard_destination(source.as_ref(), dest_dir)?;
        }
        let source_name = source.name();
        let mut destination = child_node(dest_dir, &source_name)?;
        if batch.mode == TransferMode::Move && destination.uri() == source.uri() {
            result.skipped.push(uri.to_string());
            return Ok(());
        }
        if destination.exists(Some(cancel)) {
            match batch.policy {
                // Skip never touches the existing item.
                ConflictPolicy::Skip => {
                    result.skipped.push(uri.to_string());
                    return Ok(());
                }
                ConflictPolicy::KeepBoth => {
                    destination = free_copy_name(dest_dir, destination, &source_name, is_directory, cancel)?;
                }
                ConflictPolicy::Replace => {}
            }
        }
        // Check every affected path before changing this top-level item: a
        // writable parent can contain protected backup descendants.
        let source_writable = batch.mode == TransferMode::Move;
        let guard = self.guard();
        check_write_tree(
            guard,
            source.as_ref(),
            Some(destination.as_ref()),
            cancel,
            source_writable,
        )?;
        if batch.mode == TransferMode::Move {
            moved_from.remember(source.as_ref());
            self.move_item(source.as_ref(), destination.as_ref(), batch.policy, cancel)?;
            result.done.push(uri.to_string());
            return Ok(());
        }
        let copy = StagedCopy {
            source: source.as_ref(),
            is_directory,
            dest_dir,
            destination: destination.as_ref(),
            replace: batch.policy == ConflictPolicy::Replace,
            cancel,
            guard: self.assert_writable.as_deref(),
            emit: &mut *self.emit,
        };
        copy.run(slot)?;
        result.done.push(uri.to_string());
        // The private folder is empty now. A plain delete removes only an
        // empty folder, so it can never take the published item with it.
        if let Some(stage) = &slot.stage {
            stage.root().delete()?;
        }
        slot.stage = None;
        Ok(())
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
        check_write_tree(self.guard(), source, None, cancel, true)?;
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

    fn guard(&self) -> Option<&WriteGuard> {
        self.assert_writable.as_deref()
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

/// The first free Windows-style duplicate name, starting at `(copy 2)`.
fn free_copy_name(
    dest_dir: &dyn Node,
    taken: Box<dyn Node>,
    source_name: &OsStr,
    is_directory: bool,
    cancel: &Cancellation,
) -> Result<Box<dyn Node>, TransferError> {
    // Duplicate names are text. A name that is not UTF-8 is refused rather
    // than given a lossily converted "(copy N)" name.
    let Some(source_name) = source_name.to_str() else {
        return Err(TransferError::failed(
            "This item's name is not valid UTF-8, so no duplicate name can be made. Rename it before choosing Keep both.",
        ));
    };
    let mut destination = taken;
    let mut number = 2;
    while destination.exists(Some(cancel)) {
        cancel.check()?;
        let name = new_copy_name(source_name, number, is_directory)?;
        destination = child_node(dest_dir, &name)?;
        number += 1;
        if number > MAX_COPY_NUMBER {
            return Err(TransferError::failed(
                "Too many duplicate names. Rename the item before copying.",
            ));
        }
    }
    Ok(destination)
}

/// The folders moves took items from, each once, in first-use order.
#[derive(Default)]
struct SourceFolders {
    folders: Vec<Box<dyn Node>>,
}

impl SourceFolders {
    /// Remembers `source`'s folder. Called before the move, so a failed or
    /// partial device move is relisted too.
    fn remember(&mut self, source: &dyn Node) {
        let Some(parent) = source.parent() else {
            return;
        };
        let uri = parent.uri();
        if !self.folders.iter().any(|folder| folder.uri() == uri) {
            self.folders.push(parent);
        }
    }

    /// Relists every remembered folder. Best effort: an unmounted device
    /// drops its cache anyway.
    fn refresh_all(&self) {
        for folder in &self.folders {
            let _ = folder.refresh_listing(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `test_unknown_operation_rejected` in
    /// `desktop/tests/test_operations.py` (the parsing half; the engine half
    /// is in `tests/transfer_operations.rs`).
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

    #[test]
    fn duplicates_are_dropped_in_order() {
        let uris: Vec<String> = ["b", "a", "b", "c", "a"].iter().map(|s| s.to_string()).collect();
        assert_eq!(deduplicate(&uris), ["b", "a", "c"]);
    }
}
