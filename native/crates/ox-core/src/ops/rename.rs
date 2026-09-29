// SPDX-License-Identifier: AGPL-3.0-only
//! Rename (F2).
//!
//! Ports `rename_item` in `desktop/gio_backend.py` and the check the
//! `rename` branch of `dispatch` in `desktop/winspace.py` runs first. The
//! rules (OPS-008): the new name is one valid path component; a whole
//! share, a device or a filesystem root is never renamed; the same name is
//! a no-op; the whole tree is checked against the write protection first
//! (XFER-020); and the item is renamed by a native move within its folder
//! that never overwrites (`set_display_name` on a phone, XFER-024).

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::create::name_taken_or;
use super::error::OpsError;
use super::undo::UndoRecord;
use crate::gio_node::GioNode;
use crate::location::{require_item_uri, validate_name};
use crate::transfer::{Node, SourceChange};

/// A finished rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedItem {
    /// The item's URI before the rename.
    pub original_uri: String,
    /// The item's URI now, to select it.
    pub uri: String,
    /// The item's name now.
    pub name: String,
}

impl RenamedItem {
    /// True when the new name was the old one, so nothing changed.
    pub fn is_unchanged(&self) -> bool {
        self.original_uri == self.uri
    }

    /// How Undo renames the item back, or `None` when nothing changed.
    pub fn undo_record(&self) -> Option<UndoRecord> {
        if self.is_unchanged() {
            return None;
        }
        Some(UndoRecord::Rename {
            original_uri: self.original_uri.clone(),
            renamed_uri: self.uri.clone(),
        })
    }
}

/// Renames the item at `uri` to `new_name` within its folder.
///
/// # Errors
///
/// An invalid name, a share, device or filesystem root, a protected
/// location anywhere in the item's tree, a taken name
/// ([`OpsError::Exists`]; nothing is overwritten), cancellation, or the
/// backend's failure.
pub async fn rename_item(
    uri: &str,
    new_name: &str,
    context: &OperationContext,
) -> Result<RenamedItem, OpsError> {
    let uri = uri.to_owned();
    let new_name = new_name.to_owned();
    let context = context.clone();
    on_worker(move || rename_item_blocking(&uri, &new_name, &context)).await
}

/// [`rename_item`] on the calling thread.
fn rename_item_blocking(
    uri: &str,
    new_name: &str,
    context: &OperationContext,
) -> Result<RenamedItem, OpsError> {
    validate_name(new_name)?;
    // OPS-035: a whole share or device is not an item that can be renamed.
    let uri = require_item_uri(uri)?;
    context.protection.check(&uri)?;
    let source = GioNode::new(&uri);
    let Some(folder) = source.parent() else {
        return Err(OpsError::failed("Cannot rename a filesystem root."));
    };
    let destination = folder.child(new_name.as_ref());
    let renamed = RenamedItem {
        original_uri: source.uri(),
        uri: destination.uri(),
        name: new_name.to_owned(),
    };
    if renamed.is_unchanged() {
        return Ok(renamed);
    }
    move_within_folder(&source, destination.as_ref(), context)
        .map_err(|error| name_taken_or(error, new_name))?;
    Ok(renamed)
}

/// Undoes a rename: gives the item at `renamed_uri` its name from
/// `original_uri` back, under the same rules as a rename. The original
/// name is used byte for byte, so a name that is not valid UTF-8 comes
/// back exactly.
///
/// # Errors
///
/// A location that is no longer in the same folder, a protected location,
/// a taken original name (nothing is overwritten), cancellation, or the
/// backend's failure.
pub(crate) fn rename_back(
    original_uri: &str,
    renamed_uri: &str,
    context: &OperationContext,
) -> Result<(), OpsError> {
    let source = GioNode::new(renamed_uri);
    let original = GioNode::new(original_uri);
    if !are_in_same_folder(&source, &original) {
        return Err(OpsError::failed(
            "Undo can only rename an item back within its own folder.",
        ));
    }
    context.protection.check(renamed_uri)?;
    move_within_folder(&source, &original, context)
        .map_err(|error| name_taken_or(error, &original.display_name()))
}

/// True when `first` and `second` have the same parent folder.
fn are_in_same_folder(first: &GioNode, second: &GioNode) -> bool {
    let (Some(first_folder), Some(second_folder)) = (first.parent(), second.parent()) else {
        return false;
    };
    let first_folder = gio::File::for_uri(&first_folder.uri());
    first_folder.equal(&gio::File::for_uri(&second_folder.uri()))
}

/// The whole-tree write check, then the native same-folder move that never
/// overwrites.
fn move_within_folder(
    source: &GioNode,
    destination: &dyn Node,
    context: &OperationContext,
) -> Result<(), OpsError> {
    // XFER-020: every location in the renamed tree and its new location,
    // as `_check_write_tree` with `source_writable=True`, which
    // `rename_item` in `desktop/gio_backend.py` runs.
    let protection = &context.protection;
    protection.check_tree(source, destination, &context.cancel, SourceChange::Changed)?;
    source.move_native(destination, Some(&context.cancel))?;
    Ok(())
}
