// SPDX-License-Identifier: AGPL-3.0-only
//! Custom icons (PROP-016): an item whose `GVfs` metadata names a picture
//! (`metadata::custom-icon`, which Files reads and writes too) shows that
//! picture in place of its art in the details and icon views. The lookup
//! runs when a cell is bound, at low priority, like a thumbnail lookup,
//! and a picture that arrives after the cell shows another item is
//! dropped.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use super::FileCell;
use crate::folder_view::item::FileItem;

/// The metadata key Files stores a custom icon under.
pub(crate) const CUSTOM_ICON: &str = "metadata::custom-icon";
/// Pictures larger than this are not decoded for an icon.
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;

impl FileCell {
    /// Shows the art again, then looks up `item`'s custom icon.
    pub(crate) fn look_up_custom_icon(&self, item: &FileItem) {
        let imp = self.imp();
        let lookup = imp.icon_lookup.get().wrapping_add(1);
        imp.icon_lookup.set(lookup);
        imp.custom_icon.set_visible(false);
        imp.image.set_visible(true);
        let uri = item.entry().uri.clone();
        if !uri.starts_with("file:") {
            return;
        }
        let cell = self.downgrade();
        glib::spawn_future_local(async move {
            let file = gio::File::for_uri(&uri);
            let info = file
                .query_info_future(CUSTOM_ICON, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW)
                .await;
            let Some(icon_uri) = info.ok().and_then(|info| info.attribute_string(CUSTOM_ICON)) else {
                return;
            };
            if icon_uri.is_empty() {
                return;
            }
            let icon_uri = icon_uri.to_string();
            let texture = gio::spawn_blocking(move || decode(&icon_uri))
                .await
                .ok()
                .flatten();
            let (Some(cell), Some(texture)) = (cell.upgrade(), texture) else {
                return;
            };
            let imp = cell.imp();
            if imp.icon_lookup.get() == lookup {
                imp.custom_icon.set_paintable(Some(&texture));
                imp.custom_icon.set_visible(true);
                imp.image.set_visible(false);
            }
        });
    }

    /// Whether the cell shows a custom icon, for tests.
    #[cfg(test)]
    pub(crate) fn shows_custom_icon(&self) -> bool {
        self.imp().custom_icon.is_visible()
    }
}

/// The picture at `uri`, if it is a local image of a sensible size.
fn decode(uri: &str) -> Option<gdk::Texture> {
    let file = gio::File::for_uri(uri);
    let path = file.path()?;
    let size = std::fs::metadata(&path).ok()?.len();
    (size <= MAX_ICON_BYTES)
        .then(|| gdk::Texture::from_file(&file).ok())
        .flatten()
}
