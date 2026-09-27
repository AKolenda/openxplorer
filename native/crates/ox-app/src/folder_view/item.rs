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
use crate::icons::ArtKind;

mod imp {
    use super::*;

    /// Private state of [`super::FileItem`]; set once at construction.
    #[derive(Default)]
    pub struct FileItem {
        pub entry: OnceCell<Entry>,
        pub name_key: OnceCell<String>,
        pub type_key: OnceCell<String>,
        pub lower_name: OnceCell<String>,
        pub art: OnceCell<ArtKind>,
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
    pub fn new(entry: Entry) -> Self {
        let item: Self = glib::Object::new();
        let imp = item.imp();
        let _ = imp.name_key.set(sorting::sort_key(&entry.name));
        let _ = imp.type_key.set(sorting::sort_key(&entry.type_label));
        let _ = imp.lower_name.set(entry.name.to_lowercase());
        let _ = imp.art.set(crate::icons::art::kind_for_entry(&entry));
        let _ = imp.entry.set(entry);
        item
    }

    /// The listed entry.
    pub fn entry(&self) -> &Entry {
        self.imp().entry.get().expect("set in FileItem::new")
    }

    /// Natural-order key of the name.
    pub fn name_key(&self) -> &str {
        self.imp().name_key.get().expect("set in FileItem::new")
    }

    /// Natural-order key of the type label.
    pub fn type_key(&self) -> &str {
        self.imp().type_key.get().expect("set in FileItem::new")
    }

    /// Lower-cased name for the search filter.
    pub fn lower_name(&self) -> &str {
        self.imp().lower_name.get().expect("set in FileItem::new")
    }

    /// The icon art for the item.
    pub fn art(&self) -> &ArtKind {
        self.imp().art.get().expect("set in FileItem::new")
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
