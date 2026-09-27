// SPDX-License-Identifier: AGPL-3.0-only
//! Merging a new listing into a tab's items without emptying the view.
//!
//! A reload of the same folder (F5, a change the directory monitor saw)
//! keeps the rows on screen until the new listing is complete, as
//! `load(t, false)` in `desktop/ui/app.js` does. GTK's list views follow the
//! item objects for their scroll anchor, keyboard focus and selection, so
//! the listing is merged by URI: items that are gone are removed, new ones
//! are appended (the views sort them), and only items whose details changed
//! are replaced. Every other [`FileItem`] stays the same object. Dolphin's
//! `KDirLister` reports added, deleted and refreshed items the same way.

use std::collections::HashMap;

use gtk::gio;
use gtk::prelude::*;
use ox_core::entry::Entry;

use crate::folder_view::item::FileItem;

/// The new listing, looked up by URI.
struct Listing {
    entries: Vec<Option<Entry>>,
    positions: HashMap<String, usize>,
}

impl Listing {
    fn new(entries: Vec<Entry>) -> Self {
        let positions = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.uri.clone(), position))
            .collect();
        let entries = entries.into_iter().map(Some).collect();
        Self { entries, positions }
    }

    /// Takes the entry for `uri` out of the listing, if it is listed.
    fn take(&mut self, uri: &str) -> Option<Entry> {
        let position = *self.positions.get(uri)?;
        self.entries[position].take()
    }

    /// The entries nothing took, in listing order.
    fn into_remaining(self) -> Vec<FileItem> {
        self.entries.into_iter().flatten().map(FileItem::new).collect()
    }
}

/// Removes `count` items starting at `start`.
fn remove_run(store: &gio::ListStore, start: u32, count: u32) {
    if count > 0 {
        store.splice(start, count, &[] as &[FileItem]);
    }
}

/// Makes `store` hold exactly `entries`, keeping the item objects of
/// entries that did not change.
pub(crate) fn update_in_place(store: &gio::ListStore, entries: Vec<Entry>) {
    let mut listing = Listing::new(entries);
    // Walk from the end so removals never shift a position still to visit;
    // neighbouring removals are merged into one change.
    let mut removals = 0;
    for position in (0..store.n_items()).rev() {
        let item = store
            .item(position)
            .and_downcast::<FileItem>()
            .expect("tab stores hold FileItems");
        let Some(entry) = listing.take(&item.entry().uri) else {
            removals += 1;
            continue;
        };
        remove_run(store, position + 1, removals);
        removals = 0;
        if entry != *item.entry() {
            store.splice(position, 1, &[FileItem::new(entry)]);
        }
    }
    remove_run(store, 0, removals);
    store.extend_from_slice(&listing.into_remaining());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    fn store_with(names: &[&str]) -> gio::ListStore {
        let store = gio::ListStore::new::<FileItem>();
        for name in names {
            store.append(&FileItem::new(file_entry(name)));
        }
        store
    }

    fn items(store: &gio::ListStore) -> Vec<FileItem> {
        store
            .iter::<FileItem>()
            .map(|item| item.expect("the store is not changed while iterating"))
            .collect()
    }

    fn names(store: &gio::ListStore) -> Vec<String> {
        let mut names: Vec<String> = items(store)
            .iter()
            .map(|item| item.entry().name.clone())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_reload_keeps_unchanged_items_as_the_same_objects() {
        let store = store_with(&["a.txt", "b.txt"]);
        let before = items(&store);
        update_in_place(&store, vec![file_entry("a.txt"), file_entry("b.txt")]);
        assert_eq!(items(&store), before, "unchanged rows keep their objects");
    }

    #[test]
    fn removed_entries_disappear_and_new_ones_are_added() {
        let store = store_with(&["a.txt", "b.txt", "c.txt", "d.txt"]);
        let kept = items(&store)[2].clone();
        update_in_place(&store, vec![file_entry("c.txt"), file_entry("e.txt")]);
        assert_eq!(names(&store), ["c.txt", "e.txt"]);
        assert!(items(&store).contains(&kept), "c.txt is still the same object");
    }

    #[test]
    fn changed_entries_are_replaced() {
        let store = store_with(&["a.txt"]);
        let before = items(&store);
        let mut changed = file_entry("a.txt");
        changed.size = Some(4096);
        update_in_place(&store, vec![changed]);
        let after = items(&store);
        assert_ne!(after, before, "a changed entry gets a new item");
        assert_eq!(after[0].entry().size, Some(4096));
    }

    #[test]
    fn an_empty_listing_empties_the_store() {
        let store = store_with(&["a.txt", "b.txt"]);
        update_in_place(&store, Vec::new());
        assert_eq!(store.n_items(), 0);
    }
}
