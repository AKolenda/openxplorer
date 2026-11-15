// SPDX-License-Identifier: AGPL-3.0-only
//! The "Gallery Locations" dialog of Collection > Manage collection
//! (HOME-025).

use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::GalleryLocations`].
    #[derive(Debug, Default)]
    pub(crate) struct GalleryLocations {}

    #[glib::object_subclass]
    impl ObjectSubclass for GalleryLocations {
        const NAME: &'static str = "OxGalleryLocations";
        type Type = super::GalleryLocations;
        type ParentType = gtk::Window;
    }

    impl ObjectImpl for GalleryLocations {}
    impl WidgetImpl for GalleryLocations {}
    impl WindowImpl for GalleryLocations {}
}

glib::wrapper! {
    /// The "Gallery Locations" dialog.
    pub(crate) struct GalleryLocations(ObjectSubclass<imp::GalleryLocations>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

#[cfg(test)]
impl GalleryLocations {
    pub(crate) fn heading_text(&self) -> String {
        let _ = self.imp();
        String::new()
    }
    pub(crate) fn body_text(&self) -> String {
        String::new()
    }
    pub(crate) fn list_label(&self) -> String {
        String::new()
    }
    pub(crate) fn rows(&self) -> Vec<(String, String, String)> {
        Vec::new()
    }
    pub(crate) fn select_row(&self, _index: usize) {}
    pub(crate) fn can_remove(&self) -> bool {
        false
    }
    pub(crate) fn remove_selected(&self) {}
    pub(crate) fn add_folder(&self, _uri: &str) {}
    pub(crate) fn ok(&self) {}
    pub(crate) fn cancel(&self) {}
}
