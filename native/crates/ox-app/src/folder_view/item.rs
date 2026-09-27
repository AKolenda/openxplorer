// SPDX-License-Identifier: AGPL-3.0-only
//! The list-model object for one listed item.
//!
//! Wraps an [`ox_core::entry::Entry`] with what the views need often and
//! should compute once: the natural-order sort keys, the lower-cased name
//! used by the search filter, and the icon art kind.

use std::cell::OnceCell;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;

use crate::folder_view::sorting;
use crate::icons::{art, ArtKind};

/// Everything a [`FileItem`] holds, computed once when it is created.
#[derive(Debug)]
pub struct ItemData {
    name_key: String,
    type_key: String,
    lower_name: String,
    art: ArtKind,
    entry: Entry,
}

impl ItemData {
    fn new(entry: Entry) -> Self {
        Self {
            name_key: sorting::sort_key(&entry.name),
            type_key: sorting::sort_key(&entry.type_label),
            lower_name: entry.name.to_lowercase(),
            art: art::kind_for_entry(&entry),
            entry,
        }
    }
}

mod imp {
    use super::*;

    /// Private state of [`super::FileItem`]; set once at construction.
    #[derive(Default)]
    pub struct FileItem {
        pub data: OnceCell<ItemData>,
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
    /// Never: a new object has no data yet.
    pub fn new(entry: Entry) -> Self {
        let item: Self = glib::Object::new();
        item.imp()
            .data
            .set(ItemData::new(entry))
            .expect("a new FileItem has no data yet");
        item
    }

    fn data(&self) -> &ItemData {
        self.imp()
            .data
            .get()
            .expect("FileItem::new is the only constructor and sets the data")
    }

    /// The listed entry.
    pub fn entry(&self) -> &Entry {
        &self.data().entry
    }

    /// Natural-order key of the name.
    pub fn name_key(&self) -> &str {
        &self.data().name_key
    }

    /// Natural-order key of the type label.
    pub fn type_key(&self) -> &str {
        &self.data().type_key
    }

    /// Lower-cased name for the search filter.
    pub fn lower_name(&self) -> &str {
        &self.data().lower_name
    }

    /// The icon art for the item.
    pub fn art(&self) -> ArtKind {
        self.data().art
    }

    /// Where activating the item goes: a virtual folder's target, or the
    /// item itself.
    pub fn open_uri(&self) -> &str {
        let entry = self.entry();
        match (&entry.target_uri, entry.is_virtual) {
            (Some(target), true) => target,
            _ => &entry.uri,
        }
    }
}
