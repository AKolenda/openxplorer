// SPDX-License-Identifier: AGPL-3.0-only
//! New folder and New file (an empty file with any name).
//!
//! Ports `create_item` in `desktop/gio_backend.py` and the checks the
//! `create` branch of `dispatch` in `desktop/winspace.py` runs first.
//! Creating never overwrites (OPS-008): a folder is made with GIO's
//! exclusive `make_directory` and a file with an exclusive `create`, so a
//! taken name fails and the existing item stays as it was.

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::undo::UndoRecord;
use crate::location::{is_smb_server, normalise, validate_name, ItemKind};

/// An item an operation created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedItem {
    /// The new item's URI, to select it.
    pub uri: String,
    /// Whether it is a folder or a file.
    pub kind: ItemKind,
}

impl CreatedItem {
    /// How Undo removes the item again: it goes to the Trash.
    pub fn undo_record(&self) -> UndoRecord {
        UndoRecord::Create {
            uri: self.uri.clone(),
            kind: self.kind,
        }
    }
}

/// Creates an empty folder or file called `name` in the folder at
/// `folder_uri`.
///
/// # Errors
///
/// An invalid name, a server listing ("Open a network share before
/// creating files or folders."), a protected folder, a taken name
/// ([`OpsError::Exists`]; nothing is overwritten), cancellation, or the
/// backend's failure.
pub async fn create_item(
    folder_uri: &str,
    name: &str,
    kind: ItemKind,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let name = name.to_owned();
    let context = context.clone();
    on_worker(move || create_item_blocking(&folder_uri, &name, kind, &context)).await
}

/// [`create_item`] on the calling thread.
fn create_item_blocking(
    folder_uri: &str,
    name: &str,
    kind: ItemKind,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    validate_name(name)?;
    // A server listing holds shares, not files; nothing can be created there.
    if is_smb_server(folder_uri) {
        return Err(OpsError::failed(
            "Open a network share before creating files or folders.",
        ));
    }
    let folder_uri = normalise(folder_uri)?;
    context.protection.check(&folder_uri)?;
    context.cancel.check()?;
    let item = gio::File::for_uri(&folder_uri).child(name);
    create_exclusively(&item, kind, context).map_err(|error| name_taken_or(error, name))?;
    Ok(CreatedItem {
        uri: item.uri().to_string(),
        kind,
    })
}

/// OPS-008: creates `item` only when its name is free. Both GIO calls are
/// exclusive, so an item that appears meanwhile is never replaced.
fn create_exclusively(item: &gio::File, kind: ItemKind, context: &OperationContext) -> Result<(), OpsError> {
    let cancellable = Some(context.cancellable());
    match kind {
        ItemKind::Folder => item.make_directory(cancellable)?,
        ItemKind::File => {
            let stream = item.create(gio::FileCreateFlags::NONE, cancellable)?;
            stream.close(cancellable)?;
        }
    }
    Ok(())
}

/// A taken name reported in the app's wording (OPS-008); other failures
/// keep their message.
pub(crate) fn name_taken_or(error: OpsError, name: &str) -> OpsError {
    match error {
        OpsError::Exists(_) => OpsError::Exists(format!(
            "An item named “{name}” already exists. Nothing was overwritten."
        )),
        other => other,
    }
}
