// SPDX-License-Identifier: AGPL-3.0-only
//! The list-model object for one listed item.
//!
//! Wraps an [`ox_core::entry::Entry`] with what the views need often and
//! should compute once: the natural-order sort keys, the lower-cased name
//! used by the search filter, and the icon art kind. A row of `renderRows`
//! in `desktop/ui/app.js` reads the same fields from the entry.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;

use crate::folder_view::sorting::{SortKey, SortName};
use crate::icons::{art, ArtKind};

/// A listed entry with what the views compute from it once, when its
/// [`FileItem`] is created.
#[derive(Debug)]
struct PreparedEntry {
    name_sort_key: SortKey,
    type_sort_key: SortKey,
    lowercase_name: String,
    art: ArtKind,
    entry: Entry,
}

impl PreparedEntry {
    fn new(entry: Entry) -> Self {
        Self {
            name_sort_key: SortKey::new(&entry.name),
            type_sort_key: SortKey::new(&entry.type_label),
            lowercase_name: entry.name.to_lowercase(),
            art: art::kind_for_entry(&entry),
            entry,
        }
    }
}

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::PreparedEntry;

    /// Private state of [`super::FileItem`]; set once at construction.
    #[derive(Default)]
    pub struct FileItem {
        /// The entry and what is computed from it, set by
        /// [`super::FileItem::new`].
        pub(super) prepared: OnceCell<PreparedEntry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileItem {
        const NAME: &'static str = "OxFileItem";
        type Type = super::FileItem;
    }

    impl ObjectImpl for FileItem {}
}

glib::wrapper! {
    /// One row of a folder listing.
    pub struct FileItem(ObjectSubclass<imp::FileItem>);
}

impl FileItem {
    /// Wraps a listed entry.
    ///
    /// # Panics
    ///
    /// Never: a new object has no entry yet.
    pub(crate) fn new(entry: Entry) -> Self {
        let item: Self = glib::Object::new();
        item.imp()
            .prepared
            .set(PreparedEntry::new(entry))
            .expect("a new FileItem has no entry yet");
        item
    }

    fn prepared(&self) -> &PreparedEntry {
        self.imp()
            .prepared
            .get()
            .expect("FileItem::new is the only constructor and sets the entry")
    }

    /// The listed entry.
    pub(crate) fn entry(&self) -> &Entry {
        &self.prepared().entry
    }

    /// The name with its natural-order key, as the Name column sorts it.
    pub(crate) fn sort_name(&self) -> SortName<'_> {
        let prepared = self.prepared();
        SortName {
            key: &prepared.name_sort_key,
            name: &prepared.entry.name,
        }
    }

    /// Natural-order key of the type label.
    pub(crate) fn type_sort_key(&self) -> &SortKey {
        &self.prepared().type_sort_key
    }

    /// Lower-cased name for the search filter.
    pub(crate) fn lowercase_name(&self) -> &str {
        &self.prepared().lowercase_name
    }

    /// The icon art for the item.
    pub(crate) fn art(&self) -> ArtKind {
        self.prepared().art
    }

    /// The size of a file; `None` for folders and for files of unknown
    /// size, which show an empty Size cell and add nothing to a selection's
    /// total.
    pub(crate) fn file_size(&self) -> Option<u64> {
        let entry = self.entry();
        if entry.is_dir {
            return None;
        }
        entry.size
    }
}

/// A tab's store holding a file called each of `names`, for tests of the
/// models built over it.
#[cfg(test)]
pub(crate) fn store_of_files(names: &[&str]) -> gtk::gio::ListStore {
    let store = gtk::gio::ListStore::new::<FileItem>();
    for name in names {
        store.append(&FileItem::new(crate::test_support::file_entry(name)));
    }
    store
}
