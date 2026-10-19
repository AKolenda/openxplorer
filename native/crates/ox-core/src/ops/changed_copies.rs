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
//! huge copied tree cannot hold Undo back for long. An incomplete check
//! requires confirmation too: unread entries cannot be assumed unchanged.

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
/// Unix epoch), or could not be checked completely. Empty for every other step.
pub async fn changed_copies(record: &UndoRecord, since: u64) -> Vec<String> {
    let (UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies }) = record else {
        return Vec::new();
    };
    let pending = copies.clone();
    on_worker(move || Ok(changed_copies_blocking(&pending, since, MAX_CHECKED)))
        .await
        .unwrap_or_else(|_| {
            copies
                .iter()
                .map(|uri| copy_name(&gio::File::for_uri(uri)))
                .collect()
        })
}

/// [`changed_copies`] of `copies` on the calling thread.
fn changed_copies_blocking(copies: &[String], since: u64, mut budget: usize) -> Vec<String> {
    copies
        .iter()
        .filter_map(|uri| {
            let copy = gio::File::for_uri(uri);
            let Ok(info) = copy.query_info(
                ATTRIBUTES,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            ) else {
                return Some(copy_name(&copy));
            };
            let changed = is_newer(&info, since)
                || (info.file_type() == gio::FileType::Directory
                    && tree_changed(&copy, since, 1, &mut budget));
            changed.then(|| info.display_name().to_string())
        })
        .collect()
}

/// A displayable fallback when metadata could not be read.
fn copy_name(copy: &gio::File) -> String {
    copy.basename().map_or_else(
        || copy.uri().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// True when anything inside `folder`, `depth` levels below a copy, was
/// modified after `since`, or the check is incomplete. Each item looked
/// at uses up one of `budget`.
fn tree_changed(folder: &gio::File, since: u64, depth: usize, budget: &mut usize) -> bool {
    if depth > MAX_DEPTH || *budget == 0 {
        return true;
    }
    let Ok(listing) = folder.enumerate_children(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    ) else {
        return true;
    };
    let changed = loop {
        let info = match listing.next_file(gio::Cancellable::NONE) {
            Ok(Some(info)) => info,
            Ok(None) => break false,
            Err(_) => break true,
        };
        if *budget == 0 {
            break true;
        }
        *budget -= 1;
        let changed = is_newer(&info, since)
            || (info.file_type() == gio::FileType::Directory
                && tree_changed(&folder.child(info.name()), since, depth + 1, budget));
        if changed {
            break true;
        }
    };
    let _ = listing.close(gio::Cancellable::NONE);
    changed
}

/// True when `info`'s modification time is later than `since`.
fn is_newer(info: &gio::FileInfo, since: u64) -> bool {
    info.modification_date_time()
        .and_then(|time| u64::try_from(time.to_unix()).ok())
        .is_some_and(|modified| modified > since)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn exhausting_the_budget_requires_confirmation_for_remaining_copies() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first-copy");
        let second = temporary.path().join("second-copy");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("unchanged.txt"), "original").unwrap();
        fs::write(second.join("edited.txt"), "new work").unwrap();
        let since = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        fs::File::open(second.join("edited.txt"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(since + 1))
            .unwrap();
        let uris = [first, second].map(|path| gio::File::for_path(path).uri().to_string());

        assert_eq!(changed_copies_blocking(&uris[1..], since, 1), ["second-copy"]);
        assert_eq!(changed_copies_blocking(&uris, since, 1), ["second-copy"]);
    }

    #[test]
    fn failed_or_depth_limited_checks_require_confirmation() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = gio::File::for_path(temporary.path());
        assert!(tree_changed(&folder, u64::MAX, MAX_DEPTH + 1, &mut 10));
        let missing = folder.child("unavailable");
        assert!(tree_changed(&missing, u64::MAX, 1, &mut 10));
        assert_eq!(
            changed_copies_blocking(&[missing.uri().to_string()], u64::MAX, 10),
            ["unavailable"]
        );
    }
}
