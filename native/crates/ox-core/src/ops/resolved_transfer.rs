// SPDX-License-Identifier: AGPL-3.0-only
//! A copy or move whose name conflicts the user answered, as Windows
//! Explorer carries them out (OPS-026, OPS-028).
//!
//! The plan comes from [`scan_name_conflicts`]: each item goes into its
//! folder, the destination or a folder merged into one there. Every step
//! runs on the transfer engine with all of its safety rules:
//!
//! - Items without a conflict go in with Skip, so a name taken after the
//!   check is still never overwritten (OPS-027); a copy into the item's
//!   own folder becomes `name - Copy` (Keep both).
//! - Skip ("Skip this file") leaves both files as they are.
//! - Both versions kept in the File Conflict window: the copied file gets
//!   Windows' number, `report (2).png`, the first free one.
//! - Replace: Windows overwrites the existing file. Here the existing
//!   file goes to the Recycle Bin first, where the desktop's Trash works,
//!   and the incoming file then takes its free name, so Undo can bring
//!   the replaced file back. Should the incoming file not arrive, the
//!   replaced one is restored at once. Where nothing can go to the
//!   Recycle Bin (most shares and devices), or a file meets a folder, the
//!   engine's own Replace runs, which overwrites a file reversibly and
//!   refuses to replace a folder with a file or the other way round
//!   (XFER-009, XFER-010); that replaced file is gone, so Undo leaves
//!   its replacement in place.
//!
//! The items without a conflict go first, before the user answers
//! ([`run_free_items`]), as Windows queues the conflicts "to the end of
//! the copy job" (Building Windows 8, 2011); the answered ones follow
//! ([`run_answered_items`]), the safe answers first. A move removes each
//! merged folder it emptied, as Windows does; one still holding a skipped
//! file stays. A cancellation stops the steps that have not started.
//!
//! [`scan_name_conflicts`]: super::scan_name_conflicts

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use super::conflicts::{ConflictScan, NameConflict, PlannedItem, PlannedKind};
use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::folder_groups::FolderGroups;
use super::recycle_bin::restore_trashed_since;
use super::results::merge_results;
use super::run_transfer::{
    gio_transfer_engine, run_on_engine, run_renamed_on_engine, unix_seconds_now, TransferOutcome,
    TransferRequest,
};
use super::undo::{MovedItem, UndoRecord};
use crate::gio_node::GioNode;
use crate::location::{kept_both_name, ItemKind};
use crate::transfer::SourceChange;
use crate::transfer::{
    ConflictPolicy, Landed, Node, Operation, Progress, ProgressScope, TransferEngine, TransferMode,
    TransferResult, MAX_ITEMS,
};

/// The most numbers tried for a kept-both name.
pub(super) const MAX_KEPT_NUMBER: u32 = 10_000;

/// The user's answer about one conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Keep the existing item; the incoming one stays where it is.
    Skip,
    /// The incoming item takes the existing one's place.
    Replace,
    /// Both stay; the incoming one gets a number added to its name.
    KeepBoth,
}

/// What the first part of a transfer with conflicts did: the items
/// without a conflict, copied or moved while the user has not answered
/// yet. Windows copies those first and asks about the rest at the end
/// ("conflicts are all queued to the end of the copy job", Building
/// Windows 8, 2011).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeItemsDone {
    /// Their outcome.
    total: TransferOutcome,
    /// Where they landed.
    landed: Vec<Landed>,
    /// When the transfer started, in seconds since the Unix epoch.
    started: u64,
    /// How many planned items the steps so far handled.
    before: u64,
    /// The merged folders a move emptied and removed.
    removed_folders: Vec<String>,
}

impl FreeItemsDone {
    /// True when the user cancelled while they were copied or moved.
    pub fn cancelled(&self) -> bool {
        self.total.result.cancelled
    }

    /// The outcome of the transfer stopped before the conflicts were
    /// answered: only these items count, and it reads as cancelled, so
    /// nothing is reported but what needs the user.
    pub fn into_outcome(mut self, scan: &ConflictScan, context: &OperationContext) -> TransferOutcome {
        self.total.result.cancelled = true;
        let mut run = Run::new(scan, &[], context, |_| {}, Some(self));
        if scan.mode == TransferMode::Move {
            run.remove_emptied_folders();
        }
        run.finish()
    }
}

/// Runs `scan` with `answers`, one per conflict in order, sending
/// throttled progress to `progress` on the worker thread: the items
/// without a conflict first ([`run_free_items`]), then the answered ones
/// ([`run_answered_items`]). The outcome adds up every step, and its Undo
/// reverses the whole paste as one step.
///
/// # Errors
///
/// A step refused before anything changed, as [`run_transfer`] refuses
/// it, or answers that do not match the conflicts; the steps that already
/// ran stay done.
///
/// [`run_transfer`]: super::run_transfer
pub async fn run_resolved_transfer(
    scan: &ConflictScan,
    answers: &[Resolution],
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    check_answers(scan, answers)?;
    let scan = scan.clone();
    let answers = answers.to_vec();
    let context = context.clone();
    on_worker(move || {
        preflight(&scan, &context)?;
        let mut run = Run::new(&scan, &answers, &context, progress, None);
        let ran = run
            .free_items()
            .and_then(|cancelled| if cancelled { Ok(()) } else { run.answered_items() });
        run.finish_after(ran)
    })
    .await
}

/// Copies or moves the items of `scan` without a conflict, before the
/// user answers about the others, sending throttled progress to
/// `progress` on the worker thread.
///
/// # Errors
///
/// As [`run_resolved_transfer`].
pub async fn run_free_items(
    scan: &ConflictScan,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<FreeItemsDone, OpsError> {
    let scan = scan.clone();
    let context = context.clone();
    on_worker(move || {
        preflight(&scan, &context)?;
        let mut run = Run::new(&scan, &[], &context, progress, None);
        let ran = run.free_items().map(|_| ());
        run.record_late_error(ran)?;
        Ok(run.into_free_items_done())
    })
    .await
}

/// Goes on with the transfer whose items without a conflict are `done`:
/// the conflicts of `scan` as `answers` says, one per conflict in order.
///
/// # Errors
///
/// As [`run_resolved_transfer`].
pub async fn run_answered_items(
    scan: &ConflictScan,
    answers: &[Resolution],
    done: FreeItemsDone,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    check_answers(scan, answers)?;
    let scan = scan.clone();
    let answers = answers.to_vec();
    let context = context.clone();
    on_worker(move || {
        let mut run = Run::new(&scan, &answers, &context, progress, Some(done));
        let ran = if run.total.result.cancelled {
            Ok(())
        } else {
            run.answered_items()
        };
        run.finish_after(ran)
    })
    .await
}

/// XFER-020 for the whole selection: every selected item, with the item
/// of its name it merges into, is one tree to the write protection, which
/// refuses the transfer before anything changes. The steps then run per
/// folder, which alone would let a free file move before a protected
/// folder merged elsewhere in the same selected tree is refused.
fn preflight(scan: &ConflictScan, context: &OperationContext) -> Result<(), OpsError> {
    let source_change = if scan.mode == TransferMode::Move {
        SourceChange::Changed
    } else {
        SourceChange::Kept
    };
    let destination = GioNode::new(&scan.destination_folder);
    for uri in &scan.selection {
        let source = GioNode::new(uri);
        let target = destination.child(&source.name());
        context
            .protection
            .check_tree(&source, target.as_ref(), &context.cancel, source_change)?;
    }
    Ok(())
}

/// Refuses answers that are not one per conflict.
pub(super) fn check_answers(scan: &ConflictScan, answers: &[Resolution]) -> Result<(), OpsError> {
    if answers.len() == scan.conflicts.len() {
        return Ok(());
    }
    Err(OpsError::failed(crate::i18n::gettext(
        "Answer every name conflict first.",
    )))
}

/// One run of a resolved transfer.
struct Run<'a> {
    scan: &'a ConflictScan,
    answers: &'a [Resolution],
    context: &'a OperationContext,
    engine: TransferEngine,
    /// When the run started, in seconds since the Unix epoch.
    started: u64,
    /// The outcome so far.
    total: TransferOutcome,
    /// Every item that landed, from every step.
    landed: Vec<Landed>,
    /// The replaced files now in the Recycle Bin.
    kept: Vec<PathBuf>,
    /// Where incoming files overwrote files that are gone.
    overwritten: HashSet<String>,
    /// The planned items before the running step, and the step's own,
    /// which place a step's progress in the whole transfer.
    place: Arc<Mutex<(u64, u64)>>,
    /// How many planned items the steps so far handled.
    before: u64,
    /// The merged folders a move emptied and removed.
    removed_folders: Vec<String>,
}

impl<'a> Run<'a> {
    /// A run of `scan` with `answers`, going on from `earlier` when the
    /// items without a conflict are done.
    fn new(
        scan: &'a ConflictScan,
        answers: &'a [Resolution],
        context: &'a OperationContext,
        mut progress: impl FnMut(Progress) + Send + 'static,
        earlier: Option<FreeItemsDone>,
    ) -> Self {
        let all_items = u64::try_from(scan.items.len()).unwrap_or(u64::MAX).max(1);
        // One step's item counts are the transfer's; several steps' are not.
        let several_steps = planned_steps(scan) > 1;
        let place = Arc::new(Mutex::new((0_u64, 0_u64)));
        let step_place = Arc::clone(&place);
        let mut previous_steps = 0_u64;
        let mut current_step = 0_u64;
        let engine = gio_transfer_engine(context, move |mut report: Progress| {
            if let Some(bytes) = report.bytes.as_mut() {
                current_step = bytes.batch_written;
                bytes.batch_written = bytes.batch_written.saturating_add(previous_steps);
                // The steps' sizes are not the whole job's.
                bytes.batch_size = None;
            } else if report.scope == ProgressScope::Batch && report.fraction >= 1.0 {
                previous_steps = previous_steps.saturating_add(current_step);
                current_step = 0;
            }
            // A step's fraction is its own: the transfer is done only when
            // its last step is, so a step's share of the items places it.
            if report.scope == ProgressScope::Batch {
                let (before, items) = *step_place.lock().unwrap_or_else(PoisonError::into_inner);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a progress bar needs far less precision than f64 keeps"
                )]
                let fraction =
                    (before as f64 + report.fraction.clamp(0.0, 1.0) * items as f64) / all_items as f64;
                report.fraction = fraction.min(1.0);
            }
            if several_steps {
                report.items = None;
            }
            progress(report);
        });
        let earlier = earlier.unwrap_or_else(|| FreeItemsDone {
            started: unix_seconds_now(),
            ..FreeItemsDone::default()
        });
        Self {
            scan,
            answers,
            context,
            engine,
            started: earlier.started,
            total: earlier.total,
            landed: earlier.landed,
            kept: Vec::new(),
            overwritten: HashSet::new(),
            place,
            before: earlier.before,
            removed_folders: earlier.removed_folders,
        }
    }

    /// Keeps an error that came after some item was done in the outcome,
    /// which still lists and undoes what was done; an error before
    /// anything was done is the transfer's.
    fn record_late_error(&mut self, ran: Result<(), OpsError>) -> Result<(), OpsError> {
        let Err(error) = ran else {
            return Ok(());
        };
        if self.landed.is_empty() && self.total.result.done.is_empty() {
            return Err(error);
        }
        if error.is_cancelled() {
            self.total.result.cancelled = true;
        } else {
            let folder = GioNode::new(&self.scan.destination_folder).display_name();
            super::results::record_failure(&mut self.total.result, &folder, &error);
        }
        Ok(())
    }

    /// The outcome once the steps ended with `ran`.
    fn finish_after(mut self, ran: Result<(), OpsError>) -> Result<TransferOutcome, OpsError> {
        self.record_late_error(ran)?;
        Ok(self.finish())
    }

    /// The next step handles `items` planned items.
    fn start_step(&mut self, items: usize) {
        let items = u64::try_from(items).unwrap_or(u64::MAX);
        *self.place.lock().unwrap_or_else(PoisonError::into_inner) = (self.before, items);
        self.before = self.before.saturating_add(items);
    }

    /// What the items without a conflict did, to go on with later.
    fn into_free_items_done(self) -> FreeItemsDone {
        FreeItemsDone {
            total: self.total,
            landed: self.landed,
            started: self.started,
            before: self.before,
            removed_folders: self.removed_folders,
        }
    }

    /// The items without a conflict, and copies into their own folder;
    /// true when the user cancelled.
    fn free_items(&mut self) -> Result<bool, OpsError> {
        let scan = self.scan;
        let groups = [
            (PlannedKind::Free, ConflictPolicy::Skip),
            (PlannedKind::OwnFolderCopy, ConflictPolicy::KeepBoth),
        ];
        for (kind, policy) in groups {
            for (folder, uris) in grouped(&scan.items, |item| item.kind == kind) {
                for chunk in uris.chunks(MAX_ITEMS) {
                    let request = TransferRequest {
                        mode: scan.mode,
                        uris: chunk.to_vec(),
                        destination_folder: Some(folder.clone()),
                        policy,
                    };
                    self.start_step(chunk.len());
                    let part = run_on_engine(&mut self.engine, &request, self.context)?;
                    if self.add(part) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// The answered conflicts, the safe answers first, then, for a move,
    /// the merged folders it emptied.
    fn answered_items(&mut self) -> Result<(), OpsError> {
        let scan = self.scan;
        let answered: Vec<(&PlannedItem, &NameConflict, Resolution)> = scan
            .items
            .iter()
            .filter_map(|item| match item.kind {
                PlannedKind::Conflict(index) => Some((item, &scan.conflicts[index], self.answers[index])),
                _ => None,
            })
            .collect();
        for (_, conflict, _) in answered
            .iter()
            .filter(|(_, _, answer)| *answer == Resolution::Skip)
        {
            self.total.result.skipped.push(conflict.incoming_uri.clone());
            self.before = self.before.saturating_add(1);
        }
        for resolution in [Resolution::KeepBoth, Resolution::Replace] {
            for (item, conflict, _) in answered.iter().filter(|(_, _, answer)| *answer == resolution) {
                self.start_step(1);
                let part = match resolution {
                    Resolution::KeepBoth => self.keep_both(item, conflict)?,
                    _ => self.replace(item, conflict)?,
                };
                if self.add(part) {
                    return Ok(());
                }
            }
        }
        if scan.mode == TransferMode::Move {
            self.remove_emptied_folders();
        }
        Ok(())
    }

    /// Adds `part` to the outcome; true when the user cancelled.
    fn add(&mut self, part: TransferOutcome) -> bool {
        self.landed.extend(part.result.landed.iter().cloned());
        merge_results(&mut self.total.result, part.result);
        self.total.result.cancelled
    }

    /// The incoming item of `conflict` goes in under the first free
    /// "name (N)" name.
    fn keep_both(
        &mut self,
        item: &PlannedItem,
        conflict: &NameConflict,
    ) -> Result<TransferOutcome, OpsError> {
        let source = GioNode::new(&item.uri);
        let folder = GioNode::new(&item.destination_folder);
        let Some(name) = source.name().to_str().map(str::to_owned) else {
            return Ok(failed(
                &source.display_name(),
                &OpsError::failed(crate::i18n::gettext(
                    "This item's name is not valid UTF-8, so no number can be added to it. \
                     Rename it before keeping both.",
                )),
            ));
        };
        let kind = if conflict.incoming.is_folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        for number in 2..MAX_KEPT_NUMBER {
            self.context.cancel.check()?;
            let candidate = match kept_both_name(&name, number, kind) {
                Ok(candidate) => candidate,
                Err(error) => return Ok(failed(&source.display_name(), &error.into())),
            };
            if folder
                .child(OsStr::new(&candidate))
                .exists(Some(&self.context.cancel))
            {
                continue;
            }
            return run_renamed_on_engine(
                &mut self.engine,
                self.scan.mode,
                &item.destination_folder,
                &item.uri,
                &candidate,
                self.context,
            );
        }
        Ok(failed(
            &source.display_name(),
            &OpsError::failed(crate::i18n::gettext(
                "Too many numbered names. Rename the item before keeping both.",
            )),
        ))
    }

    /// The incoming item of `conflict` takes the existing one's place;
    /// the replaced file goes to the Recycle Bin first where it can.
    fn replace(&mut self, item: &PlannedItem, conflict: &NameConflict) -> Result<TransferOutcome, OpsError> {
        let request = |policy| TransferRequest {
            mode: self.scan.mode,
            uris: vec![item.uri.clone()],
            destination_folder: Some(item.destination_folder.clone()),
            policy,
        };
        let cancel = &self.context.cancel;
        let existing = GioNode::new(&conflict.existing_uri);
        let path = existing.path();
        let both_files = !conflict.incoming.is_folder
            && !conflict.existing.is_folder
            && existing
                .is_directory(Some(cancel))
                .is_ok_and(|is_folder| !is_folder);
        let keepable = both_files && path.is_some() && existing.can_trash(Some(cancel)).unwrap_or(false);
        let (true, Some(path)) = (keepable, path) else {
            let part = run_on_engine(&mut self.engine, &request(ConflictPolicy::Replace), self.context)?;
            self.overwritten
                .extend(part.result.landed.iter().map(|landed| landed.destination.clone()));
            return Ok(part);
        };
        let context = self.context;
        let skip = request(ConflictPolicy::Skip);
        let engine = &mut self.engine;
        let (part, kept) = replace_through_recycle_bin(
            &existing,
            &path,
            context,
            || run_on_engine(engine, &skip, context),
            |part| part.result.done.contains(&item.uri),
        )?;
        if kept {
            self.kept.push(path);
        }
        Ok(part)
    }

    /// Removes the merged folders a move emptied, the deepest first; one
    /// that still holds an item stays.
    fn remove_emptied_folders(&mut self) {
        for uri in &self.scan.merged_folders {
            let folder = GioNode::new(uri);
            let empty = folder.children(None).is_ok_and(|children| children.is_empty());
            // A folder that cannot go stays; its items moved.
            if empty && self.context.protection.check(uri).is_ok() && folder.delete().is_ok() {
                self.removed_folders.push(uri.clone());
            }
        }
    }

    /// The outcome, with where the items are now and how to undo them:
    /// each selected item that arrived, and each merged folder.
    fn finish(mut self) -> TransferOutcome {
        let destination = GioNode::new(&self.scan.destination_folder);
        let mut created = Vec::new();
        for uri in &self.scan.selection {
            if let Some(landed) = self.landed.iter().find(|landed| &landed.source == uri) {
                created.push(landed.destination.clone());
            } else if self.scan.merged_folders.contains(uri) {
                created.push(destination.child(&GioNode::new(uri).name()).uri());
            }
        }
        self.total.created = created;
        // A selected folder that a move merged and then removed has moved
        // whole, so a cut paste takes it off the clipboard.
        for uri in &self.scan.selection {
            let removed = self.removed_folders.contains(&GioNode::new(uri).uri());
            if removed && !self.total.result.done.contains(uri) {
                self.total.result.done.push(uri.clone());
            }
        }
        self.total.undo = self.undo_record();
        self.total
    }

    /// How Undo reverses the whole run: copies go to the Trash, moved
    /// items go back, then the replaced files come back from the Recycle
    /// Bin.
    fn undo_record(&self) -> Option<UndoRecord> {
        let step = match self.scan.mode {
            TransferMode::Copy => {
                let copies: Vec<String> = self
                    .landed
                    .iter()
                    .map(|landed| landed.destination.clone())
                    .filter(|copy| !self.overwritten.contains(copy))
                    .collect();
                (!copies.is_empty()).then_some(UndoRecord::Copy { copies })
            }
            TransferMode::Move => {
                let items: Vec<MovedItem> = self
                    .landed
                    .iter()
                    .map(|landed| MovedItem {
                        original_uri: landed.source.clone(),
                        moved_uri: landed.destination.clone(),
                    })
                    .collect();
                (!items.is_empty()).then_some(UndoRecord::Move { items })
            }
            TransferMode::Trash | TransferMode::Delete => None,
        };
        // Undo makes the removed folders again first, the deepest last, so
        // the items can go back into them.
        let remade = (!self.removed_folders.is_empty()).then(|| UndoRecord::EmptiedFolders {
            folders: self.removed_folders.iter().rev().cloned().collect(),
        });
        if self.kept.is_empty() && remade.is_none() {
            return step;
        }
        let steps: Vec<UndoRecord> = remade.into_iter().chain(step).collect();
        if steps.is_empty() && self.kept.is_empty() {
            return None;
        }
        Some(UndoRecord::Replace {
            operation: if self.scan.mode == TransferMode::Move {
                super::undo::Operation::Move
            } else {
                super::undo::Operation::Copy
            },
            steps,
            replaced: self.kept.clone(),
            replaced_since: self.started,
        })
    }
}

/// How many engine runs `scan` plans at most: each folder's items without
/// a conflict, each folder's copies into their own folder, and each
/// conflict.
fn planned_steps(scan: &ConflictScan) -> usize {
    let free = grouped(&scan.items, |item| item.kind == PlannedKind::Free);
    let own = grouped(&scan.items, |item| item.kind == PlannedKind::OwnFolderCopy);
    let chunks = |groups: &[(String, Vec<String>)]| {
        groups
            .iter()
            .map(|(_, uris)| uris.len().div_ceil(MAX_ITEMS))
            .sum::<usize>()
    };
    chunks(&free) + chunks(&own) + scan.conflicts.len()
}

/// The items `keep` selects, grouped by the folder they go into.
fn grouped(items: &[PlannedItem], keep: impl Fn(&PlannedItem) -> bool) -> Vec<(String, Vec<String>)> {
    let mut groups = FolderGroups::default();
    for item in items.iter().filter(|item| keep(item)) {
        groups.add(item.destination_folder.clone(), item.uri.clone());
    }
    groups
        .into_groups()
        .into_iter()
        .map(|group| (group.folder_uri, group.uris))
        .collect()
}

/// Replace as the name-conflict answers do it where the desktop's Trash
/// works: the existing item goes to the Recycle Bin, then `place` puts
/// the incoming item at its free name. Should the incoming item not
/// arrive (`arrived` says), the replaced one comes back from the Recycle
/// Bin at once. The step's outcome, and true when the replaced item
/// stays in the Recycle Bin, whose local `path` Undo then restores.
///
/// Copy, move and Restore from the Recycle Bin all replace this way.
///
/// # Errors
///
/// The write protection refusing the existing item, or the Trash
/// request failing before anything changed.
pub(super) fn replace_through_recycle_bin(
    existing: &GioNode,
    path: &Path,
    context: &OperationContext,
    place: impl FnOnce() -> Result<TransferOutcome, OpsError>,
    arrived: impl FnOnce(&TransferOutcome) -> bool,
) -> Result<(TransferOutcome, bool), OpsError> {
    let existing_uri = existing.uri();
    context.protection.check(&existing_uri)?;
    let since = unix_seconds_now();
    let trashed = gio_transfer_engine(context, |_| {}).run(
        Operation::Trash,
        std::slice::from_ref(&existing_uri),
        &context.cancel,
    )?;
    if trashed.done.is_empty() {
        // The existing item could not go: nothing was replaced.
        let mut result = trashed;
        result.skipped.clear();
        let outcome = TransferOutcome {
            result,
            ..TransferOutcome::default()
        };
        return Ok((outcome, false));
    }
    let placed = place();
    if placed.as_ref().is_ok_and(arrived) {
        return placed.map(|part| (part, true));
    }
    // The incoming item did not take the name: the replaced item comes
    // back from the Recycle Bin.
    let mut part = placed.unwrap_or_else(|error| failed(&existing.display_name(), &error));
    let path = path.to_path_buf();
    let restored = restore_trashed_since(std::slice::from_ref(&path), since, context);
    if !restored.is_ok_and(|result| result.errors.is_empty()) {
        let message = crate::i18n::format_message(
            "{name}: The file it was to replace is in the Recycle Bin. Restore it from there.",
            &[("name", &existing.display_name())],
        );
        part.result.recovery.push(message.clone());
        part.result.errors.push(message);
    }
    Ok((part, false))
}

/// An outcome in which the item called `name` failed with `error`.
pub(super) fn failed(name: &str, error: &OpsError) -> TransferOutcome {
    let mut result = TransferResult::default();
    super::results::record_failure(&mut result, name, error);
    TransferOutcome {
        result,
        ..TransferOutcome::default()
    }
}
