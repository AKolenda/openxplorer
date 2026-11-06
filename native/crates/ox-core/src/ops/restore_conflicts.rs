// SPDX-License-Identifier: AGPL-3.0-only
//! Restore from the Recycle Bin onto names that are taken again, as
//! Windows 11 asks about it (OPS-041, OPS-026, OPS-028).
//!
//! Windows restores an item by moving it back into its original folder,
//! so a name taken there meets the same "Replace or Skip Files" question
//! as any move ("Moving 1 item to Desktop", "The destination already has
//! a file named ..."). The plan is a [`ConflictScan`] like a paste's, so
//! the progress window's question and the File Conflict window ask about
//! it unchanged:
//!
//! - Each item goes back into the folder it was deleted from, under its
//!   original name; one whose name is free goes first, before the user
//!   answers, as Windows queues the conflicts to the end of the job.
//! - Skip leaves the item in the Recycle Bin and says nothing more, as
//!   in Windows.
//! - Replace moves the item in the original folder to the Recycle Bin
//!   first, then restores the incoming one, as a paste's Replace does
//!   ([`replace_through_recycle_bin`]); should it not arrive, the
//!   replaced item comes back at once. Where nothing can go to the
//!   Recycle Bin, or a file meets a folder, nothing is replaced and the
//!   item stays in the Recycle Bin, so no restore ever overwrites.
//! - Both kept in the File Conflict window: the restored item takes
//!   Windows' number, `report (2).txt`, the first free one.
//! - A folder meeting a folder of its name is a conflict too: the insides
//!   of a trashed folder are part of that item, so they cannot be merged
//!   one by one.
//!
//! Every step moves the item natively and never overwrites
//! ([`super::recycle_bin`]), and Undo puts the restored items back in
//! the Recycle Bin, then restores the replaced ones.

use std::path::PathBuf;

use gio::prelude::*;

use super::conflicts::{facts, ConflictScan, NameConflict, PlannedItem, PlannedKind};
use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::recycle_bin::{
    forget_removed_items, move_out, path_name, recreate_original_folder, restore_item, top_level_item,
};
use super::resolved_transfer::{
    check_answers, failed, replace_through_recycle_bin, Resolution, MAX_KEPT_NUMBER,
};
use super::results::{merge_results, record_failure};
use super::run_transfer::{unix_seconds_now, TransferOutcome};
use super::undo::{Operation, UndoRecord};
use crate::entry::{entry_from_info, ATTRIBUTES};
use crate::gio_node::GioNode;
use crate::location::{kept_both_name, ItemKind};
use crate::transfer::{Cancellation, Node, TransferMode};

/// The plan of restoring the Recycle Bin items `uris`: each item goes
/// into the folder it was deleted from, and each one whose original name
/// is taken there is a conflict to ask about. An item the Recycle Bin
/// cannot place is planned as free, and its restore says why it failed.
///
/// The plan's `destination_folder` is the folder of its first conflict,
/// or of its first item; [`restores_into_one_folder`] tells whether every
/// item goes there.
///
/// # Errors
///
/// Only cancellation.
pub async fn scan_restore_conflicts(
    uris: &[String],
    cancel: &Cancellation,
) -> Result<ConflictScan, OpsError> {
    let uris = uris.to_vec();
    let cancel = cancel.clone();
    on_worker(move || scan_blocking(&uris, &cancel)).await
}

/// True when every item of the restore `scan` goes into its
/// `destination_folder`.
pub fn restores_into_one_folder(scan: &ConflictScan) -> bool {
    scan.items
        .iter()
        .all(|item| item.destination_folder == scan.destination_folder)
}

/// The questions the restore `scan` asks, in order: one per original
/// folder that has conflicts, each a plan of that folder's conflicts
/// alone, with where they are in `scan.conflicts`. Each question and its
/// File Conflict window name one destination, as Windows' do, so
/// conflicts in several folders are asked about folder by folder.
pub fn restore_questions(scan: &ConflictScan) -> Vec<(ConflictScan, Vec<usize>)> {
    let mut questions: Vec<(ConflictScan, Vec<usize>)> = Vec::new();
    for item in &scan.items {
        let PlannedKind::Conflict(index) = item.kind else {
            continue;
        };
        let asked = questions
            .iter()
            .position(|(question, _)| question.destination_folder == item.destination_folder);
        let at = asked.unwrap_or_else(|| {
            let question = ConflictScan {
                mode: scan.mode,
                destination_folder: item.destination_folder.clone(),
                selection: Vec::new(),
                items: Vec::new(),
                conflicts: Vec::new(),
                merged_folders: Vec::new(),
            };
            questions.push((question, Vec::new()));
            questions.len() - 1
        });
        let (question, indices) = &mut questions[at];
        question.selection.push(item.uri.clone());
        question.items.push(PlannedItem {
            uri: item.uri.clone(),
            destination_folder: item.destination_folder.clone(),
            kind: PlannedKind::Conflict(question.conflicts.len()),
        });
        question.conflicts.push(scan.conflicts[index].clone());
        indices.push(index);
    }
    questions
}

/// [`scan_restore_conflicts`] on the calling thread.
fn scan_blocking(uris: &[String], cancel: &Cancellation) -> Result<ConflictScan, OpsError> {
    let mut scan = ConflictScan {
        mode: TransferMode::Move,
        destination_folder: String::new(),
        selection: uris.to_vec(),
        items: Vec::new(),
        conflicts: Vec::new(),
        merged_folders: Vec::new(),
    };
    for uri in uris {
        cancel.check()?;
        let (destination_folder, conflict) = match plan_item(uri, cancel) {
            Ok(planned) => planned,
            Err(error) if error.is_cancelled() => return Err(error),
            // The restore itself reports it.
            Err(_) => (String::new(), None),
        };
        let kind = match conflict {
            Some(conflict) => {
                scan.conflicts.push(conflict);
                PlannedKind::Conflict(scan.conflicts.len() - 1)
            }
            None => PlannedKind::Free,
        };
        scan.items.push(PlannedItem {
            uri: uri.clone(),
            destination_folder,
            kind,
        });
    }
    let first_conflict = scan
        .items
        .iter()
        .find(|item| matches!(item.kind, PlannedKind::Conflict(_)));
    if let Some(item) = first_conflict.or_else(|| scan.items.first()) {
        scan.destination_folder = item.destination_folder.clone();
    }
    Ok(scan)
}

/// The folder the Recycle Bin item `uri` goes back into, and its
/// conflict when its original name is taken there.
fn plan_item(uri: &str, cancel: &Cancellation) -> Result<(String, Option<NameConflict>), OpsError> {
    let item = top_level_item(uri)?;
    let original_path = original_path(&item, cancel)?;
    let target = gio::File::for_path(&original_path);
    let folder = target
        .parent()
        .map(|parent| parent.uri().to_string())
        .unwrap_or_default();
    let existing = GioNode::from_file(target);
    if !existing.exists(Some(cancel)) {
        cancel.check()?;
        return Ok((folder, None));
    }
    let conflict = NameConflict {
        incoming_uri: uri.to_owned(),
        existing_uri: existing.uri(),
        incoming: facts(&GioNode::new(uri), cancel)?,
        existing: facts(&existing, cancel)?,
    };
    Ok((folder, Some(conflict)))
}

/// Where the Recycle Bin item `item` was deleted from.
fn original_path(item: &gio::File, cancel: &Cancellation) -> Result<PathBuf, OpsError> {
    let info = item.query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    entry_from_info(item, &info)
        .trash_orig_path
        .ok_or_else(|| OpsError::failed(String::new()))
}

/// What a restore with conflicts did so far: the items whose names were
/// free, restored while the user has not answered yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreSoFar {
    /// Its outcome so far.
    outcome: TransferOutcome,
    /// The local paths of the replaced items, now in the Recycle Bin.
    replaced: Vec<PathBuf>,
    /// When the restore started, in seconds since the Unix epoch.
    started: u64,
}

impl RestoreSoFar {
    /// True when the user cancelled while the free items were restored.
    pub fn cancelled(&self) -> bool {
        self.outcome.result.cancelled
    }

    /// The outcome of the restore stopped at the question: only these
    /// items count, and it reads as cancelled, so nothing more is said.
    pub fn into_outcome(mut self) -> TransferOutcome {
        self.outcome.result.cancelled = true;
        self.finish()
    }

    /// Adds the restore of the item `uri` to `restored`, or its failure.
    fn add(&mut self, uri: &str, restored: Result<String, OpsError>, name: &str) {
        match restored {
            Ok(restored) => {
                self.outcome.result.done.push(uri.to_owned());
                self.outcome.created.push(restored);
            }
            Err(error) => record_failure(&mut self.outcome.result, name, &error),
        }
    }

    /// The outcome, with how Undo reverses it: the restored items go to
    /// the Trash again, then the items they replaced come back.
    fn finish(mut self) -> TransferOutcome {
        if self.outcome.created.is_empty() {
            return self.outcome;
        }
        forget_removed_items();
        let restored = UndoRecord::Restore {
            restored: self.outcome.created.clone(),
        };
        self.outcome.undo = Some(if self.replaced.is_empty() {
            restored
        } else {
            UndoRecord::Replace {
                operation: Operation::Restore,
                steps: vec![restored],
                replaced: self.replaced,
                replaced_since: self.started,
            }
        });
        self.outcome
    }
}

/// Restores the items of the restore `scan` whose names are free, before
/// the user answers about the others.
///
/// # Errors
///
/// None for the whole request; each item's failure is in the outcome.
pub async fn restore_free_items(
    scan: &ConflictScan,
    context: &OperationContext,
) -> Result<RestoreSoFar, OpsError> {
    let scan = scan.clone();
    let context = context.clone();
    on_worker(move || {
        let mut so_far = RestoreSoFar {
            started: unix_seconds_now(),
            ..RestoreSoFar::default()
        };
        for item in scan.items.iter().filter(|item| item.kind == PlannedKind::Free) {
            let restored = restore_item(&item.uri, &context);
            so_far.add(&item.uri, restored, &GioNode::new(&item.uri).display_name());
            if so_far.cancelled() {
                break;
            }
        }
        Ok(so_far)
    })
    .await
}

/// Goes on with the restore whose free items are `so_far`: the
/// conflicts of `scan` as `answers` says, one per conflict in order,
/// Keep both before Replace. A skipped item stays in the Recycle Bin and
/// is not reported, as in Windows.
///
/// # Errors
///
/// Answers that do not match the conflicts.
pub async fn restore_answered_items(
    scan: &ConflictScan,
    answers: &[Resolution],
    so_far: RestoreSoFar,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    check_answers(scan, answers)?;
    let scan = scan.clone();
    let answers = answers.to_vec();
    let context = context.clone();
    on_worker(move || {
        let mut so_far = so_far;
        if so_far.cancelled() {
            return Ok(so_far.finish());
        }
        'answers: for resolution in [Resolution::KeepBoth, Resolution::Replace] {
            for item in &scan.items {
                let PlannedKind::Conflict(index) = item.kind else {
                    continue;
                };
                if answers[index] != resolution {
                    continue;
                }
                let conflict = &scan.conflicts[index];
                let name = conflict.name();
                if context.cancel.is_cancelled() {
                    so_far.outcome.result.cancelled = true;
                    break 'answers;
                }
                match resolution {
                    Resolution::KeepBoth => {
                        let restored = keep_both(item, conflict, &context);
                        so_far.add(&item.uri, restored, &name);
                    }
                    _ => replace(item, conflict, &context, &mut so_far),
                }
                if so_far.cancelled() {
                    break 'answers;
                }
            }
        }
        Ok(so_far.finish())
    })
    .await
}

/// Restores the item of `conflict` under the first free "name (N)" name
/// in its original folder.
fn keep_both(
    item: &PlannedItem,
    conflict: &NameConflict,
    context: &OperationContext,
) -> Result<String, OpsError> {
    let trashed = top_level_item(&item.uri)?;
    let existing = gio::File::for_uri(&conflict.existing_uri);
    let folder = existing.parent().ok_or_else(|| OpsError::failed(String::new()))?;
    let name = existing
        .path()
        .map_or_else(|| conflict.name(), |path| path_name(&path));
    let kind = if conflict.incoming.is_folder {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    for number in 2..MAX_KEPT_NUMBER {
        context.cancel.check()?;
        let candidate = kept_both_name(&name, number, kind)?;
        let target = folder.child(&candidate);
        if GioNode::from_file(target.clone()).exists(Some(&context.cancel)) {
            continue;
        }
        let taken = || taken_meanwhile(&candidate);
        return move_out(&trashed, &target, context, taken, |target| {
            recreate_original_folder(target, context)
        });
    }
    Err(OpsError::failed(crate::i18n::gettext(
        "Too many numbered names. Rename the item before keeping both.",
    )))
}

/// The item of `conflict` takes the existing item's place, which goes to
/// the Recycle Bin first; nothing is replaced where it cannot.
///
/// The original folder is read again first, as it is now: the question
/// may have shown for a long time.
///
/// - A name freed meanwhile takes the item as any free name does.
/// - An item there of the other kind now (a folder where a file was, or
///   the other way round) is not replaced.
/// - A name this restore already replaced holds an item it restored, so
///   a second item from the same place stays in the Recycle Bin rather
///   than sending the first one back there; Undo then has one item to
///   take back and one replaced item to bring back.
fn replace(
    item: &PlannedItem,
    conflict: &NameConflict,
    context: &OperationContext,
    so_far: &mut RestoreSoFar,
) {
    let name = conflict.name();
    let existing = GioNode::new(&conflict.existing_uri);
    let cancel = &context.cancel;
    if !existing.exists(Some(cancel)) {
        let restored = restore_item(&item.uri, context);
        so_far.add(&item.uri, restored, &name);
        return;
    }
    if existing
        .path()
        .is_some_and(|path| so_far.replaced.contains(&path))
    {
        record_failure(&mut so_far.outcome.result, &name, &taken_meanwhile(&name));
        return;
    }
    let same_kind = facts(&existing, cancel).is_ok_and(|now| {
        now.is_folder == conflict.incoming.is_folder && now.is_folder == conflict.existing.is_folder
    });
    let kept = existing
        .path()
        .filter(|_| same_kind && existing.can_trash(Some(cancel)).unwrap_or(false));
    let Some(path) = kept else {
        let refusal = OpsError::failed(crate::i18n::format_message(
            "The item named “{name}” in its original folder cannot be replaced. It was left in the Recycle Bin.",
            &[("name", &name)],
        ));
        record_failure(&mut so_far.outcome.result, &name, &refusal);
        return;
    };
    let place = || {
        let mut part = TransferOutcome::default();
        match restore_item(&item.uri, context) {
            Ok(restored) => {
                part.result.done.push(item.uri.clone());
                part.created.push(restored);
                Ok(part)
            }
            Err(error) => Ok(failed(&name, &error)),
        }
    };
    let arrived = |part: &TransferOutcome| part.result.done.contains(&item.uri);
    match replace_through_recycle_bin(&existing, &path, context, place, arrived) {
        Ok((part, replaced)) => {
            if replaced {
                so_far.replaced.push(path);
            }
            so_far.outcome.created.extend(part.created);
            merge_results(&mut so_far.outcome.result, part.result);
        }
        Err(error) => record_failure(&mut so_far.outcome.result, &name, &error),
    }
}

/// The refusal when the kept-both name `name` was taken after it was
/// found free.
fn taken_meanwhile(name: &str) -> OpsError {
    OpsError::Exists(crate::i18n::format_message(
        "An item named “{name}” already exists in its original folder. It was left in the Recycle Bin.",
        &[("name", name)],
    ))
}
