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
use std::path::Path;
use std::rc::Rc;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;
use ox_core::location::{is_archive_location, is_smb_share_root, parent_location, scheme, LocationContext};
use ox_core::search::display_path;

use crate::folder_view::filter::Visibility;
use crate::folder_view::groups::{Group, GroupClocks, Grouping};
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
    /// The art with no icon chosen in File icons.
    art: Art,
    emblems: Emblems,
    entry: Entry,
    /// Worked out the first time a search shows the item's folder.
    folder_path: OnceCell<FolderPath>,
    /// Worked out the first time the Recycle Bin shows where the item
    /// was deleted from.
    original_location: OnceCell<FolderPath>,
    /// Worked out the first time the file commands ask.
    address_facts: OnceCell<AddressFacts>,
}

/// What the file commands read from an item's address (`canOperate` and
/// `readonlyLocation` in app.js). Reading it means splitting and decoding
/// the address, about five microseconds an item: with 100,000 items
/// selected, worked out again whenever the commands were updated, it froze
/// the window for seconds after Select all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AddressFacts {
    /// A whole SMB server or share, which cannot be moved, renamed or
    /// trashed.
    pub smb_share_root: bool,
    /// Read-only by its path alone: a snapshot component (`.snapshot`,
    /// `@GMT-…`) or inside a ZIP. A configured snapshot root is the
    /// window's to check, as it can change while the item is listed.
    pub read_only_by_path: bool,
    /// A row of Recent files.
    pub recent_row: bool,
}

impl AddressFacts {
    /// The facts of the item at `uri`.
    pub(crate) fn of(uri: &str) -> Self {
        #[cfg(test)]
        ADDRESS_READS.with(|reads| reads.set(reads.get() + 1));
        Self {
            smb_share_root: is_smb_share_root(uri),
            // With no snapshot roots configured, only the path decides.
            read_only_by_path: LocationContext::default().is_snapshot_location(uri)
                || is_archive_location(uri),
            recent_row: scheme(uri).as_deref() == Some("recent"),
        }
    }
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
        // A Recycle Bin item found by a search is where it was deleted
        // from, not in the Recycle Bin's own "/".
        if entry.trash_orig_path.is_some() {
            return Self::original_of(entry);
        }
        let folder = parent_location(&entry.uri).unwrap_or_else(|| entry.uri.clone());
        Self::shown_as(display_path(&folder))
    }

    /// The folder a Recycle Bin item was deleted from, or nothing for an
    /// item that is not in it.
    fn original_of(entry: &Entry) -> Self {
        let folder = entry.trash_orig_path.as_deref().and_then(Path::parent);
        Self::shown_as(
            folder
                .map(|folder| folder.display().to_string())
                .unwrap_or_default(),
        )
    }

    fn shown_as(text: String) -> Self {
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
            art: Art::usual_for_entry(&entry),
            emblems: Emblems::for_entry(&entry),
            entry,
            folder_path: OnceCell::new(),
            original_location: OnceCell::new(),
            address_facts: OnceCell::new(),
        }
    }
}

/// A group worked out for an item, kept with what it was worked out under.
type KeptGroup = (Grouping, GroupClocks, Rc<Group>);

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
        /// The group the item was last put in, with the grouping and the
        /// clocks it was put in under.
        pub(super) group: RefCell<Option<super::KeptGroup>>,
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

    /// The folder a Recycle Bin item was deleted from, for its Original
    /// location column (VIEW-062).
    pub(crate) fn original_location(&self) -> &FolderPath {
        let prepared = self.prepared();
        prepared
            .original_location
            .get_or_init(|| FolderPath::original_of(&prepared.entry))
    }

    /// What the file commands read from the item's address, worked out
    /// once.
    pub(crate) fn address_facts(&self) -> AddressFacts {
        let prepared = self.prepared();
        *prepared
            .address_facts
            .get_or_init(|| AddressFacts::of(&prepared.entry.uri))
    }

    /// The icon art for the item, as File icons now has it (LOOK-029).
    pub(crate) fn art(&self) -> Art {
        let prepared = self.prepared();
        crate::icons::file_icons::art_for(&prepared.entry.name, prepared.art)
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
        // Its size group may have changed.
        self.imp().group.replace(None);
    }

    /// The group of the item under `grouping` with `clocks`, worked out by
    /// `work_out` the first time and kept while they stay the same.
    ///
    /// Sorting 100,000 items into groups compares about two million pairs;
    /// working both items' groups out for each comparison (a title, often
    /// normalized text) froze the window for about a second.
    pub(crate) fn group_in(
        &self,
        grouping: Grouping,
        clocks: &GroupClocks,
        work_out: impl FnOnce() -> Group,
    ) -> Rc<Group> {
        if let Some((kept_grouping, kept_clocks, group)) = self.imp().group.borrow().as_ref() {
            if *kept_grouping == grouping && kept_clocks == clocks {
                return Rc::clone(group);
            }
        }
        let group = Rc::new(work_out());
        self.imp()
            .group
            .replace(Some((grouping, *clocks, Rc::clone(&group))));
        group
    }

    /// How many items the folder holds, once counted.
    pub(crate) fn item_count(&self) -> Option<u32> {
        self.imp().item_count.get()
    }

    /// Records how many items the folder holds.
    pub(crate) fn set_item_count(&self, count: u32) {
        self.imp().item_count.set(Some(count));
        self.imp().group.replace(None);
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

#[cfg(test)]
thread_local! {
    /// How many addresses [`AddressFacts::of`] read, for tests.
    pub(crate) static ADDRESS_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// parity: VIEW-062
    #[test]
    fn a_recycle_bin_item_s_folder_path_is_where_it_was_deleted_from() {
        let mut entry = crate::test_support::file_entry("Budget.ods");
        entry.uri = String::from("trash:///Budget.ods");
        entry.trash_orig_path = Some(PathBuf::from("/home/user/Documents/Budget.ods"));
        let other = crate::test_support::file_entry("Notes.txt");

        assert_eq!(FolderPath::of(&entry).text, "/home/user/Documents");
        assert_eq!(
            FolderPath::of(&other).text,
            FolderPath::shown_as(display_path(&parent_location(&other.uri).unwrap())).text
        );
    }
}
