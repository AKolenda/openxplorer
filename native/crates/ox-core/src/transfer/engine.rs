// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer orchestration. Ports `TransferEngine.run` and
//! `_run_items` in `v2.0.0:desktop/operations.py`; see the module documentation for
//! the rules. The request is validated in `request.rs`, the settings of the
//! run are in `batch.rs`, the destination name is chosen in `conflicts.rs`,
//! and the copy of one item (staging, publishing, device checks) is in
//! `staged_copy.rs`.

use std::ffi::OsStr;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::batch::{Batch, ItemAction, Removal};
use super::cancellation::Cancellation;
use super::commit::commit_replace;
use super::conflicts::Placement;
use super::containment::guard_destination;
use super::error::TransferError;
use super::guard::{check_write_tree, SourceChange};
use super::item_failure::{FailedItem, FailureAnswer, ItemFailures};
use super::labels::{completed_label, item_label, CHECKING_SPACE_LABEL};
use super::limits::{FreeSpace, Incoming, StorageRules, TreeTotals};
use super::move_by_copying::{MoveByCopying, MoveByCopyingItem};
use super::names::child_node;
use super::node::{Node, NodeFactory, NodeInfo, NodeKind, WriteGuard};
use super::relisting::SourceFolders;
use super::request::{destination_folder, distinct_items};
use super::source_removal::{remove_copied_source, CopiedItem};
use super::staged_copy::{ItemStaging, StagedCopy};
use super::staging::{discard_stage, leftover_report};
use super::types::{
    ConflictPolicy, ItemCount, Landed, Operation, Progress, ProgressScope, TransferMode, TransferResult,
};
use super::unstorable::{Fix, Unstorable, UnstorableAnswer, UnstorableItem};

/// Receives the progress of a run for the transfer panel.
type ProgressCallback = Box<dyn FnMut(Progress) + Send>;

/// Waits between device cleanup attempts; tests record the delay instead.
type SleepCallback = Box<dyn Fn(Duration) + Send + Sync>;

/// The URI under which the destination folder with the given URI is
/// written as administrator, or why it may not be (OPS-047).
pub type ElevatedDestination = dyn Fn(&str) -> Result<String, TransferError> + Send + Sync;

/// Runs copy, move, Trash and delete operations over [`Node`]s.
pub struct TransferEngine {
    factory: NodeFactory,
    /// Named after `self.emit` of the Python engine.
    emit: ProgressCallback,
    /// Accounting is shared with the report closure and reset per engine run.
    byte_counts: Arc<Mutex<Accounting>>,
    write_guard: Option<Box<WriteGuard>>,
    sleep: SleepCallback,
    /// What the run's destination cannot store (XFER-028).
    unstorable: Unstorable,
    /// Whether moves the backend cannot do are finished by copying.
    move_by_copying: MoveByCopying,
    /// What to do when an item fails (OPS-047).
    failures: ItemFailures,
    /// Where Continue writes a destination folder as administrator; without
    /// it Continue fails the item.
    elevation: Option<Box<ElevatedDestination>>,
    /// Asked about every existing folder a Replace written as
    /// administrator merges into.
    elevated_merge_check: Option<Box<WriteGuard>>,
    /// The `admin://` URI an item of a Recycle Bin or delete run is
    /// deleted under after Continue, or why it may not be.
    elevated_item: Option<Box<ElevatedDestination>>,
}

impl fmt::Debug for TransferEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferEngine")
            .field("has_write_guard", &self.write_guard.is_some())
            .finish_non_exhaustive()
    }
}

/// The accounting of a run, shared between the engine, its copies and
/// the progress closure.
pub(crate) type SharedAccounting = Arc<Mutex<Accounting>>;

/// What a run had written and started when an attempt began, so a
/// discarded attempt can be taken back.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Checkpoint {
    written: u64,
    files: u64,
}

/// What a run has written and counted, for its progress reports
/// (OPS-021).
#[derive(Debug, Default)]
pub(crate) struct Accounting {
    /// The bytes of the current file written so far.
    file_written: u64,
    /// The bytes written across the run.
    batch_written: u64,
    /// What the run writes, when the free-space check measured it.
    batch_size: Option<u64>,
    /// The files of the run, when the free-space check counted them.
    files_total: Option<u64>,
    /// The files started so far.
    files_started: u64,
}

impl Accounting {
    /// Where the run is now, before an attempt that may be discarded.
    pub(crate) fn checkpoint(accounting: &SharedAccounting) -> Checkpoint {
        let counts = accounting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Checkpoint {
            written: counts.batch_written,
            files: counts.files_started,
        }
    }

    /// Takes back what an attempt wrote and started since `checkpoint`,
    /// once its copy was discarded to be tried again: the retry writes
    /// those bytes again, and they must count once (OPS-021, OPS-047).
    pub(crate) fn roll_back(accounting: &SharedAccounting, checkpoint: Checkpoint) {
        let mut counts = accounting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        counts.batch_written = counts.batch_written.min(checkpoint.written);
        counts.files_started = counts.files_started.min(checkpoint.files);
        counts.file_written = 0;
    }

    /// Fills in `progress` what the run has written and counted: the
    /// bytes, and the items, counted in files once the free-space check
    /// counted them. An item report says how many files are done; the
    /// last report of the run, all of them.
    fn account(&mut self, progress: &mut Progress) {
        if let Some(bytes) = progress.bytes.as_mut() {
            if bytes.file_written == 0 {
                self.file_written = 0;
            }
            if bytes.file_started {
                self.files_started += 1;
            }
            self.batch_written = self
                .batch_written
                .saturating_add(bytes.file_written.saturating_sub(self.file_written));
            self.file_written = bytes.file_written;
            bytes.batch_written = self.batch_written;
            bytes.batch_size = self.batch_size;
        }
        let Some(total) = self.files_total else {
            return;
        };
        let done = match (progress.items, progress.bytes) {
            (Some(items), _) if items.done >= items.total => total,
            (Some(_), _) => self.files_started,
            (None, Some(_)) => self.files_started.saturating_sub(1),
            (None, None) => return,
        };
        progress.items = Some(ItemCount {
            done: done.min(total),
            total,
        });
    }
}

/// What one run has done so far.
#[derive(Default)]
struct RunState {
    result: TransferResult,
    /// Folders that moves took items from, relisted at the end (MTP).
    moved_from: SourceFolders,
    /// Continue for all current items was answered in a Recycle Bin or
    /// delete run: every later item is deleted as administrator (OPS-047).
    elevated_removal: bool,
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
    /// What it was when selected, before anything changed it.
    info: NodeInfo,
}

impl TransferEngine {
    /// An engine resolving URIs with `factory`, without progress reports or
    /// a write guard.
    pub fn new(factory: NodeFactory) -> Self {
        Self {
            factory,
            emit: Box::new(|_| {}),
            byte_counts: Arc::default(),
            write_guard: None,
            sleep: Box::new(std::thread::sleep),
            unstorable: Unstorable::default(),
            move_by_copying: MoveByCopying::default(),
            failures: ItemFailures::default(),
            elevation: None,
            elevated_merge_check: None,
            elevated_item: None,
        }
    }

    /// Receives progress for the transfer panel.
    #[must_use]
    pub fn with_progress(mut self, mut emit: impl FnMut(Progress) + Send + 'static) -> Self {
        let counts = Arc::clone(&self.byte_counts);
        self.emit = Box::new(move |mut progress: Progress| {
            counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .account(&mut progress);
            emit(progress);
        });
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

    /// XFER-028: asks about each name or link the destination's file system
    /// cannot store. It runs on the engine's thread and blocks the run until
    /// it answers. Without it such items are attempted as they are.
    #[must_use]
    pub fn with_unstorable_question(
        mut self,
        question: impl FnMut(&UnstorableItem) -> UnstorableAnswer + Send + 'static,
    ) -> Self {
        self.unstorable = Unstorable::with_question(Box::new(question));
        self
    }

    /// XFER-011 and XFER-013: asks, the first time in this engine's
    /// operation, whether moves the backend cannot do natively are finished
    /// by copying and then removing the unchanged originals. It runs on the
    /// engine's thread and blocks the run until it answers. Without it, or
    /// when it answers `false`, such moves are refused and the source kept.
    #[must_use]
    pub fn with_move_by_copying_question(
        mut self,
        question: impl FnMut(&MoveByCopyingItem) -> bool + Send + 'static,
    ) -> Self {
        self.move_by_copying = MoveByCopying::with_question(Box::new(question));
        self
    }

    /// OPS-047: asks what to do when an item fails for a reason other than
    /// a name conflict: retry it, skip it, skip every later failure, or
    /// cancel. It runs on the engine's thread and blocks the run until it
    /// answers. Without it the error is recorded and the next item runs.
    #[must_use]
    pub fn with_failure_question(
        mut self,
        question: impl FnMut(&FailedItem) -> FailureAnswer + Send + 'static,
    ) -> Self {
        self.failures = ItemFailures::with_question(Box::new(question));
        self
    }

    /// OPS-047: where the user's Continue writes a destination folder as
    /// administrator. `elevation` returns the folder's `admin://` URI once
    /// it is safe to write there ([`crate::admin_copy::elevated_destination`]),
    /// or the refusal, which fails the item and is asked about like any
    /// other failure.
    #[must_use]
    pub fn with_elevation(
        mut self,
        elevation: impl Fn(&str) -> Result<String, TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.elevation = Some(Box::new(elevation));
        self
    }

    /// OPS-047: where the user's Continue deletes an item of a Recycle Bin
    /// or delete run as administrator: `elevation` returns the item's
    /// `admin://` URI once it is safe to delete it there
    /// ([`crate::admin_copy::elevated_item`]), or the refusal, which fails
    /// the item and is asked about like any other failure.
    #[must_use]
    pub fn with_elevated_item(
        mut self,
        elevation: impl Fn(&str) -> Result<String, TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.elevated_item = Some(Box::new(elevation));
        self
    }

    /// OPS-047: checks each existing folder a Replace written as
    /// administrator merges into before anything is written into it
    /// ([`crate::admin_copy::check_elevated_folder`]); a refusal fails the
    /// item. Without it such a merge is refused outright.
    #[must_use]
    pub fn with_elevated_merge_check(
        mut self,
        check: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.elevated_merge_check = Some(Box::new(check));
        self
    }

    /// The check for folders a Replace into `placement` merges into: none
    /// for an ordinary run, and for a run written as administrator the
    /// installed check, or a refusal of every merge without one.
    fn merge_check(&self, placement: &Placement) -> Option<&WriteGuard> {
        merge_check(self.elevated_merge_check.as_deref(), placement)
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
            } => self.run_transfer(operation.mode(), folder_uri, policy, None, &uris, cancel)?,
            Operation::Trash => {
                self.unstorable.start_run(StorageRules::default(), None);
                let trash = ItemAction::Remove(Removal::Trash);
                self.run_items(trash, &uris, cancel)
            }
            Operation::Delete => {
                self.unstorable.start_run(StorageRules::default(), None);
                let permanent_delete = ItemAction::Remove(Removal::PermanentDelete);
                self.run_items(permanent_delete, &uris, cancel)
            }
        };
        Ok(result)
    }

    /// Copies or moves the item at `uri` into `destination_folder` under
    /// `name` instead of its own name: the user's Rename answer to a name
    /// conflict (OPS-028). Every rule of [`run`](Self::run) applies; a
    /// `name` that is taken by now fails the item rather than replacing
    /// anything.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run), and for a mode other than copy or move.
    pub fn run_renamed(
        &mut self,
        mode: TransferMode,
        destination_folder: &str,
        uri: &str,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferResult, TransferError> {
        if !matches!(mode, TransferMode::Copy | TransferMode::Move) {
            return Err(TransferError::failed(crate::i18n::gettext(
                "Only copies and moves can rename an item.",
            )));
        }
        let uris = [uri.to_owned()];
        let uris = distinct_items(&uris)?;
        self.run_transfer(
            mode,
            destination_folder,
            ConflictPolicy::Skip,
            Some(name),
            &uris,
            cancel,
        )
    }

    /// Copies or moves `uris` into the folder at `folder_uri`.
    fn run_transfer(
        &mut self,
        mode: TransferMode,
        folder_uri: &str,
        policy: ConflictPolicy,
        name: Option<&OsStr>,
        uris: &[&str],
        cancel: &Cancellation,
    ) -> Result<TransferResult, TransferError> {
        let folder = destination_folder(&self.factory, folder_uri, cancel)?;
        // XFER-028: what the destination's file system can hold.
        let mut filesystem = folder.filesystem(Some(cancel)).unwrap_or_default();
        self.unstorable.start_run(
            StorageRules::of(filesystem.kind.as_deref()),
            filesystem.id.clone(),
        );
        // Walking folders on a share can take a while: say so once, as
        // Dolphin shows its examining phase.
        let mut announced = false;
        let emit = &mut self.emit;
        let mut on_folder = || {
            if !std::mem::replace(&mut announced, true) {
                emit(Progress {
                    label: crate::i18n::gettext(CHECKING_SPACE_LABEL),
                    fraction: 0.0,
                    scope: ProgressScope::Batch,
                    bytes: None,
                    items: None,
                    name: None,
                });
            }
        };
        let measured = loop {
            let incoming = Incoming {
                mode,
                policy,
                folder: folder.as_ref(),
                filesystem: &filesystem,
            };
            let shortfall = match incoming.check_free_space(&self.factory, uris, cancel, &mut on_folder)? {
                FreeSpace::Fits(measured) => break measured,
                FreeSpace::Short(shortfall) => shortfall,
            };
            // As Windows asks before it copies: "There is not enough
            // space", with Try Again, which checks again, and Cancel.
            let error = shortfall.error(&folder.display_name());
            if cancel.is_cancelled() || !self.failures.asks_about(&error) {
                return Err(error);
            }
            let question = FailedItem {
                more_items: false,
                needed: Some(shortfall.needed),
                ..FailedItem::new(mode, folder.display_name(), "", Some(folder_uri), &error)
            };
            match self.failures.answer(&question) {
                // The user may have made room meanwhile: read the free
                // space again.
                Some(FailureAnswer::Retry) => {
                    filesystem = folder.filesystem(Some(cancel)).unwrap_or_default();
                }
                Some(FailureAnswer::Cancel) => return Err(TransferError::Cancelled),
                _ => return Err(error),
            }
        };
        self.report_batch_size(measured);
        let placement = Placement {
            mode,
            policy,
            destination_folder: folder.as_ref(),
            requested_folder: folder.as_ref(),
            name,
        };
        Ok(self.run_items(ItemAction::Transfer(placement), uris, cancel))
    }

    /// Makes every byte report of this run say that the batch writes
    /// the `measured` bytes, for the time left, and every item report
    /// count its files, for the items remaining (OPS-021). A run that
    /// writes no file counts its selected items instead.
    fn report_batch_size(&mut self, measured: Option<TreeTotals>) {
        let mut counts = self
            .byte_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *counts = Accounting {
            batch_size: measured.map(|totals| totals.bytes),
            files_total: measured.map(|totals| totals.files).filter(|files| *files > 0),
            ..Accounting::default()
        };
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
        // The destination under `admin://` once the user answered Continue
        // for all current items (OPS-047).
        let mut elevated: Option<Box<dyn Node>> = None;
        for (index, uri) in uris.iter().enumerate() {
            self.run_item(&batch, index, uri, &mut state, &mut elevated);
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
            scope: ProgressScope::Batch,
            bytes: None,
            items: Some(ItemCount {
                done: batch.total as u64,
                total: batch.total as u64,
            }),
            name: None,
        });
        state.result
    }

    /// Runs one top-level item and records its outcome. Staging this item
    /// created is removed afterwards, whatever happened; a leftover is
    /// reported with its exact location.
    ///
    /// A failure the user may answer is asked about first (OPS-047):
    /// Retry runs the item again once its staging is gone, Continue runs it
    /// again into the destination under `admin://` (and with Continue for
    /// all, every later item of the run, through `elevated`), Cancel stops
    /// the run after recording it.
    fn run_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        state: &mut RunState,
        elevated: &mut Option<Box<dyn Node>>,
    ) {
        let mut staging = ItemStaging::default();
        let checkpoint = Accounting::checkpoint(&self.byte_counts);
        // The question names the folder the user chose, never `admin://`.
        let destination = batch.destination_uri();
        // This item's own destination under `admin://`, after Continue.
        let mut item_folder: Option<Box<dyn Node>> = None;
        self.failures
            .start_item(batch.mode(), self.display_name(uri), destination.clone());
        // Whether this item is deleted as administrator, after Continue.
        let (mut outcome, mut removal_elevated) =
            self.first_attempt(batch, elevated.as_deref(), index, uri, state, &mut staging);
        // Whether the user saw the error of `outcome` in the question.
        let mut answered = false;
        while let Err(error) = &outcome {
            if batch.cancel.is_cancelled() {
                break;
            }
            // Continue answered about an entry inside the item: the item
            // runs again as administrator without asking twice.
            let asked_inside = self.failures.take_elevation();
            if asked_inside.is_none() && !self.failures.asks_about(error) {
                break;
            }
            let failed = FailedItem {
                more_items: index + 1 < batch.total,
                ..FailedItem::new(
                    batch.mode(),
                    self.display_name(uri),
                    uri,
                    destination.as_deref(),
                    error,
                )
            };
            let answer = match asked_inside {
                Some(answer) => Some(answer),
                None => self.failures.answer(&failed),
            };
            match answer {
                Some(FailureAnswer::Retry) => {
                    self.discard_leftover_stage(std::mem::take(&mut staging), &mut state.result);
                    Accounting::roll_back(&self.byte_counts, checkpoint);
                    self.failures
                        .start_item(batch.mode(), self.display_name(uri), destination.clone());
                    let folder = elevated.as_deref().or(item_folder.as_deref());
                    outcome = if removal_elevated {
                        self.remove_elevated(batch, index, uri, state)
                    } else {
                        self.attempt(batch, folder, index, uri, state, &mut staging)
                    };
                }
                Some(answer @ (FailureAnswer::Elevate | FailureAnswer::ElevateAll)) if batch.is_removal() => {
                    removal_elevated = true;
                    state.elevated_removal |= answer == FailureAnswer::ElevateAll;
                    outcome = self.remove_elevated(batch, index, uri, state);
                }
                Some(answer @ (FailureAnswer::Elevate | FailureAnswer::ElevateAll)) => {
                    self.discard_leftover_stage(std::mem::take(&mut staging), &mut state.result);
                    Accounting::roll_back(&self.byte_counts, checkpoint);
                    self.failures
                        .start_item(batch.mode(), self.display_name(uri), destination.clone());
                    outcome = match self.elevated_folder(batch) {
                        Ok(folder) => {
                            let slot = if answer == FailureAnswer::ElevateAll {
                                &mut *elevated
                            } else {
                                &mut item_folder
                            };
                            let folder: &dyn Node = &**slot.insert(folder);
                            self.attempt(batch, Some(folder), index, uri, state, &mut staging)
                        }
                        // Refused before anything was written: asked about
                        // like any failure, so the user sees why.
                        Err(error) => Err(error),
                    };
                }
                Some(FailureAnswer::Cancel) => {
                    state.result.cancelled = true;
                    answered = true;
                    break;
                }
                Some(FailureAnswer::Skip | FailureAnswer::SkipAll) => {
                    answered = true;
                    break;
                }
                None => break,
            }
        }
        // OPS-047: the entries inside it that were left out, even when the
        // item failed or was cancelled after them.
        for (message, seen) in self.failures.take_left_out() {
            if seen {
                state.result.presented.push(message.clone());
            }
            state.result.errors.push(message);
        }
        match outcome {
            Ok(ItemOutcome::Skipped) => state.result.skipped.push(uri.to_owned()),
            Ok(ItemOutcome::Done) => {
                state.result.done.push(uri.to_owned());
                // A published copy leaves its private folder empty. Failing
                // to remove it is reported, but the copy stays done.
                if let Err(error) = staging.remove_empty_folder() {
                    self.record_failure(batch, uri, &error, false, &mut state.result);
                }
            }
            Err(error) => self.record_failure(batch, uri, &error, answered, &mut state.result),
        }
        self.discard_leftover_stage(staging, &mut state.result);
    }

    /// The first attempt at the item at `uri`, and whether it ended as
    /// administrator. Every item is first copied, moved, trashed or deleted
    /// as the user. Continue for all current items in a Recycle Bin or
    /// delete run then answers only an item the user may not delete;
    /// anything else stays the user's own delete, as asked (OPS-047).
    fn first_attempt(
        &mut self,
        batch: &Batch,
        elevated: Option<&dyn Node>,
        index: usize,
        uri: &str,
        state: &mut RunState,
        staging: &mut ItemStaging,
    ) -> (Result<ItemOutcome, TransferError>, bool) {
        let outcome = self.attempt(batch, elevated, index, uri, state, staging);
        let denied = matches!(outcome, Err(TransferError::PermissionDenied(_)));
        if state.elevated_removal && batch.is_removal() && denied && !batch.cancel.is_cancelled() {
            return (self.remove_elevated(batch, index, uri, state), true);
        }
        (outcome, false)
    }

    /// Runs one attempt at the top-level item at `uri`, writing into
    /// `elevated`, the destination folder under `admin://`, when the user
    /// answered Continue.
    fn attempt(
        &mut self,
        batch: &Batch,
        elevated: Option<&dyn Node>,
        index: usize,
        uri: &str,
        state: &mut RunState,
        staging: &mut ItemStaging,
    ) -> Result<ItemOutcome, TransferError> {
        let Some(elevated_batch) = elevated.and_then(|folder| batch.writing_into(folder)) else {
            return self.process_item(batch, index, uri, state, staging);
        };
        self.move_by_copying.set_elevated(true);
        let outcome = self.process_item(&elevated_batch, index, uri, state, staging);
        self.move_by_copying.set_elevated(false);
        outcome
    }

    /// Deletes the item at `uri` of the Recycle Bin or delete run `batch`
    /// permanently as administrator, under `admin://`: Continue (OPS-047).
    /// The Recycle Bin has no administrator path, so the app confirmed the
    /// permanent delete first. A folder goes to the privileged helper of
    /// [`crate::admin_delete`], anything else to `gvfsd-admin`.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotSupported`] without an item elevation, its
    /// refusal, and every error of a permanent delete.
    fn remove_elevated(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        state: &mut RunState,
    ) -> Result<ItemOutcome, TransferError> {
        let Some(elevation) = self.elevated_item.as_ref() else {
            return Err(TransferError::NotSupported(crate::i18n::gettext(
                "This item can't be deleted as administrator.",
            )));
        };
        let admin = elevation(uri)?;
        let delete = Batch {
            action: ItemAction::Remove(Removal::PermanentDelete),
            cancel: batch.cancel,
            total: batch.total,
        };
        let mut staging = ItemStaging::default();
        self.process_item(&delete, index, &admin, state, &mut staging)
    }

    /// The destination folder of `batch` under `admin://`, for Continue.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotSupported`] without an elevation or a
    /// destination, the elevation's refusal, and the error of reaching the
    /// folder there.
    fn elevated_folder(&self, batch: &Batch) -> Result<Box<dyn Node>, TransferError> {
        let (Some(elevation), Some(folder)) = (self.elevation.as_ref(), batch.destination_uri()) else {
            return Err(TransferError::NotSupported(crate::i18n::gettext(
                "This item can't be copied as administrator.",
            )));
        };
        let admin = elevation(&folder)?;
        destination_folder(&self.factory, &admin, batch.cancel)
    }

    /// Copies, moves, trashes or deletes one top-level item.
    fn process_item(
        &mut self,
        batch: &Batch,
        index: usize,
        uri: &str,
        state: &mut RunState,
        staging: &mut ItemStaging,
    ) -> Result<ItemOutcome, TransferError> {
        let selected = self.start_item(batch, index, uri)?;
        match &batch.action {
            ItemAction::Remove(removal) => {
                self.remove(*removal, &selected, batch.cancel)?;
                Ok(ItemOutcome::Done)
            }
            ItemAction::Transfer(placement) => self.transfer(batch, placement, &selected, state, staging),
        }
    }

    /// Resolves the selected item at `uri` and announces it on the progress
    /// panel.
    fn start_item(&mut self, batch: &Batch, index: usize, uri: &str) -> Result<SelectedItem, TransferError> {
        // OPS-021: a paused run waits before its next item.
        batch.cancel.wait_while_paused();
        batch.cancel.check()?;
        let node = (self.factory)(uri)?;
        // XFER-019: a root has no name to copy, move or trash it under.
        if node.parent().is_none() {
            return Err(TransferError::failed(crate::i18n::gettext(
                "Filesystem roots cannot be copied, moved or trashed as items.",
            )));
        }
        let info = node.info(Some(batch.cancel))?;
        let name = node.display_name();
        (self.emit)(Progress {
            label: item_label(batch.mode(), &name, index + 1, batch.total),
            fraction: batch.start_fraction(index),
            scope: ProgressScope::Batch,
            bytes: None,
            items: Some(ItemCount {
                done: index as u64,
                total: batch.total as u64,
            }),
            name: Some(name),
        });
        Ok(SelectedItem {
            node,
            kind: info.kind,
            info,
        })
    }

    /// Trashes or permanently deletes one user-selected item.
    fn remove(
        &self,
        removal: Removal,
        selected: &SelectedItem,
        cancel: &Cancellation,
    ) -> Result<(), TransferError> {
        let source = selected.node.as_ref();
        // A permanent delete of anything but a folder never looks inside
        // the item (`delete_tree_of_kind`), so the kind found when it was
        // selected is enough here: on a share every question is a queued
        // round trip, which adds up over many files. The Trash takes the
        // item whole, so it is asked again.
        let known_kind = match removal {
            Removal::PermanentDelete if selected.kind != NodeKind::Directory => Some(selected.kind),
            _ => None,
        };
        // XFER-020: a protected descendant anywhere in the tree stops the
        // whole item before anything is removed.
        check_write_tree(
            self.guard(),
            source,
            known_kind,
            None,
            cancel,
            SourceChange::Changed,
        )?;
        match removal {
            // XFER-014: the backend reports an error rather than falling
            // back to a permanent delete.
            Removal::Trash => source.trash(cancel),
            // XFER-015: reached only after the user confirmed a permanent
            // delete. The backend re-checks every item with the write guard.
            // The item is bound to what was selected, where the backend
            // can tell (`delete_selected_tree`).
            Removal::PermanentDelete => source.delete_selected_tree(&selected.info, cancel, self.guard()),
        }
    }

    /// Copies or moves one selected item into the destination folder of
    /// `placement`, under the name its conflict policy chooses, and records
    /// where it landed in `state`.
    fn transfer(
        &mut self,
        batch: &Batch,
        placement: &Placement,
        selected: &SelectedItem,
        state: &mut RunState,
        staging: &mut ItemStaging,
    ) -> Result<ItemOutcome, TransferError> {
        let source = selected.node.as_ref();
        if selected.kind == NodeKind::Directory {
            guard_destination(source, placement.destination_folder)?;
            // Written as administrator, the folder is also checked against
            // the `file:` folder the user chose, whose links resolve.
            guard_destination(source, placement.requested_folder)?;
        }
        self.unstorable.start_item(source, batch.cancel);
        // XFER-028: a name or link the destination cannot store.
        let Fix::Name(name) = self.unstorable.fix(source, Some(selected.kind), batch.cancel)? else {
            return Ok(ItemOutcome::Skipped);
        };
        let Some(destination) = placement.destination_for(source, &name, selected.kind, batch.cancel)? else {
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
            None,
            Some(destination),
            batch.cancel,
            source_change,
        )?;
        // Written as administrator, the same tree is checked again where
        // the user sees it: snapshot roots are known by their `file:` URIs.
        if placement.requested_folder.uri() != placement.destination_folder.uri() {
            let requested = child_node(placement.requested_folder, destination.name())?;
            check_write_tree(
                self.guard(),
                source,
                None,
                Some(requested.as_ref()),
                batch.cancel,
                SourceChange::Kept,
            )?;
        }
        if placement.mode == TransferMode::Move {
            state.moved_from.remember(source);
            let moved = self.move_item(
                source,
                destination,
                placement.policy,
                batch.cancel,
                self.merge_check(placement),
            );
            match moved {
                // XFER-013: the backend cannot move here (another filesystem,
                // share or device). Once the user agreed, the item is copied
                // through staging and the source is removed only once its
                // copy is published; otherwise it is kept (XFER-011).
                Err(TransferError::NotSupported(_))
                    if self.move_by_copying.allowed(|| MoveByCopyingItem {
                        name: source.display_name(),
                        destination: placement.destination_folder.display_name(),
                    }) =>
                {
                    let kept =
                        self.move_by_copying(placement, selected, destination, batch.cancel, staging)?;
                    if let Some(notice) = kept {
                        state
                            .result
                            .errors
                            .push(format!("{}: {notice}", source.display_name()));
                    }
                }
                moved => moved?,
            }
        } else {
            self.copy_item(placement, selected, destination, batch.cancel, staging, None)?;
        }
        state.result.landed.push(Landed {
            source: source.uri(),
            destination: destination.uri(),
        });
        Ok(ItemOutcome::Done)
    }

    /// Moves one item natively. XFER-011: backends never degrade a move to
    /// an unstaged copy-then-delete (`NO_FALLBACK_FOR_MOVE`); where they
    /// cannot move, the engine copies through staging instead once the user
    /// agreed (XFER-013).
    fn move_item(
        &self,
        source: &dyn Node,
        destination: &dyn Node,
        policy: ConflictPolicy,
        cancel: &Cancellation,
        merge_check: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        if policy == ConflictPolicy::Replace {
            commit_replace(source, destination, cancel, self.guard(), None, merge_check)
        } else {
            source.move_native(destination, Some(cancel))
        }
    }

    /// XFER-013: moves one item the backend cannot move natively by copying
    /// it through private staging (every copy rule applies) and then
    /// removing the copied source items. A failed or cancelled copy keeps
    /// the source. Once the copy is published the source removal runs to
    /// the end, so the item is either moved or kept, never half removed by
    /// a late cancellation. Items that appeared in the source during the
    /// copy, or changed after the copy read them, are kept; the answer then
    /// says where.
    fn move_by_copying(
        &mut self,
        placement: &Placement,
        selected: &SelectedItem,
        destination: &dyn Node,
        cancel: &Cancellation,
        staging: &mut ItemStaging,
    ) -> Result<Option<String>, TransferError> {
        let mut copied = Vec::new();
        self.copy_item(
            placement,
            selected,
            destination,
            cancel,
            staging,
            Some(&mut copied),
        )?;
        if self.unstorable.take_skipped() > 0 {
            return Err(TransferError::RecoveryRequired(crate::i18n::format_message(
                "The copy at {uri} leaves out items the destination cannot store, so the original was kept.",
                &[("uri", &destination.uri())],
            )));
        }
        let source = selected.node.as_ref();
        let left_out = self.failures.left_out_sources();
        let kept = remove_copied_source(source, &selected.info, &copied, left_out, self.guard()).map_err(|error| {
            TransferError::RecoveryRequired(crate::i18n::format_message("The item was copied to {uri}, but the original could not be removed. Check the copy, then delete the original. {error}", &[("uri", &destination.uri()), ("error", &(error).to_string())]))
        })?;
        Ok(kept.notice(&source.uri()))
    }

    /// Copies one item into the destination folder of `placement` through
    /// private staging, until the user cancels through `cancel`; `staging`
    /// receives the staging as soon as it exists, and `copied` each copied
    /// source item when the copy finishes a move.
    fn copy_item(
        &mut self,
        placement: &Placement,
        selected: &SelectedItem,
        destination: &dyn Node,
        cancel: &Cancellation,
        staging: &mut ItemStaging,
        copied: Option<&mut Vec<CopiedItem>>,
    ) -> Result<(), TransferError> {
        let copy = StagedCopy {
            source: selected.node.as_ref(),
            source_kind: selected.kind,
            destination_folder: placement.destination_folder,
            destination,
            policy: placement.policy,
            cancel,
            guard: self.write_guard.as_deref(),
            merge_check: merge_check(self.elevated_merge_check.as_deref(), placement),
            unstorable: &mut self.unstorable,
            failures: &mut self.failures,
            emit: &mut *self.emit,
            accounting: &self.byte_counts,
            copied,
        };
        copy.run(staging)
    }

    /// Adds a failed item to `result`, unless the user's cancellation
    /// explains it; `answered` when the user saw the error in the question.
    fn record_failure(
        &self,
        batch: &Batch,
        uri: &str,
        error: &TransferError,
        answered: bool,
        result: &mut TransferResult,
    ) {
        // Only the user's own cancellation counts: a backend reporting
        // "cancelled" by itself is an ordinary error.
        let cancelled = error.is_cancelled() || batch.cancel.is_cancelled();
        result.cancelled |= cancelled;
        // XFER-003 and XFER-010: cancellation must not hide the location of
        // a retained backup or another problem that requires manual
        // recovery.
        let needs_recovery = matches!(error, TransferError::RecoveryRequired(_));
        if !cancelled || needs_recovery {
            let message = format!("{}: {error}", self.display_name(uri));
            if needs_recovery {
                result.recovery.push(message.clone());
            }
            if answered {
                result.presented.push(message.clone());
            }
            result.errors.push(message);
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
            let message = leftover_report(root, staging.place, &problem);
            result.recovery.push(message.clone());
            result.errors.push(message);
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

/// The check for folders a Replace into `placement` merges into, from the
/// engine's `installed` check (see `TransferEngine::merge_check`).
fn merge_check<'a>(installed: Option<&'a WriteGuard>, placement: &Placement) -> Option<&'a WriteGuard> {
    if placement.requested_folder.uri() == placement.destination_folder.uri() {
        return None;
    }
    Some(installed.unwrap_or(&refuse_elevated_merge))
}

/// Refuses a merge into an existing folder written as administrator when
/// no check is installed.
fn refuse_elevated_merge(uri: &str) -> Result<(), TransferError> {
    Err(TransferError::failed(crate::i18n::format_message(
        "{uri} already exists, so the items can't be merged into it as administrator.",
        &[("uri", uri)],
    )))
}
