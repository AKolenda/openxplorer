// SPDX-License-Identifier: AGPL-3.0-only
//! Carrying out an Undo (OPS-029). Each step reuses the operation that
//! reverses it, with all of that operation's safety rules:
//!
//! - A rename goes back through the rename rules: the write protection
//!   first, then a same-folder move that never overwrites.
//! - Created items and copies go to the Trash through the transfer engine,
//!   which never falls back to a permanent delete (XFER-014).
//! - Moved items go back through a transfer-engine move with the Skip
//!   policy, so an item whose old name is taken again stays where it is
//!   and is reported as skipped.
//! - Trashed items come back through the Recycle Bin's restore, which
//!   never overwrites either.

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::folder_groups::{FolderGroup, FolderGroups};
use super::recycle_bin::restore_trashed_since;
use super::rename::rename_back;
use super::results::{merge_results, record_failure};
use super::run_transfer::gio_transfer_engine;
use super::undo::{MovedItem, UndoRecord};
use crate::gio_node::GioNode;
use crate::transfer::{ConflictPolicy, Node, Operation, Progress, TransferEngine, TransferResult};

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
    let mut engine = gio_transfer_engine(&context.protection, progress);
    let result = match record {
        UndoRecord::Rename {
            original_uri,
            renamed_uri,
        } => undo_rename(original_uri, renamed_uri, context),
        UndoRecord::Create { uri, .. } => move_to_trash(&mut engine, std::slice::from_ref(uri), context),
        UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies } => {
            move_to_trash(&mut engine, copies, context)
        }
        UndoRecord::Restore { restored } => move_to_trash(&mut engine, restored, context),
        UndoRecord::Move { items } => move_back(&mut engine, items, context),
        UndoRecord::Trash {
            original_paths,
            trashed_since,
        } => restore_trashed_since(original_paths, *trashed_since, context)?,
    };
    Ok(result)
}

/// Renames the item back, reporting it as one item.
fn undo_rename(original_uri: &str, renamed_uri: &str, context: &OperationContext) -> TransferResult {
    let mut result = TransferResult::default();
    match rename_back(original_uri, renamed_uri, context) {
        Ok(()) => result.done.push(original_uri.to_owned()),
        Err(error) => record_failure(&mut result, &GioNode::new(renamed_uri).display_name(), &error),
    }
    result
}

/// Moves `uris` to the Trash with the transfer engine.
fn move_to_trash(engine: &mut TransferEngine, uris: &[String], context: &OperationContext) -> TransferResult {
    if uris.is_empty() {
        return TransferResult::default();
    }
    engine
        .run(Operation::Trash, uris, &context.cancel)
        .unwrap_or_else(|error| failed_request(&error.into()))
}

/// Moves each item of `items` back into the folder it came from, one
/// engine run per folder, never overwriting.
fn move_back(engine: &mut TransferEngine, items: &[MovedItem], context: &OperationContext) -> TransferResult {
    let mut total = TransferResult::default();
    for group in group_by_original_folder(items) {
        let operation = Operation::Move {
            destination_folder: &group.folder_uri,
            policy: ConflictPolicy::Skip,
        };
        let part = engine
            .run(operation, &group.uris, &context.cancel)
            .unwrap_or_else(|error| failed_request(&error.into()));
        merge_results(&mut total, part);
        if total.cancelled {
            break;
        }
    }
    total
}

/// The moved URIs of `items`, grouped by the folder each came from.
fn group_by_original_folder(items: &[MovedItem]) -> Vec<FolderGroup> {
    let mut groups = FolderGroups::default();
    for item in items {
        if let Some(folder) = GioNode::new(&item.original_uri).parent() {
            groups.add(folder.uri(), item.moved_uri.clone());
        }
    }
    groups.into_groups()
}

/// A request the engine refused as a whole, as a result with one error.
fn failed_request(error: &OpsError) -> TransferResult {
    let mut result = TransferResult::default();
    record_failure(&mut result, "Undo", error);
    result
}
