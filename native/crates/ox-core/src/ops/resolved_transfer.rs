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
//! A move removes each merged folder it emptied, as Windows does; one
//! still holding a skipped file stays. The groups run the safe answers
//! first; a cancellation stops the steps that have not started.
//!
//! [`scan_name_conflicts`]: super::scan_name_conflicts

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::PathBuf;

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
use crate::transfer::{
    ConflictPolicy, Landed, Node, Operation, Progress, ProgressScope, TransferEngine, TransferMode,
    TransferResult, MAX_ITEMS,
};

/// The most numbers tried for a kept-both name.
const MAX_KEPT_NUMBER: u32 = 10_000;

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

/// Runs `scan` with `answers`, one per conflict in order, sending
/// throttled progress to `progress` on the worker thread. The outcome
/// adds up every step, and its Undo reverses the whole paste as one step.
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
    if answers.len() != scan.conflicts.len() {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Answer every name conflict first.",
        )));
    }
    let scan = scan.clone();
    let answers = answers.to_vec();
    let context = context.clone();
    on_worker(move || Run::new(&scan, &answers, &context, progress).run()).await
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
}

impl<'a> Run<'a> {
    fn new(
        scan: &'a ConflictScan,
        answers: &'a [Resolution],
        context: &'a OperationContext,
        mut progress: impl FnMut(Progress) + Send + 'static,
    ) -> Self {
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
            progress(report);
        });
        Self {
            scan,
            answers,
            context,
            engine,
            started: unix_seconds_now(),
            total: TransferOutcome::default(),
            landed: Vec::new(),
            kept: Vec::new(),
            overwritten: HashSet::new(),
        }
    }

    /// Every step, safe answers first, then the outcome.
    fn run(mut self) -> Result<TransferOutcome, OpsError> {
        let scan = self.scan;
        let mode = scan.mode;
        let groups = [
            (PlannedKind::Free, ConflictPolicy::Skip),
            (PlannedKind::OwnFolderCopy, ConflictPolicy::KeepBoth),
        ];
        for (kind, policy) in groups {
            for (folder, uris) in grouped(&scan.items, |item| item.kind == kind) {
                for chunk in uris.chunks(MAX_ITEMS) {
                    let request = TransferRequest {
                        mode,
                        uris: chunk.to_vec(),
                        destination_folder: Some(folder.clone()),
                        policy,
                    };
                    let part = run_on_engine(&mut self.engine, &request, self.context)?;
                    if self.add(part) {
                        return Ok(self.finish());
                    }
                }
            }
        }
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
        }
        for resolution in [Resolution::KeepBoth, Resolution::Replace] {
            for (item, conflict, _) in answered.iter().filter(|(_, _, answer)| *answer == resolution) {
                let part = match resolution {
                    Resolution::KeepBoth => self.keep_both(item, conflict)?,
                    _ => self.replace(item, conflict)?,
                };
                if self.add(part) {
                    return Ok(self.finish());
                }
            }
        }
        if mode == TransferMode::Move {
            self.remove_emptied_folders();
        }
        Ok(self.finish())
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
            let candidate = kept_both_name(&name, number, kind)?;
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
        self.context.protection.check(&conflict.existing_uri)?;
        let since = unix_seconds_now();
        let trashed = gio_transfer_engine(self.context, |_| {}).run(
            Operation::Trash,
            std::slice::from_ref(&conflict.existing_uri),
            cancel,
        )?;
        if trashed.done.is_empty() {
            // The existing file could not go: nothing was replaced.
            let mut result = trashed;
            result.skipped.clear();
            return Ok(TransferOutcome {
                result,
                ..TransferOutcome::default()
            });
        }
        let placed = run_on_engine(&mut self.engine, &request(ConflictPolicy::Skip), self.context);
        let arrived = placed
            .as_ref()
            .is_ok_and(|part| part.result.done.contains(&item.uri));
        if arrived {
            self.kept.push(path);
            return placed;
        }
        // The incoming file did not take the name: the replaced file comes
        // back from the Recycle Bin.
        let mut part = placed.unwrap_or_else(|error| failed(&existing.display_name(), &error));
        let restored = restore_trashed_since(std::slice::from_ref(&path), since, self.context);
        if !restored.is_ok_and(|result| result.errors.is_empty()) {
            let message = crate::i18n::format_message(
                "{name}: The file it was to replace is in the Recycle Bin. Restore it from there.",
                &[("name", &existing.display_name())],
            );
            part.result.recovery.push(message.clone());
            part.result.errors.push(message);
        }
        Ok(part)
    }

    /// Removes the merged folders a move emptied, the deepest first; one
    /// that still holds an item stays.
    fn remove_emptied_folders(&mut self) {
        for uri in &self.scan.merged_folders {
            let folder = GioNode::new(uri);
            let empty = folder.children(None).is_ok_and(|children| children.is_empty());
            if empty && self.context.protection.check(uri).is_ok() {
                // A folder that cannot go stays; its items moved.
                let _ = folder.delete();
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
        if self.kept.is_empty() {
            return step;
        }
        Some(UndoRecord::Replace {
            operation: if self.scan.mode == TransferMode::Move {
                super::undo::Operation::Move
            } else {
                super::undo::Operation::Copy
            },
            steps: step.into_iter().collect(),
            replaced: self.kept.clone(),
            replaced_since: self.started,
        })
    }
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

/// An outcome in which the item called `name` failed with `error`.
fn failed(name: &str, error: &OpsError) -> TransferOutcome {
    let mut result = TransferResult::default();
    super::results::record_failure(&mut result, name, error);
    TransferOutcome {
        result,
        ..TransferOutcome::default()
    }
}
