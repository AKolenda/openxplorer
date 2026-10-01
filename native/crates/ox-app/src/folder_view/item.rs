// SPDX-License-Identifier: AGPL-3.0-only
//! The list-model object for one listed item.
//!
//! Wraps an [`ox_core::entry::Entry`] with what the views need often and
//! should compute once: the natural-order sort keys, the lower-cased name
//! used by the search filter, and the icon art, and while searching the
//! folder the item is in. A row of `renderRows` in `v2.0.0:desktop/ui/app.js`
//! reads the same fields from the entry. A folder also carries its
//! measured size once the user asked for it (`state.folderSizes` in
//! app.js), which the Size column shows and sorts by.

use std::cell::OnceCell;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;
use ox_core::location::parent_location;
use ox_core::search::display_path;

use crate::folder_view::filter::Visibility;
use crate::folder_view::sorting::{SortKey, SortName};
use crate::icons::{Art, Emblems};
use crate::properties::FolderSizeState;

/// A listed entry with what the views compute from it once, when its
/// [`FileItem`] is created.
#[derive(Debug)]
struct PreparedEntry {
    name_sort_key: SortKey,
    type_sort_key: SortKey,
    lowercase_name: String,
    art: Art,
    emblems: Emblems,
    entry: Entry,
    /// Worked out the first time a search shows the item's folder.
    folder_path: OnceCell<FolderPath>,
}

/// Where an item is, as the Folder path column of a search shows it.
#[derive(Debug)]
pub(crate) struct FolderPath {
    /// The folder's display path, a UNC path for SMB
    /// (`displayUri(e.parentUri||parentUri(e.uri))` in app.js).
    pub text: String,
    /// Its natural-order key, as the column sorts it.
    pub key: SortKey,
}

impl FolderPath {
    /// The folder `entry` is in.
    fn of(entry: &Entry) -> Self {
        let folder = parent_location(&entry.uri).unwrap_or_else(|| entry.uri.clone());
        let text = display_path(&folder);
        let key = SortKey::new(&text);
        Self { text, key }
    }
}

impl PreparedEntry {
    fn new(entry: Entry) -> Self {
        Self {
            name_sort_key: SortKey::new(&entry.name),
            type_sort_key: SortKey::new(&entry.type_label),
            lowercase_name: entry.name.to_lowercase(),
            art: Art::for_entry(&entry),
            emblems: Emblems::for_entry(&entry),
            entry,
            folder_path: OnceCell::new(),
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::PreparedEntry;
    use crate::properties::FolderSizeState;

    /// Private state of [`super::FileItem`]: the entry, set once at
    /// construction, and a folder's measured size.
    #[derive(Debug, Default)]
    pub(crate) struct FileItem {
        /// The entry and what is computed from it, set by
        /// [`super::FileItem::new`].
        pub(super) prepared: OnceCell<PreparedEntry>,
        /// What a folder-size scan found, for a folder that was measured.
        pub(super) folder_size: RefCell<Option<FolderSizeState>>,
        /// How many items a folder holds, once counted (VIEW-037).
        pub(super) item_count: Cell<Option<u32>>,
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
    pub(crate) struct FileItem(ObjectSubclass<imp::FileItem>);
}

impl FileItem {
    /// Wraps a listed entry.
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

    /// Whether GIO marks the item hidden, for the filter.
    pub(crate) fn visibility(&self) -> Visibility {
        if self.entry().is_hidden {
            Visibility::Hidden
        } else {
            Visibility::Visible
        }
    }

    /// The folder the item is in, for the Folder path column of a search
    /// (VIEW-042). Worked out when first asked, as a folder listing never
    /// shows it.
    pub(crate) fn folder_path(&self) -> &FolderPath {
        let prepared = self.prepared();
        prepared
            .folder_path
            .get_or_init(|| FolderPath::of(&prepared.entry))
    }

    /// The icon art for the item.
    pub(crate) fn art(&self) -> Art {
        self.prepared().art
    }

    /// The emblems over the item's icon: a link, a lock.
    pub(crate) fn emblems(&self) -> Emblems {
        self.prepared().emblems
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

    /// What a folder-size scan found for this folder, if it was measured.
    pub(crate) fn folder_size(&self) -> Option<FolderSizeState> {
        self.imp().folder_size.borrow().clone()
    }

    /// Records what a folder-size scan found for this folder.
    pub(crate) fn set_folder_size(&self, state: FolderSizeState) {
        self.imp().folder_size.replace(Some(state));
    }

    /// How many items the folder holds, once counted.
    pub(crate) fn item_count(&self) -> Option<u32> {
        self.imp().item_count.get()
    }

    /// Records how many items the folder holds.
    pub(crate) fn set_item_count(&self, count: u32) {
        self.imp().item_count.set(Some(count));
    }

    /// The size the Size column sorts by: a file's size, a folder's
    /// measured size, else 0 (`itemSize` in app.js).
    pub(crate) fn sort_size(&self) -> u64 {
        if self.entry().is_dir {
            return self
                .imp()
                .folder_size
                .borrow()
                .as_ref()
                .map_or(0, FolderSizeState::sort_bytes);
        }
        self.entry().size.unwrap_or(0)
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
