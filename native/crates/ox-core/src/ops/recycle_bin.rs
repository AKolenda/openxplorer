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
//!   again stays in the Recycle Bin, and the error says so. Restore and
//!   Restore all items first ask about such names as Windows does
//!   ([`super::restore_conflicts`]); only Replace there moves the item in
//!   the way, to the Recycle Bin.
//! - Missing parent folders of the original location are created again
//!   (OPS-041). Before anything changes, the write protection is asked
//!   about the original location and every location the item's tree
//!   recreates below it (XFER-020).
//! - Only whole items, directly in the Recycle Bin, are restored or
//!   deleted: the backend treats the insides of a trashed folder as part of
//!   that item.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use gio::prelude::*;

use super::context::{on_worker, unless_cancelled, OperationContext};
use super::error::OpsError;
use super::progress::PROGRESS_INTERVAL;
use super::recycle_bin_storage::{listed_targets, StoreDeleter, LISTED_FROM};
use super::results::{AskedItem, ItemQuestions};
use super::run_transfer::TransferOutcome;
use super::undo::UndoRecord;
use crate::entry::{entry_from_info, ATTRIBUTES};
use crate::gio_node::GioNode;
use crate::location::TRASH_URI;
use crate::transfer::item_label;
use crate::transfer::{
    Cancellation, ItemCount, Node, Progress, ProgressScope, SourceChange, TransferMode, TransferResult,
};

/// How often Undo of Move to Trash lists the Recycle Bin again when an
/// item it restores is not listed yet, and how long it waits each time:
/// while a window watches `trash:///`, the backend may list a moment
/// after the Move to Trash finished.
const RELISTS: u32 = 4;
const RELIST_WAIT: Duration = Duration::from_millis(250);

/// How long the listing that makes the backend forget removed items may
/// take ([`forget_removed_items`]).
const FORGET_WAIT: Duration = Duration::from_secs(3);

/// What a listing by name reads of each item.
const NAME_ATTRIBUTES: &str = "standard::name,standard::display-name";

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

/// One item in the Recycle Bin, as a quick listing names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycledName {
    /// The item's `trash:///` URI.
    pub uri: String,
    /// The name it is shown with.
    pub name: String,
}

/// Lists the items in the Recycle Bin by name only, which the backend
/// answers many times faster than [`list_recycle_bin`], since it need not
/// read each item's description: what counting and emptying the Recycle
/// Bin need.
///
/// # Errors
///
/// Cancellation, or the backend's failure.
pub async fn list_recycle_bin_names(cancel: &Cancellation) -> Result<Vec<RecycledName>, OpsError> {
    let cancel = cancel.clone();
    on_worker(move || {
        listed_blocking(NAME_ATTRIBUTES, &cancel, |listing, info| RecycledName {
            uri: listing.child(info).uri().to_string(),
            name: info.display_name().to_string(),
        })
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
/// An item that fails is asked about as Windows asks, in the progress
/// window, through `context`'s question.
///
/// # Errors
///
/// None for the whole request; each item's failure is in the result.
pub async fn delete_from_recycle_bin(
    uris: &[String],
    context: &OperationContext,
) -> Result<TransferResult, OpsError> {
    delete_from_recycle_bin_reporting(uris, context, |_| {}).await
}

/// [`delete_from_recycle_bin`], reporting to `progress` how many items are
/// done, as Windows' progress window counts them while it deletes.
///
/// # Errors
///
/// None for the whole request; each item's failure is in the result.
pub async fn delete_from_recycle_bin_reporting(
    uris: &[String],
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferResult, OpsError> {
    let uris = uris.to_vec();
    let context = context.clone();
    on_worker(move || Ok(delete_items_blocking(&uris, &context, progress))).await
}

/// Deletes everything in the Recycle Bin permanently, after the user
/// confirmed Empty Recycle Bin.
///
/// # Errors
///
/// When the Recycle Bin cannot be listed. Each item's failure is asked
/// about, as [`delete_from_recycle_bin`] asks, and is in the result.
pub async fn empty_recycle_bin(context: &OperationContext) -> Result<TransferResult, OpsError> {
    empty_recycle_bin_reporting(context, |_| {}).await
}

/// [`empty_recycle_bin`], reporting to `progress` how many items are done.
///
/// # Errors
///
/// When the Recycle Bin cannot be listed.
pub async fn empty_recycle_bin_reporting(
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferResult, OpsError> {
    let context = context.clone();
    on_worker(move || {
        let listing =
            |listing: &gio::FileEnumerator, info: &gio::FileInfo| listing.child(info).uri().to_string();
        let uris = listed_blocking(NAME_ATTRIBUTES, &context.cancel, listing)?;
        Ok(delete_items_blocking(&uris, &context, progress))
    })
    .await
}

/// [`list_recycle_bin`] on the calling thread.
fn list_recycle_bin_blocking(cancel: &Cancellation) -> Result<Vec<RecycledItem>, OpsError> {
    listed_blocking(ATTRIBUTES, cancel, |listing, info| {
        let entry = entry_from_info(&listing.child(info), info);
        RecycledItem {
            uri: entry.uri,
            name: entry.name,
            original_path: entry.trash_orig_path,
            deleted_at: entry.trash_deletion_date,
        }
    })
}

/// Each item of the Recycle Bin, read with `attributes` and made by
/// `item`. The listing is closed on success, cancellation and error alike.
fn listed_blocking<T>(
    attributes: &str,
    cancel: &Cancellation,
    item: impl FnMut(&gio::FileEnumerator, &gio::FileInfo) -> T,
) -> Result<Vec<T>, OpsError> {
    cancel.check()?;
    // Without a watcher, GVfs skips rescanning when the Trash folder's
    // modification time is unchanged. A rapid delete and new Trash can
    // share even a nanosecond timestamp, leaving the new item invisible.
    keep_trash_watched();
    let bin = gio::File::for_uri(TRASH_URI);
    enumerate_recycle_bin(&bin, attributes, cancel, item)
}

/// How many watches on `trash:///` this process made, for tests.
static WATCHES_MADE: AtomicUsize = AtomicUsize::new(0);

/// Makes sure that the Trash backend of `GVfs` watches the Trash folders,
/// which it does while a program watches `trash:///`: with the process's
/// one watch, made on a thread of its own the first time the Recycle Bin
/// is listed and kept while the process runs. True once it watches.
///
/// Each listing used to make a watch of its own and drop it: the backend
/// kept sending its changes to every watch whose connection had closed,
/// and with thousands of changes it spent minutes on them, listing items
/// that were gone meanwhile.
fn keep_trash_watched() -> bool {
    static WATCHED: OnceLock<bool> = OnceLock::new();
    *WATCHED.get_or_init(|| {
        let (made, is_made) = std::sync::mpsc::channel();
        let watching = thread::Builder::new()
            .name(String::from("recycle-bin-watch"))
            .spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let bin = gio::File::for_uri(TRASH_URI);
                    let monitor = bin
                        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                        .ok();
                    if monitor.is_some() {
                        WATCHES_MADE.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = made.send(monitor.is_some());
                    if monitor.is_some() {
                        // The watch's replies arrive on this context.
                        loop {
                            context.iteration(true);
                        }
                    }
                });
            });
        watching.is_ok() && is_made.recv().unwrap_or(false)
    })
}

/// Reads a listing with `attributes`, making each item with `item`.
fn enumerate_recycle_bin<T>(
    bin: &gio::File,
    attributes: &str,
    cancel: &Cancellation,
    item: impl FnMut(&gio::FileEnumerator, &gio::FileInfo) -> T,
) -> Result<Vec<T>, OpsError> {
    let listing = bin.enumerate_children(
        attributes,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    let items = read_items(&listing, cancel, item);
    // Closing only releases the listing; what was read stays valid.
    let _ = listing.close(gio::Cancellable::NONE);
    items
}

/// Reads every item of an open Recycle Bin listing.
fn read_items<T>(
    listing: &gio::FileEnumerator,
    cancel: &Cancellation,
    mut item: impl FnMut(&gio::FileEnumerator, &gio::FileInfo) -> T,
) -> Result<Vec<T>, OpsError> {
    let mut items = Vec::new();
    loop {
        cancel.check()?;
        let Some(info) = listing.next_file(Some(cancel.cancellable()))? else {
            return Ok(items);
        };
        items.push(item(listing, &info));
    }
}

/// Restores each item of `uris`, asking about each failure.
fn restore_items_blocking(uris: &[String], context: &OperationContext) -> TransferOutcome {
    let mut outcome = TransferOutcome::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    for (index, uri) in uris.iter().enumerate() {
        let name = GioNode::new(uri).display_name();
        let item = AskedItem {
            name: &name,
            uri,
            more_items: index + 1 < uris.len(),
        };
        if let Some(restored) = questions.run(&mut outcome.result, item, || restore_item(uri, context)) {
            outcome.result.done.push(uri.clone());
            outcome.created.push(restored);
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
///
/// The operation waits for it at most [`FORGET_WAIT`], and it finishes
/// on its own thread after that: a backend that takes longer is busy
/// following the changes for a program that watches `trash:///`, which
/// updates its record anyway, and after thousands of removals it could
/// hold the operation for a minute.
pub(super) fn forget_removed_items() {
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let listing = thread::Builder::new()
        .name(String::from("recycle-bin-forget"))
        .spawn(move || {
            let _ = listed_blocking(NAME_ATTRIBUTES, &Cancellation::new(), |_, _| ());
            let _ = done.send(());
        });
    if listing.is_ok() {
        let _ = finished.recv_timeout(FORGET_WAIT);
    }
}

/// Restores the Recycle Bin item `uri` to its original location and
/// returns that location's URI.
pub(super) fn restore_item(uri: &str, context: &OperationContext) -> Result<String, OpsError> {
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
pub(super) fn recreate_original_folder(
    target: &gio::File,
    context: &OperationContext,
) -> Result<(), OpsError> {
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

/// Deletes each item of `uris` permanently, asking about each failure as
/// Windows does: an item that is gone asks "Item Not Found", one the user
/// may not delete "File Access Denied", anything else "Interrupted
/// Action". From [`LISTED_FROM`] items on, where they are kept is read
/// from one listing first ([`recycle_bin_storage`](super::recycle_bin_storage)).
fn delete_items_blocking(
    uris: &[String],
    context: &OperationContext,
    mut progress: impl FnMut(Progress),
) -> TransferResult {
    let mut result = TransferResult::default();
    // Only a cancellation fails the listing.
    let targets = if uris.len() >= LISTED_FROM {
        let Ok(targets) = listed_targets(&context.cancel) else {
            result.cancelled = true;
            return result;
        };
        targets
    } else {
        HashMap::new()
    };
    let mut store = StoreDeleter::with_targets(targets);
    let mut questions = ItemQuestions::new(context, TransferMode::Delete);
    let mut reports = ItemReports::new(uris.len());
    for (position, index) in store.deletion_order(uris).into_iter().enumerate() {
        let uri = &uris[index];
        let name = GioNode::new(uri).display_name();
        reports.report(position, &name, &mut progress);
        let item = AskedItem {
            name: &name,
            uri,
            more_items: position + 1 < uris.len(),
        };
        if questions
            .run(&mut result, item, || {
                delete_item(uri, &mut store, &context.cancel)
            })
            .is_some()
        {
            result.done.push(uri.clone());
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

/// The progress reports of a run over the Recycle Bin's items: one for
/// the first item, then at most one per [`PROGRESS_INTERVAL`], so a run
/// over thousands of items does not flood the window with them.
struct ItemReports {
    /// The items of the run.
    total: usize,
    /// When the last report went.
    last: Option<Instant>,
}

impl ItemReports {
    fn new(total: usize) -> Self {
        Self { total, last: None }
    }

    /// Reports that the item at `index`, called `name`, is under way.
    fn report(&mut self, index: usize, name: &str, progress: &mut impl FnMut(Progress)) {
        let now = Instant::now();
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL)
        {
            return;
        }
        self.last = Some(now);
        #[expect(clippy::cast_precision_loss, reason = "a fraction for a progress bar")]
        let fraction = index as f64 / self.total.max(1) as f64;
        progress(Progress {
            label: item_label(TransferMode::Delete, name, index + 1, self.total),
            fraction,
            scope: ProgressScope::Batch,
            bytes: None,
            items: Some(ItemCount {
                done: index as u64,
                total: self.total as u64,
            }),
            name: Some(name.to_owned()),
        });
    }
}

/// Deletes the Recycle Bin item `uri` permanently, with everything in a
/// trashed folder: from its Trash folder on disk where `store` finds it
/// kept in one, which spares a large Recycle Bin the backend's work per
/// item, otherwise through the backend.
fn delete_item(uri: &str, store: &mut StoreDeleter, cancel: &Cancellation) -> Result<(), OpsError> {
    cancel.check()?;
    let item = top_level_item(uri)?;
    if let Some(deleted) = store.delete(uri, &item, cancel) {
        return deleted;
    }
    cancel.check()?;
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
    let found = find_trashed_since(original_paths, since, &context.cancel)?;
    Ok(restore_found(original_paths, &found, context))
}

/// The Recycle Bin item, for each of `original_paths`, deleted from it
/// most recently since `since`, or `None` when there is none. Undo of a
/// Replace looks them up before it moves anything else to the Trash
/// from the same paths, so it restores the replaced files, not those.
///
/// # Errors
///
/// When the Recycle Bin cannot be listed.
pub(crate) fn find_trashed_since(
    original_paths: &[PathBuf],
    since: u64,
    cancel: &Cancellation,
) -> Result<Vec<Option<String>>, OpsError> {
    let items = listing_holding(original_paths, since, cancel)?;
    let newest = newest_trashed_by_path(&items, since);
    Ok(original_paths
        .iter()
        .map(|path| newest.get(path.as_path()).map(|item| item.uri.clone()))
        .collect())
}

/// Restores each found Recycle Bin item (`found`, from
/// [`find_trashed_since`]) to its path of `original_paths`, never
/// overwriting, asking about each failure.
pub(crate) fn restore_found(
    original_paths: &[PathBuf],
    found: &[Option<String>],
    context: &OperationContext,
) -> TransferResult {
    let mut result = TransferResult::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    for (index, (original_path, item)) in original_paths.iter().zip(found).enumerate() {
        let name = path_name(original_path);
        let asked = AskedItem {
            name: &name,
            uri: item.as_deref().unwrap_or_default(),
            more_items: index + 1 < original_paths.len(),
        };
        let restore = || match item {
            Some(uri) => put_back(&gio::File::for_uri(uri), original_path, context),
            None => Err(OpsError::NotFound(crate::i18n::gettext(
                "It is no longer in the Recycle Bin.",
            ))),
        };
        if let Some(uri) = questions.run(&mut result, asked, restore) {
            result.done.push(uri);
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

/// The Recycle Bin's items, listed again a few times while one of
/// `original_paths` deleted since `since` is not listed yet: the backend
/// may list an item a moment after the Move to Trash finished.
fn listing_holding(
    original_paths: &[PathBuf],
    since: u64,
    cancel: &Cancellation,
) -> Result<Vec<RecycledItem>, OpsError> {
    let mut items = list_recycle_bin_blocking(cancel)?;
    let mut relists = 0;
    while relists < RELISTS && {
        let newest = newest_trashed_by_path(&items, since);
        original_paths
            .iter()
            .any(|path| !newest.contains_key(path.as_path()))
    } {
        thread::sleep(RELIST_WAIT);
        items = list_recycle_bin_blocking(cancel)?;
        relists += 1;
    }
    Ok(items)
}

/// The newest of `items` deleted from each original path since `since`,
/// by that path: one pass over the Recycle Bin, so an Undo of thousands
/// of items does not read the whole listing once per item. Of items
/// deleted in the same second, the one listed last counts.
fn newest_trashed_by_path(items: &[RecycledItem], since: u64) -> HashMap<&Path, &RecycledItem> {
    let mut newest: HashMap<&Path, &RecycledItem> = HashMap::new();
    for item in items {
        let (Some(path), Some(deleted_at)) = (item.original_path.as_deref(), item.deleted_at) else {
            continue;
        };
        if deleted_at < since {
            continue;
        }
        newest
            .entry(path)
            .and_modify(|known| {
                if known.deleted_at <= Some(deleted_at) {
                    *known = item;
                }
            })
            .or_insert(item);
    }
    newest
}

/// The last component of `path`, for messages.
pub(super) fn path_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_share_the_process_s_one_watch_of_the_recycle_bin() {
        assert_eq!(std::env::var("OX_ISOLATED_SESSION").as_deref(), Ok("1"));
        let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("private data directory"));
        assert!(data.starts_with(std::env::temp_dir()));
        assert_eq!(glib::user_data_dir(), data);

        // The real GVfs backend; nothing is trashed or removed.
        for _ in 0..16 {
            listed_blocking(NAME_ATTRIBUTES, &Cancellation::new(), |_, _| ()).unwrap();
        }
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let result = listed_blocking(NAME_ATTRIBUTES, &cancelled, |_, _| ());

        assert_eq!(result, Err(OpsError::Cancelled));
        assert!(keep_trash_watched());
        assert_eq!(
            WATCHES_MADE.load(Ordering::Relaxed),
            1,
            "one watch for every listing"
        );
    }

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

        let found = newest_trashed_by_path(&items, 200);
        let too_old = newest_trashed_by_path(&items, 206);

        let path = Path::new("/home/user/a.txt");
        assert_eq!(
            found.get(path).map(|item| item.uri.as_str()),
            Some("trash:///a.2.txt")
        );
        assert_eq!(too_old.get(path), None);
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
