// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin (`trash:///`): what it holds, putting items back where
//! they came from, deleting them for good and emptying it (OPS-040 to
//! OPS-043).
//!
//! New in the native app, from the Dolphin baseline; the Python app could
//! only move items to the Trash. Everything goes through GIO's `trash:///`
//! backend (gvfsd-trash), so items trashed by Nautilus, Dolphin or any
//! other freedesktop Trash user appear, restore and empty the same way.
//! The safety rules:
//!
//! - Restoring never overwrites: an item whose original name is taken
//!   again stays in the Recycle Bin, and the error says so.
//! - Missing parent folders of the original location are created again
//!   (OPS-041). Before anything changes, the write protection is asked
//!   about the original location and every location the item's tree
//!   recreates below it (XFER-020).
//! - Only whole items, directly in the Recycle Bin, are restored or
//!   deleted: the backend treats the insides of a trashed folder as part of
//!   that item.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use gio::prelude::*;

use super::context::{on_worker, unless_cancelled, OperationContext};
use super::error::OpsError;
use super::results::record_failure;
use super::run_transfer::TransferOutcome;
use super::undo::UndoRecord;
use crate::entry::{entry_from_info, ATTRIBUTES};
use crate::gio_node::GioNode;
use crate::location::TRASH_URI;
use crate::transfer::{Cancellation, Node, SourceChange, TransferResult};

/// How often Undo of Move to Trash lists the Recycle Bin again when an
/// item it restores is not listed yet, and how long it waits each time:
/// while a window watches `trash:///`, the backend may list a moment
/// after the Move to Trash finished.
const RELISTS: u32 = 4;
const RELIST_WAIT: Duration = Duration::from_millis(250);

/// One item in the Recycle Bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycledItem {
    /// The item's `trash:///` URI.
    pub uri: String,
    /// The name it is shown with.
    pub name: String,
    /// Where Restore puts it back, when the Recycle Bin recorded it.
    pub original_path: Option<PathBuf>,
    /// When it was deleted, in seconds since the Unix epoch.
    pub deleted_at: Option<u64>,
}

/// Lists the items in the Recycle Bin.
///
/// # Errors
///
/// Cancellation, or the backend's failure (for example when the Trash
/// backend of `GVfs` is not installed).
pub async fn list_recycle_bin(cancel: &Cancellation) -> Result<Vec<RecycledItem>, OpsError> {
    let cancel = cancel.clone();
    on_worker(move || list_recycle_bin_blocking(&cancel)).await
}

/// The number of items in the Recycle Bin; Empty Recycle Bin is disabled
/// at zero (OPS-042).
///
/// # Errors
///
/// Cancellation, or the backend's failure.
pub async fn recycle_bin_item_count(cancel: &Cancellation) -> Result<u32, OpsError> {
    let cancel = cancel.clone();
    on_worker(move || {
        let bin = gio::File::for_uri(TRASH_URI);
        let info = bin.query_info(
            "trash::item-count",
            gio::FileQueryInfoFlags::NONE,
            Some(cancel.cancellable()),
        )?;
        Ok(info.attribute_uint32("trash::item-count"))
    })
    .await
}

/// Puts each of the Recycle Bin items `uris` back where it came from. The
/// outcome lists the restored locations, to select them, and Undo moves
/// them to the Trash again.
///
/// # Errors
///
/// None for the whole request; each item's failure is reported in the
/// outcome's result.
pub async fn restore_from_recycle_bin(
    uris: &[String],
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let uris = uris.to_vec();
    let context = context.clone();
    on_worker(move || Ok(restore_items_blocking(&uris, &context))).await
}

/// Deletes the Recycle Bin items `uris` permanently, leaving the others.
///
/// # Errors
///
/// None for the whole request; each item's failure is reported in the
/// result.
pub async fn delete_from_recycle_bin(
    uris: &[String],
    cancel: &Cancellation,
) -> Result<TransferResult, OpsError> {
    let uris = uris.to_vec();
    let cancel = cancel.clone();
    on_worker(move || Ok(delete_items_blocking(&uris, &cancel))).await
}

/// Deletes everything in the Recycle Bin permanently, after the user
/// confirmed Empty Recycle Bin.
///
/// # Errors
///
/// When the Recycle Bin cannot be listed. Each item's failure is reported
/// in the result.
pub async fn empty_recycle_bin(cancel: &Cancellation) -> Result<TransferResult, OpsError> {
    let cancel = cancel.clone();
    on_worker(move || {
        let items = list_recycle_bin_blocking(&cancel)?;
        let uris: Vec<String> = items.into_iter().map(|item| item.uri).collect();
        Ok(delete_items_blocking(&uris, &cancel))
    })
    .await
}

/// [`list_recycle_bin`] on the calling thread. The listing is closed on
/// success, cancellation and error alike.
fn list_recycle_bin_blocking(cancel: &Cancellation) -> Result<Vec<RecycledItem>, OpsError> {
    let bin = gio::File::for_uri(TRASH_URI);
    let listing = bin.enumerate_children(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    let items = read_items(&listing, cancel);
    // Closing only releases the listing; what was read stays valid.
    let _ = listing.close(gio::Cancellable::NONE);
    items
}

/// Reads every item of an open Recycle Bin listing.
fn read_items(listing: &gio::FileEnumerator, cancel: &Cancellation) -> Result<Vec<RecycledItem>, OpsError> {
    let mut items = Vec::new();
    loop {
        cancel.check()?;
        let Some(info) = listing.next_file(Some(cancel.cancellable()))? else {
            return Ok(items);
        };
        let entry = entry_from_info(&listing.child(&info), &info);
        items.push(RecycledItem {
            uri: entry.uri,
            name: entry.name,
            original_path: entry.trash_orig_path,
            deleted_at: entry.trash_deletion_date,
        });
    }
}

/// Restores each item of `uris` and reports what happened.
fn restore_items_blocking(uris: &[String], context: &OperationContext) -> TransferOutcome {
    let mut outcome = TransferOutcome::default();
    for uri in uris {
        match restore_item(uri, context) {
            Ok(restored) => {
                outcome.result.done.push(uri.clone());
                outcome.created.push(restored);
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

/// Lists the Recycle Bin once, so the Trash backend of `GVfs` forgets the
/// items an operation took out of it.
///
/// While no program watches `trash:///`, gvfsd-trash updates its record of
/// the Trash folder only when the Recycle Bin is listed, and restoring or
/// deleting an item does not update that record. An item trashed later
/// under the same name would then never be listed, although it is in the
/// Trash. A failed listing only leaves this work to the next one; it runs
/// even after a cancellation, because the removals already happened.
pub(super) fn forget_removed_items() {
    let _ = list_recycle_bin_blocking(&Cancellation::new());
}

/// Restores the Recycle Bin item `uri` to its original location and
/// returns that location's URI.
fn restore_item(uri: &str, context: &OperationContext) -> Result<String, OpsError> {
    context.cancel.check()?;
    let item = top_level_item(uri)?;
    let info = item.query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(context.cancellable()),
    )?;
    let entry = entry_from_info(&item, &info);
    let Some(original_path) = entry.trash_orig_path else {
        return Err(OpsError::failed(crate::i18n::format_message(
            "The Recycle Bin does not record where “{name}” came from.",
            &[("name", &entry.name)],
        )));
    };
    put_back(&item, &original_path, context)
}

/// Moves `item` out of the Recycle Bin to `original_path`, never
/// overwriting, and returns the restored item's URI.
fn put_back(item: &gio::File, original_path: &Path, context: &OperationContext) -> Result<String, OpsError> {
    let target = gio::File::for_path(original_path);
    let taken = || name_taken_in_original_folder(original_path);
    move_out(item, &target, context, taken, |target| {
        recreate_original_folder(target, context)
    })
}

/// Moves the Recycle Bin item `item` to `target`, never overwriting, and
/// returns `target`'s URI. `taken` is the error when `target` exists;
/// `prepare` runs once the checks passed, before the move.
pub(super) fn move_out(
    item: &gio::File,
    target: &gio::File,
    context: &OperationContext,
    taken: impl Fn() -> OpsError,
    prepare: impl FnOnce(&gio::File) -> Result<(), OpsError>,
) -> Result<String, OpsError> {
    let destination = GioNode::from_file(target.clone());
    // XFER-020: the target and every location the item's tree creates
    // below it. The Recycle Bin itself is no protected location.
    let source = GioNode::from_file(item.clone());
    let protection = &context.protection;
    protection.check_tree(&source, &destination, &context.cancel, SourceChange::Kept)?;
    if unless_cancelled(&context.cancel, || destination.exists(Some(&context.cancel)))? {
        return Err(taken());
    }
    prepare(target)?;
    // Never overwriting (no OVERWRITE flag) and never degrading to a copy
    // that could leave a partial item behind (XFER-011).
    let flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE;
    match item.move_(target, flags, Some(context.cancellable()), None) {
        Ok(()) => Ok(target.uri().to_string()),
        Err(error) if error.matches(gio::IOErrorEnum::Exists) => Err(taken()),
        Err(error) => Err(error.into()),
    }
}

/// OPS-041: creates the missing folders above a restored item's original
/// location.
fn recreate_original_folder(target: &gio::File, context: &OperationContext) -> Result<(), OpsError> {
    let Some(folder) = target.parent() else {
        return Ok(());
    };
    match folder.make_directory_with_parents(Some(context.cancellable())) {
        Ok(()) => Ok(()),
        Err(error) if error.matches(gio::IOErrorEnum::Exists) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The refusal to restore onto a name that is taken again.
fn name_taken_in_original_folder(original_path: &Path) -> OpsError {
    let name = path_name(original_path);
    OpsError::Exists(crate::i18n::format_message(
        "An item named “{name}” already exists in its original folder. It was left in the Recycle Bin.",
        &[("name", &name)],
    ))
}

/// Deletes each item of `uris` permanently and reports what happened.
fn delete_items_blocking(uris: &[String], cancel: &Cancellation) -> TransferResult {
    let mut result = TransferResult::default();
    for uri in uris {
        match delete_item(uri, cancel) {
            Ok(()) => result.done.push(uri.clone()),
            Err(error) => record_failure(&mut result, &GioNode::new(uri).display_name(), &error),
        }
        if result.cancelled {
            break;
        }
    }
    if !result.done.is_empty() {
        forget_removed_items();
    }
    result
}

/// Deletes the Recycle Bin item `uri` permanently. The backend removes a
/// trashed folder with everything in it.
fn delete_item(uri: &str, cancel: &Cancellation) -> Result<(), OpsError> {
    cancel.check()?;
    let item = top_level_item(uri)?;
    item.delete(Some(cancel.cancellable()))?;
    Ok(())
}

/// The Recycle Bin item `uri`, when it is directly in the Recycle Bin.
pub(super) fn top_level_item(uri: &str) -> Result<gio::File, OpsError> {
    let item = gio::File::for_uri(uri);
    let bin = gio::File::for_uri(TRASH_URI);
    let is_top_level = item.parent().is_some_and(|parent| parent.equal(&bin));
    if !item.has_uri_scheme("trash") || !is_top_level {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Only whole items in the Recycle Bin can be restored or deleted there.",
        )));
    }
    Ok(item)
}

/// Undo of Move to Trash: restores, for each of `original_paths`, the
/// newest Recycle Bin item deleted from there since `since` (seconds since
/// the Unix epoch). An older item from the same place stays in the
/// Recycle Bin.
///
/// # Errors
///
/// Only a Recycle Bin that cannot be listed fails the whole Undo; an item
/// that cannot be restored is reported in the result instead.
pub(crate) fn restore_trashed_since(
    original_paths: &[PathBuf],
    since: u64,
    context: &OperationContext,
) -> Result<TransferResult, OpsError> {
    let mut items = list_recycle_bin_blocking(&context.cancel)?;
    let mut relists = 0;
    while relists < RELISTS
        && original_paths
            .iter()
            .any(|path| newest_trashed_from(&items, path, since).is_none())
    {
        thread::sleep(RELIST_WAIT);
        items = list_recycle_bin_blocking(&context.cancel)?;
        relists += 1;
    }
    let mut result = TransferResult::default();
    for original_path in original_paths {
        let restored = match newest_trashed_from(&items, original_path, since) {
            Some(item) => put_back(&gio::File::for_uri(&item.uri), original_path, context),
            None => Err(OpsError::NotFound(crate::i18n::gettext(
                "It is no longer in the Recycle Bin.",
            ))),
        };
        match restored {
            Ok(uri) => result.done.push(uri),
            Err(error) => record_failure(&mut result, &path_name(original_path), &error),
        }
        if result.cancelled {
            break;
        }
    }
    if !result.done.is_empty() {
        forget_removed_items();
    }
    Ok(result)
}

/// The newest of `items` deleted from `original_path` since `since`.
fn newest_trashed_from<'a>(
    items: &'a [RecycledItem],
    original_path: &Path,
    since: u64,
) -> Option<&'a RecycledItem> {
    items
        .iter()
        .filter(|item| item.original_path.as_deref() == Some(original_path))
        .filter(|item| item.deleted_at.is_some_and(|deleted_at| deleted_at >= since))
        .max_by_key(|item| item.deleted_at)
}

/// The last component of `path`, for messages.
fn path_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recycled(uri: &str, original: &str, deleted_at: u64) -> RecycledItem {
        RecycledItem {
            uri: uri.to_owned(),
            name: String::from("a.txt"),
            original_path: Some(PathBuf::from(original)),
            deleted_at: Some(deleted_at),
        }
    }

    #[test]
    fn undo_restores_only_the_newest_item_trashed_by_the_operation() {
        let items = [
            recycled("trash:///a.txt", "/home/user/a.txt", 100),
            recycled("trash:///a.2.txt", "/home/user/a.txt", 205),
            recycled("trash:///a.3.txt", "/home/user/a.txt", 200),
            recycled("trash:///b.txt", "/home/user/b.txt", 300),
        ];

        let found = newest_trashed_from(&items, Path::new("/home/user/a.txt"), 200);
        let too_old = newest_trashed_from(&items, Path::new("/home/user/a.txt"), 206);

        assert_eq!(found.map(|item| item.uri.as_str()), Some("trash:///a.2.txt"));
        assert_eq!(too_old, None);
    }

    #[test]
    fn only_whole_items_in_the_recycle_bin_are_accepted() {
        assert!(top_level_item("trash:///report.txt").is_ok());
        assert!(top_level_item("trash:///folder/inside.txt").is_err());
        assert!(top_level_item("file:///tmp/report.txt").is_err());
    }

    #[test]
    fn a_restore_onto_a_taken_name_says_the_item_stayed_in_the_recycle_bin() {
        let error = name_taken_in_original_folder(Path::new("/home/user/report.txt"));

        assert_eq!(
            error.to_string(),
            "An item named “report.txt” already exists in its original folder. It was left in the Recycle Bin."
        );
    }
}
