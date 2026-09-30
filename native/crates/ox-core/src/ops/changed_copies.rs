// SPDX-License-Identifier: AGPL-3.0-only
//! Which copies changed since their copy finished (OPS-030).
//!
//! Undoing a copy moves the copies to the Trash. As Dolphin's "Undo File
//! Copy Confirmation" does, the window asks first when a copy was
//! modified after the copy, so work done in it is not thrown away
//! unnoticed. [`changed_copies`] finds them: the copies of a Copy or
//! Duplicate step that, or anything inside which, has a modification
//! time later than when the step was recorded. Like Dolphin's
//! `copiedFileWasModified`, it looks at every copied item; editing a file
//! inside a copied folder leaves the folder's own time unchanged. Links
//! are not followed, and at most [`MAX_CHECKED`] items are looked at, so a
//! huge copied tree cannot hold Undo back for long.

use gio::prelude::*;

use super::context::on_worker;
use super::undo::UndoRecord;
use crate::transfer::MAX_DEPTH;

/// The most items inside copied folders that are looked at.
const MAX_CHECKED: usize = 20_000;

/// What is read of each item.
const ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,time::modified";

/// The names of the copies `record` would move to the Trash that, or
/// anything inside which, were modified after `since` (seconds since the
/// Unix epoch). Empty for every other step, and for copies that cannot be
/// read.
pub async fn changed_copies(record: &UndoRecord, since: u64) -> Vec<String> {
    let (UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies }) = record else {
        return Vec::new();
    };
    let copies = copies.clone();
    on_worker(move || Ok(changed_copies_blocking(&copies, since)))
        .await
        .unwrap_or_default()
}

/// [`changed_copies`] of `copies` on the calling thread.
fn changed_copies_blocking(copies: &[String], since: u64) -> Vec<String> {
    let mut budget = MAX_CHECKED;
    copies
        .iter()
        .filter_map(|uri| {
            let copy = gio::File::for_uri(uri);
            let info = copy
                .query_info(
                    ATTRIBUTES,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    gio::Cancellable::NONE,
                )
                .ok()?;
            let changed = is_newer(&info, since)
                || (info.file_type() == gio::FileType::Directory
                    && tree_changed(&copy, since, 1, &mut budget));
            changed.then(|| info.display_name().to_string())
        })
        .collect()
}

/// True when anything inside `folder`, `depth` levels below a copy, was
/// modified after `since`; each item looked at uses up one of `budget`.
fn tree_changed(folder: &gio::File, since: u64, depth: usize, budget: &mut usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let Ok(listing) = folder.enumerate_children(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    ) else {
        return false;
    };
    let mut changed = false;
    while let Ok(Some(info)) = listing.next_file(gio::Cancellable::NONE) {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        changed = is_newer(&info, since)
            || (info.file_type() == gio::FileType::Directory
                && tree_changed(&folder.child(info.name()), since, depth + 1, budget));
        if changed {
            break;
        }
    }
    let _ = listing.close(gio::Cancellable::NONE);
    changed
}

/// True when `info`'s modification time is later than `since`.
fn is_newer(info: &gio::FileInfo, since: u64) -> bool {
    info.modification_date_time()
        .and_then(|time| u64::try_from(time.to_unix()).ok())
        .is_some_and(|modified| modified > since)
}
