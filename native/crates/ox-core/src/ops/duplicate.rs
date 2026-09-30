// SPDX-License-Identifier: AGPL-3.0-only
//! Duplicate: a copy of each selected item next to itself.
//!
//! New in the native app, from the Dolphin baseline ("Duplicate Here"). It
//! is a copy into the item's own folder with the Keep both policy, so it
//! has every safety rule of a copy (private staging, publishing that never
//! overwrites, the write protection) and the app's own duplicate names:
//! `report (copy 2).pdf`, then `(copy 3)`, as Keep both names them
//! (XFER-008). Items from several folders, as a search can select, are
//! copied into their own folders one folder at a time.
//!
//! Dolphin names a duplicate `report copy.pdf`; OPS-034 in
//! `native/parity/features.toml` records that Duplicate keeps the app's
//! own `(copy N)` names instead, so a duplicate and a Keep both copy are
//! named alike.

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::folder_groups::{FolderGroup, FolderGroups};
use super::results::{merge_results, record_failure};
use super::run_transfer::{gio_transfer_engine, TransferOutcome};
use super::undo::UndoRecord;
use crate::gio_node::GioNode;
use crate::location::require_item_uri;
use crate::transfer::{ConflictPolicy, Node, Operation, Progress, TransferEngine, MAX_ITEMS};

/// Duplicates each of `uris` in its own folder, sending throttled progress
/// to `progress` on the worker thread. The outcome lists the duplicates,
/// to select them, and Undo moves them to the Trash.
///
/// # Errors
///
/// No items or more than [`MAX_ITEMS`], a share, device or filesystem
/// root among them, or a protected folder. Failures of single items and of
/// single folders, and the user's cancellation, are reported in
/// [`TransferOutcome::result`].
pub async fn duplicate_items(
    uris: &[String],
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    let uris = uris.to_vec();
    let context = context.clone();
    on_worker(move || duplicate_items_blocking(&uris, &context, progress)).await
}

/// [`duplicate_items`] on the calling thread.
fn duplicate_items_blocking(
    uris: &[String],
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    if uris.is_empty() || uris.len() > MAX_ITEMS {
        return Err(OpsError::failed("Select between 1 and 100,000 items."));
    }
    let groups = group_by_folder(uris)?;
    for group in &groups {
        context.protection.check(&group.folder_uri)?;
    }
    let mut engine = gio_transfer_engine(context, progress);
    let mut outcome = TransferOutcome::default();
    for group in &groups {
        duplicate_in_folder(&mut engine, group, context, &mut outcome);
        if outcome.result.cancelled {
            break;
        }
    }
    if !outcome.created.is_empty() {
        let copies = outcome.created.clone();
        outcome.undo = Some(UndoRecord::Duplicate { copies });
    }
    Ok(outcome)
}

/// Duplicates the items of one folder and adds what happened to
/// `outcome`. A folder that cannot be copied into any more (it vanished)
/// is reported as an error of the whole duplicate, after the folders that
/// were already done; a cancellation marks the outcome cancelled, which
/// stops the remaining folders.
fn duplicate_in_folder(
    engine: &mut TransferEngine,
    group: &FolderGroup,
    context: &OperationContext,
    outcome: &mut TransferOutcome,
) {
    let operation = Operation::Copy {
        destination_folder: &group.folder_uri,
        policy: ConflictPolicy::KeepBoth,
    };
    match engine.run(operation, &group.uris, &context.cancel) {
        Ok(result) => {
            let copies = result.landed.iter().map(|item| item.destination.clone());
            outcome.created.extend(copies);
            merge_results(&mut outcome.result, result);
        }
        Err(error) => {
            let folder_name = GioNode::new(&group.folder_uri).display_name();
            record_failure(&mut outcome.result, &folder_name, &error.into());
        }
    }
}

/// `uris` grouped by their folder, both in the order first selected.
fn group_by_folder(uris: &[String]) -> Result<Vec<FolderGroup>, OpsError> {
    let mut groups = FolderGroups::default();
    for uri in uris {
        // OPS-035: a whole share or device is not an item to copy.
        let item = require_item_uri(uri)?;
        let Some(folder) = GioNode::new(&item).parent() else {
            return Err(OpsError::failed(
                "Filesystem roots cannot be copied, moved or trashed as items.",
            ));
        };
        groups.add(folder.uri(), item);
    }
    Ok(groups.into_groups())
}
