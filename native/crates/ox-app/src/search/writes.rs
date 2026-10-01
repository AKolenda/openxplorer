// SPDX-License-Identifier: AGPL-3.0-only
//! Which folders a file operation changed, for the search cache.
//!
//! Ports `invalidate_cache_for_write` in `v2.0.0:desktop/winspace.py`
//! (SRCH-033): after every write, the folder written into and the folders
//! that held the changed items are read again in every indexed folder that
//! holds them, so a search shows the app's own changes at once, also on a
//! share that has no live watch.

use std::collections::HashSet;

use ox_core::location::parent_location;

/// The folders an operation on `items` changed: each of `folders` (the
/// destination, the folder shown), then the folder of each item, once
/// each and in that order.
pub(crate) fn changed_folders<'a>(
    folders: impl IntoIterator<Item = &'a str>,
    items: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let parents = items.into_iter().filter_map(parent_location);
    let candidates = folders.into_iter().map(str::to_owned).chain(parents);
    let mut seen = HashSet::new();
    candidates
        .filter(|folder| seen.insert(folder.trim_end_matches('/').to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A move from two folders into a third changes all three; a folder
    /// is read once however many items it held.
    ///
    /// parity: SRCH-033
    #[test]
    fn a_move_changes_its_destination_and_every_source_folder() {
        let moved = [
            "smb://nas/share/Inbox/a.pdf",
            "smb://nas/share/Inbox/b.pdf",
            "smb://nas/share/Scans/c.pdf",
        ];

        let folders = changed_folders(["smb://nas/share/Archive/"], moved);

        assert_eq!(
            folders,
            [
                "smb://nas/share/Archive/",
                "smb://nas/share/Inbox",
                "smb://nas/share/Scans"
            ]
        );
    }
}
