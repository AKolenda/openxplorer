// SPDX-License-Identifier: AGPL-3.0-only
//! Recycle Bin items dragged into a folder (OPS-046).
//!
//! As in Dolphin, dragging items out of the Recycle Bin into a folder
//! moves them there under their original names, without asking copy,
//! move or link. The rules of Restore apply ([`super::recycle_bin`]):
//! only whole items directly in the Recycle Bin, never overwriting, never
//! a copy-then-delete, and the write protection is asked first. Undo moves
//! them to the Trash again.

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::recycle_bin::{forget_removed_items, move_out, top_level_item};
use super::results::record_failure;
use super::run_transfer::TransferOutcome;
use super::undo::UndoRecord;
use crate::entry::{entry_from_info, ATTRIBUTES};
use crate::gio_node::GioNode;
use crate::location::{normalise, TRASH_URI};
use crate::transfer::Node;

/// True for an item directly in the Recycle Bin, which a drag may carry.
pub fn is_recycle_bin_item(uri: &str) -> bool {
    let item = gio::File::for_uri(uri);
    let bin = gio::File::for_uri(TRASH_URI);
    item.has_uri_scheme("trash") && item.parent().is_some_and(|parent| parent.equal(&bin))
}

/// Moves each of the Recycle Bin items `uris` into the folder at
/// `folder_uri` under its original name. The outcome lists the moved
/// items, to select them, and Undo moves them to the Trash again.
///
/// # Errors
///
/// An invalid folder address. Each item's failure is reported in the
/// outcome's result.
pub async fn move_out_of_recycle_bin(
    uris: &[String],
    folder_uri: &str,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let folder = gio::File::for_uri(&normalise(folder_uri)?);
    let uris = uris.to_vec();
    let context = context.clone();
    on_worker(move || Ok(move_items_blocking(&uris, &folder, &context))).await
}

/// Moves each item of `uris` into `folder` and reports what happened.
fn move_items_blocking(uris: &[String], folder: &gio::File, context: &OperationContext) -> TransferOutcome {
    let mut outcome = TransferOutcome::default();
    for uri in uris {
        match move_item(uri, folder, context) {
            Ok(moved) => {
                outcome.result.done.push(uri.clone());
                outcome.created.push(moved);
            }
            Err(error) => record_failure(&mut outcome.result, &GioNode::new(uri).display_name(), &error),
        }
        if outcome.result.cancelled {
            break;
        }
    }
    if !outcome.created.is_empty() {
        forget_removed_items();
        let restored = outcome.created.clone();
        outcome.undo = Some(UndoRecord::Restore { restored });
    }
    outcome
}

/// Moves the Recycle Bin item `uri` into `folder` under its original
/// name and returns its new URI.
fn move_item(uri: &str, folder: &gio::File, context: &OperationContext) -> Result<String, OpsError> {
    context.cancel.check()?;
    let item = top_level_item(uri)?;
    let info = item.query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(context.cancellable()),
    )?;
    let name = entry_from_info(&item, &info).name;
    let target = folder.child(&name);
    let taken = || {
        OpsError::Exists(format!(
            "An item named “{name}” already exists here. It was left in the Recycle Bin."
        ))
    };
    move_out(&item, &target, context, taken, |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-046
    #[test]
    fn only_whole_items_in_the_recycle_bin_are_dragged_out() {
        assert!(is_recycle_bin_item("trash:///report.txt"));
        assert!(!is_recycle_bin_item("trash:///"), "the Recycle Bin itself");
        assert!(
            !is_recycle_bin_item("trash:///folder/inside.txt"),
            "part of a trashed folder"
        );
        assert!(!is_recycle_bin_item("file:///tmp/report.txt"));
    }
}
