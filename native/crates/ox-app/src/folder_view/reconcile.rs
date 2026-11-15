// SPDX-License-Identifier: AGPL-3.0-only
//! Merging a new listing into a tab's items without emptying the view.
//!
//! A reload of the same folder (F5, a change the directory monitor saw)
//! keeps the rows on screen until the new listing is complete, as
//! `load(t, false)` in `v2.0.0:desktop/ui/app.js` does. GTK's list views follow the
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

/// The new listing, looked up by URI. The store's items take their entries
/// out of it; what nothing takes is new.
struct NewListing {
    /// The listed entries in listing order; `None` once taken.
    entries: Vec<Option<Entry>>,
    /// Where each URI is in `entries`.
    positions: HashMap<String, usize>,
}

impl NewListing {
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
        let untaken = self.entries.into_iter().flatten();
        untaken.map(FileItem::new).collect()
    }
}

/// Removes `count` items starting at `start`.
fn remove_run(store: &gio::ListStore, start: u32, count: u32) {
    if count > 0 {
        store.splice(start, count, &[] as &[FileItem]);
    }
}

/// Replaces the items from `start` with `replacements`, which hold them in
/// reverse order, in one change.
fn replace_run(store: &gio::ListStore, start: u32, replacements: &mut Vec<FileItem>) {
    if replacements.is_empty() {
        return;
    }
    replacements.reverse();
    let count = u32::try_from(replacements.len()).expect("a run is part of a list model");
    store.splice(start, count, replacements);
    replacements.clear();
}

/// Makes `store` hold exactly `entries`, keeping the item objects of
/// entries that did not change.
///
/// Every change of the store costs the sorted list and the views over it
/// a pass over all items, so neighboring removals and neighboring
/// changed items are each made in one change, and the new items are added
/// together.
///
/// # Panics
///
/// If `store` holds anything but [`FileItem`]s; tab stores are made with
/// `gio::ListStore::new::<FileItem>()`.
pub(crate) fn update_in_place(store: &gio::ListStore, entries: Vec<Entry>) {
    let mut listing = NewListing::new(entries);
    // Walk from the end so removals never shift a position still to visit.
    // `gone_run` counts the gone items directly after `position`, and
    // `changed_run` holds the replacements of the changed items directly
    // after it, last first.
    let mut gone_run = 0;
    let mut changed_run = Vec::new();
    for position in (0..store.n_items()).rev() {
        let item = store
            .item(position)
            .and_downcast::<FileItem>()
            .expect("tab stores hold FileItems");
        let Some(entry) = listing.take(&item.entry().uri) else {
            replace_run(store, position + 1, &mut changed_run);
            gone_run += 1;
            continue;
        };
        remove_run(store, position + 1, gone_run);
        gone_run = 0;
        if entry == *item.entry() {
            replace_run(store, position + 1, &mut changed_run);
        } else {
            changed_run.push(FileItem::new(entry));
        }
    }
    replace_run(store, 0, &mut changed_run);
    remove_run(store, 0, gone_run);
    let new_items = listing.into_remaining();
    if !new_items.is_empty() {
        store.extend_from_slice(&new_items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder_view::item::store_of_files;
    use crate::test_support::file_entry;

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

    /// parity: NAV-013
    #[test]
    fn a_reload_keeps_unchanged_items_as_the_same_objects() {
        let store = store_of_files(&["a.txt", "b.txt"]);
        let before = items(&store);
        update_in_place(&store, vec![file_entry("a.txt"), file_entry("b.txt")]);
        assert_eq!(items(&store), before, "unchanged rows keep their objects");
    }

    /// parity: NAV-013
    #[test]
    fn removed_entries_disappear_and_new_ones_are_added() {
        let store = store_of_files(&["a.txt", "b.txt", "c.txt", "d.txt"]);
        let kept = items(&store)[2].clone();
        update_in_place(&store, vec![file_entry("c.txt"), file_entry("e.txt")]);
        assert_eq!(names(&store), ["c.txt", "e.txt"]);
        assert!(items(&store).contains(&kept), "c.txt is still the same object");
    }

    #[test]
    fn changed_entries_are_replaced() {
        let store = store_of_files(&["a.txt"]);
        let before = items(&store);
        let mut changed = file_entry("a.txt");
        changed.size = Some(4096);
        update_in_place(&store, vec![changed]);
        let after = items(&store);
        assert_ne!(after, before, "a changed entry gets a new item");
        assert_eq!(after[0].entry().size, Some(4096));
    }

    /// Every change of the store costs the sorted list a pass over all its
    /// items, which for a folder of 100,000 items reloaded after another
    /// program changed it froze the window for seconds. Neighboring
    /// changes are made together, and a reload with nothing changed makes
    /// no change at all.
    ///
    /// parity: PERF-006
    #[test]
    fn neighboring_changes_are_made_in_one_change() {
        let names: Vec<String> = (0..10).map(|number| format!("{number}.txt")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let store = store_of_files(&names);
        let changes = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = std::rc::Rc::clone(&changes);
        store.connect_items_changed(move |_, _, _, _| counter.set(counter.get() + 1));
        let entries = |sized: &[usize], gone: &[usize], new: &[&str]| -> Vec<Entry> {
            let mut entries: Vec<Entry> = (0..10)
                .filter(|number| !gone.contains(number))
                .map(|number| {
                    let mut entry = file_entry(&format!("{number}.txt"));
                    if sized.contains(&number) {
                        entry.size = Some(4096);
                    }
                    entry
                })
                .collect();
            entries.extend(new.iter().map(|name| file_entry(name)));
            entries
        };

        update_in_place(&store, entries(&[], &[], &[]));
        assert_eq!(changes.get(), 0, "nothing changed");

        update_in_place(
            &store,
            entries(&[2, 3, 4, 5], &[7, 8], &["new 1.txt", "new 2.txt"]),
        );
        assert_eq!(
            changes.get(),
            3,
            "one change each: the changed, the gone and the new items"
        );
        assert_eq!(store.n_items(), 10);
        let sizes: Vec<Option<u64>> = items(&store).iter().map(|item| item.entry().size).collect();
        assert_eq!(sizes[2..6], [Some(4096); 4]);
        assert_eq!(items(&store)[6].entry().name, "6.txt");
        assert_eq!(items(&store)[7].entry().name, "9.txt");
    }

    #[test]
    fn an_empty_listing_empties_the_store() {
        let store = store_of_files(&["a.txt", "b.txt"]);
        update_in_place(&store, Vec::new());
        assert_eq!(store.n_items(), 0);
    }
}
