// SPDX-License-Identifier: AGPL-3.0-only
//! Which copies changed since their copy finished (OPS-030).
//!
//! Undoing a copy moves the copies to the Trash. As Dolphin's "Undo File
//! Copy Confirmation" does, the window asks first when a copy was
//! modified after the copy, so work done in it is not thrown away
//! unnoticed. [`changed_copies`] finds them: the copies of a Copy or
//! Duplicate step whose modification time, read without following links,
//! is later than when the step was recorded.

use gio::prelude::*;

use super::undo::UndoRecord;

/// The names of the copies `record` would move to the Trash that were
/// modified after `since` (seconds since the Unix epoch). Empty for
/// every other step, and for copies that cannot be read.
pub async fn changed_copies(record: &UndoRecord, since: u64) -> Vec<String> {
    let (UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies }) = record else {
        return Vec::new();
    };
    let mut changed = Vec::new();
    for uri in copies {
        let file = gio::File::for_uri(uri);
        let info = file
            .query_info_future(
                "standard::display-name,time::modified",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await;
        let Ok(info) = info else {
            continue;
        };
        let modified = info
            .modification_date_time()
            .and_then(|time| u64::try_from(time.to_unix()).ok());
        if modified.is_some_and(|modified| modified > since) {
            changed.push(info.display_name().to_string());
        }
    }
    changed
}
