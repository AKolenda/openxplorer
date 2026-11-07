// SPDX-License-Identifier: AGPL-3.0-only
//! Carrying out an Undo (OPS-029). Each step reuses the operation that
//! reverses it, with all of that operation's safety rules:
//!
//! - A rename goes back through the rename rules: the write protection
//!   first, then a same-folder move that never overwrites. A batch rename
//!   goes back item by item, the last renamed first.
//! - Links go to the Trash as links; what they point to is never
//!   touched.
//! - Created items and copies go to the Trash through the transfer engine,
//!   which never falls back to a permanent delete (XFER-014).
//! - Moved items go back through a transfer-engine move with the Skip
//!   policy, under the name they had before the move (a Keep both or a
//!   name made valid for the destination is taken back too), so an item
//!   whose old name is taken again stays where it is and is reported as
//!   skipped.
//! - Trashed items come back through the Recycle Bin's restore, which
//!   never overwrites either.
//! - A copy or move that replaced files is undone step by step, then the
//!   replaced files come back from the Recycle Bin, once the steps took
//!   their replacements away.
//! - A moved item goes back into its folder even when the move removed
//!   that folder, as a merging move does once it emptied it: the folder
//!   is made again first.

use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::folder_groups::{FolderGroup, FolderGroups};
use super::recycle_bin::{find_trashed_since, restore_found, restore_trashed_since};
use super::rename::rename_back;
use super::results::{merge_results, AskedItem, ItemQuestions};
use super::run_transfer::{gio_transfer_engine, unix_seconds_now};
use super::undo::{MovedItem, RenamedPair, UndoRecord};
use crate::gio_node::GioNode;
use crate::transfer::{
    ConflictPolicy, Node, Operation, Progress, TransferEngine, TransferMode, TransferResult,
};

/// What reversing a journal step did, and how to take that back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reversal {
    /// Every item's outcome.
    pub result: TransferResult,
    /// The record that takes this reversal back: Redo after an Undo, Undo
    /// after a Redo. `None` when nothing was reversed.
    pub inverse: Option<UndoRecord>,
}

/// Reverses `record`, as Undo or Redo does, and works out how to take the
/// reversal back. Take the step from the
/// [`UndoJournal`](super::UndoJournal) first, and record the inverse
/// there afterwards.
///
/// # Errors
///
/// As [`undo`].
pub async fn reverse(
    record: &UndoRecord,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<Reversal, OpsError> {
    // Taken before anything moves, so a Redo restores exactly the items
    // this reversal moves to the Trash.
    let reversed_since = unix_seconds_now();
    if let UndoRecord::Replace { .. } = record {
        let record = record.clone();
        let context = context.clone();
        return on_worker(move || {
            let (steps, restored) = undo_replace(&record, &context, Box::new(progress))?;
            let UndoRecord::Replace { operation, .. } = record else {
                unreachable!("only a Replace record is reversed here");
            };
            let mut result = TransferResult::default();
            for (_, part) in &steps {
                merge_results(&mut result, part.clone());
            }
            merge_results(&mut result, restored.clone());
            let inverse = UndoRecord::replace_inverse(operation, &steps, &restored, reversed_since);
            Ok(Reversal { result, inverse })
        })
        .await;
    }
    let result = undo(record, context, progress).await?;
    let inverse = record.inverse(&result, reversed_since);
    Ok(Reversal { result, inverse })
}

/// Undoes the steps of the Replace `record`, then restores its replaced
/// files: each step with its result, and the restore's result.
fn undo_replace(
    record: &UndoRecord,
    context: &OperationContext,
    progress: Box<dyn FnMut(Progress) + Send>,
) -> Result<(Vec<(UndoRecord, TransferResult)>, TransferResult), OpsError> {
    let UndoRecord::Replace {
        steps,
        replaced,
        replaced_since,
        ..
    } = record
    else {
        return Ok((Vec::new(), TransferResult::default()));
    };
    // Looked up first: the steps may move other items from these paths
    // to the Trash.
    let found = if replaced.is_empty() {
        Vec::new()
    } else {
        find_trashed_since(replaced, *replaced_since, &context.cancel)?
    };
    let progress = Arc::new(Mutex::new(progress));
    let mut done = Vec::new();
    for step in steps {
        let sink = Arc::clone(&progress);
        let result = undo_blocking(step, context, move |report| {
            if let Ok(mut progress) = sink.lock() {
                progress(report);
            }
        })?;
        let cancelled = result.cancelled;
        done.push((step.clone(), result));
        if cancelled {
            return Ok((done, TransferResult::default()));
        }
    }
    let restored = if replaced.is_empty() {
        TransferResult::default()
    } else {
        restore_found(replaced, &found, context)
    };
    Ok((done, restored))
}

/// Reverses the operation `record` describes, sending throttled progress
/// to `progress` on the worker thread. Take the record from the
/// [`UndoJournal`](super::UndoJournal) first.
///
/// # Errors
///
/// When the Recycle Bin cannot be listed to undo Move to Trash. Every
/// other failure is reported per item in the result.
pub async fn undo(
    record: &UndoRecord,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferResult, OpsError> {
    let record = record.clone();
    let context = context.clone();
    on_worker(move || undo_blocking(&record, &context, progress)).await
}

/// [`undo`] on the calling thread.
fn undo_blocking(
    record: &UndoRecord,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferResult, OpsError> {
    let mut engine = gio_transfer_engine(context, progress);
    let result = match record {
        UndoRecord::Rename {
            original_uri,
            renamed_uri,
        } => undo_rename(original_uri, renamed_uri, context),
        UndoRecord::BatchRename { items } => undo_batch_rename(items, context),
        UndoRecord::Create { uri, .. } => move_to_trash(&mut engine, std::slice::from_ref(uri), context),
        UndoRecord::Link { links } => move_to_trash(&mut engine, links, context),
        UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies } => {
            move_to_trash(&mut engine, copies, context)
        }
        UndoRecord::Restore { restored } => move_to_trash(&mut engine, restored, context),
        UndoRecord::Move { items } => move_back(&mut engine, items, context),
        UndoRecord::Trash {
            original_paths,
            trashed_since,
        } => restore_trashed_since(original_paths, *trashed_since, context)?,
        UndoRecord::EmptiedFolders { folders } => make_folders_again(folders, context),
        UndoRecord::RemadeFolders { folders } => remove_empty_folders(folders, context),
        UndoRecord::Replace { .. } => {
            drop(engine);
            let (steps, restored) = undo_replace(record, context, Box::new(|_| {}))?;
            let mut total = TransferResult::default();
            for (_, part) in steps {
                merge_results(&mut total, part);
            }
            merge_results(&mut total, restored);
            total
        }
    };
    Ok(result)
}

/// Renames the item back, as one item; a failure is asked about in the
/// progress window (OPS-047).
fn undo_rename(original_uri: &str, renamed_uri: &str, context: &OperationContext) -> TransferResult {
    let pair = RenamedPair {
        original_uri: original_uri.to_owned(),
        renamed_uri: renamed_uri.to_owned(),
    };
    undo_batch_rename(std::slice::from_ref(&pair), context)
}

/// Renames each of `items` back, the last renamed first, so a name the
/// batch freed is free again when an earlier item takes it back. Each
/// failure is asked about in the progress window.
fn undo_batch_rename(items: &[RenamedPair], context: &OperationContext) -> TransferResult {
    let mut total = TransferResult::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    for (index, item) in items.iter().rev().enumerate() {
        if context.cancel.is_cancelled() {
            total.cancelled = true;
            break;
        }
        let name = GioNode::new(&item.renamed_uri).display_name();
        let asked = AskedItem {
            name: &name,
            uri: &item.renamed_uri,
            more_items: index + 1 < items.len(),
        };
        let renamed = questions.run(&mut total, asked, || {
            rename_back(&item.original_uri, &item.renamed_uri, context)
        });
        if renamed.is_some() {
            total.done.push(item.original_uri.clone());
        }
        if total.cancelled {
            break;
        }
    }
    total
}

/// Moves `uris` to the Trash with the transfer engine.
fn move_to_trash(engine: &mut TransferEngine, uris: &[String], context: &OperationContext) -> TransferResult {
    if uris.is_empty() {
        return TransferResult::default();
    }
    engine
        .run(Operation::Trash, uris, &context.cancel)
        .unwrap_or_else(|error| failed_request(&error.into(), context))
}

/// Moves each item of `items` back into the folder it came from, never
/// overwriting: one engine run per folder for the items that kept their
/// name, and one renamed run per item that landed under another name.
fn move_back(engine: &mut TransferEngine, items: &[MovedItem], context: &OperationContext) -> TransferResult {
    let mut total = TransferResult::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    let (kept_name, renamed): (Vec<&MovedItem>, Vec<&MovedItem>) =
        items.iter().partition(|item| moved_name(item).is_none());
    for group in group_by_original_folder(&kept_name) {
        // The folder the items go back into, asked about as one item.
        let folder = AskedItem {
            name: &GioNode::new(&group.folder_uri).display_name(),
            uri: &group.folder_uri,
            more_items: true,
        };
        if questions
            .run(&mut total, folder, || recreate_folder(&group.folder_uri, context))
            .is_none()
        {
            if total.cancelled {
                return total;
            }
            continue;
        }
        let operation = Operation::Move {
            destination_folder: &group.folder_uri,
            policy: ConflictPolicy::Skip,
        };
        let part = engine
            .run(operation, &group.uris, &context.cancel)
            .unwrap_or_else(|error| failed_request(&error.into(), context));
        merge_results(&mut total, part);
        if total.cancelled {
            return total;
        }
    }
    for item in renamed {
        let original = GioNode::new(&item.original_uri);
        let Some(folder) = original.parent() else {
            continue;
        };
        let asked = AskedItem {
            name: &GioNode::new(&item.moved_uri).display_name(),
            uri: &item.moved_uri,
            more_items: true,
        };
        if questions
            .run(&mut total, asked, || recreate_folder(&folder.uri(), context))
            .is_none()
        {
            if total.cancelled {
                break;
            }
            continue;
        }
        let part = engine
            .run_renamed(
                TransferMode::Move,
                &folder.uri(),
                &item.moved_uri,
                &original.name(),
                &context.cancel,
            )
            .unwrap_or_else(|error| failed_request(&error.into(), context));
        merge_results(&mut total, part);
        if total.cancelled {
            break;
        }
    }
    total
}

/// Makes the folder at `uri` again, with the folders it is in, when a
/// merging move removed it after emptying it; an existing folder is left
/// as it is.
fn recreate_folder(uri: &str, context: &OperationContext) -> Result<(), OpsError> {
    let folder = gio::File::for_uri(uri);
    if folder.query_exists(gio::Cancellable::NONE) {
        return Ok(());
    }
    context.protection.check(uri)?;
    match folder.make_directory_with_parents(Some(context.cancellable())) {
        Err(error) if !error.matches(gio::IOErrorEnum::Exists) => Err(OpsError::failed(error.to_string())),
        _ => Ok(()),
    }
}

/// Makes each of `folders` again, the outer ones first, where nothing has
/// its name; each made is done. A folder that is there already needs
/// nothing and is skipped without a word; a failure is asked about in the
/// progress window.
fn make_folders_again(folders: &[String], context: &OperationContext) -> TransferResult {
    let mut result = TransferResult::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    for (index, uri) in folders.iter().enumerate() {
        let folder = gio::File::for_uri(uri);
        if folder.query_exists(gio::Cancellable::NONE) {
            result.skipped.push(uri.clone());
            continue;
        }
        let asked = AskedItem {
            name: &GioNode::new(uri).display_name(),
            uri,
            more_items: index + 1 < folders.len(),
        };
        if questions
            .run(&mut result, asked, || recreate_folder(uri, context))
            .is_some()
        {
            result.done.push(uri.clone());
        }
        if result.cancelled {
            break;
        }
    }
    result
}

/// Removes each of `folders` again, the inner ones first, while it is
/// still empty. One that holds an item now stays, without a word: the
/// user put something there, and the move being redone never removed a
/// folder that held anything. A failure is asked about in the progress
/// window.
fn remove_empty_folders(folders: &[String], context: &OperationContext) -> TransferResult {
    let mut result = TransferResult::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Delete);
    for (index, uri) in folders.iter().enumerate() {
        let folder = GioNode::new(uri);
        let empty = folder.children(None).is_ok_and(|children| children.is_empty());
        if !empty {
            result.skipped.push(uri.clone());
            continue;
        }
        let asked = AskedItem {
            name: &folder.display_name(),
            uri,
            more_items: index + 1 < folders.len(),
        };
        let remove = || {
            context
                .protection
                .check(uri)
                .and_then(|()| folder.delete().map_err(OpsError::from))
        };
        if questions.run(&mut result, asked, remove).is_some() {
            result.done.push(uri.clone());
        }
        if result.cancelled {
            break;
        }
    }
    result
}

/// The name `item` had before the move, when it landed under another one.
fn moved_name(item: &MovedItem) -> Option<OsString> {
    let original = GioNode::new(&item.original_uri).name();
    (GioNode::new(&item.moved_uri).name() != original).then_some(original)
}

/// The moved URIs of `items`, grouped by the folder each came from.
fn group_by_original_folder(items: &[&MovedItem]) -> Vec<FolderGroup> {
    let mut groups = FolderGroups::default();
    for item in items {
        if let Some(folder) = GioNode::new(&item.original_uri).parent() {
            groups.add(folder.uri(), item.moved_uri.clone());
        }
    }
    groups.into_groups()
}

/// A request the engine refused as a whole, as a result with one error,
/// told in the progress window.
fn failed_request(error: &OpsError, context: &OperationContext) -> TransferResult {
    let mut result = TransferResult::default();
    let undo = crate::i18n::gettext("Undo");
    let asked = AskedItem {
        name: &undo,
        uri: "",
        more_items: false,
    };
    ItemQuestions::new(context, TransferMode::Move).tell(&mut result, asked, error);
    result
}
