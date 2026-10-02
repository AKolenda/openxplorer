// SPDX-License-Identifier: AGPL-3.0-only
//! Custom icons (PROP-016): an item whose `GVfs` metadata names a picture
//! (`metadata::custom-icon`, which Files reads and writes too) shows that
//! picture in place of its art in the details and icon views. The lookup
//! runs when a cell is bound, at low priority, like a thumbnail lookup,
//! and a picture that arrives after the cell shows another item is
//! dropped. Pictures are decoded once at icon size and kept, so scrolling
//! back to an item does not decode its picture again. An item without a
//! custom icon shows its preview in the same place, when it has one
//! ([`crate::thumbnails`]).

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use super::FileCell;
use crate::folder_view::item::FileItem;
use crate::thumbnails::{self, PreviewPolicy, PreviewRequest};

/// The metadata key Files stores a custom icon under.
pub(crate) const CUSTOM_ICON: &str = "metadata::custom-icon";
/// Pictures larger than this are not decoded for an icon.
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;
/// The size a picture is decoded at: the largest icon the views draw.
const ICON_PIXELS: i32 = 256;
/// How many decoded pictures are kept.
const MAX_CACHED_ICONS: usize = 256;

thread_local! {
    /// Decoded pictures by their URI.
    static DECODED: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// The decoded picture at `icon_uri`, decoding it off the main thread the
/// first time.
async fn icon_texture(icon_uri: String) -> Option<gdk::Texture> {
    if let Some(texture) = DECODED.with_borrow(|decoded| decoded.get(&icon_uri).cloned()) {
        return Some(texture);
    }
    let uri = icon_uri.clone();
    let texture = gio::spawn_blocking(move || decode(&uri)).await.ok().flatten()?;
    DECODED.with_borrow_mut(|decoded| {
        if decoded.len() >= MAX_CACHED_ICONS {
            decoded.clear();
        }
        decoded.insert(icon_uri, texture.clone());
    });
    Some(texture)
}

impl FileCell {
    /// Shows the art again, then looks up `item`'s custom icon and, if it
    /// has none and `previews` allow one, its preview (VIEW-057). A
    /// lookup still running for the item shown before is dropped.
    pub(crate) fn look_up_picture(&self, item: &FileItem, previews: PreviewPolicy) {
        self.cancel_picture_lookup();
        let imp = self.imp();
        imp.custom_icon.set_visible(false);
        imp.image.set_opacity(1.0);
        let entry = item.entry();
        let uri = entry.uri.clone();
        let pixels = imp.icon_size.get() * self.scale_factor().max(1);
        let preview = previews
            .allows(entry)
            .then(|| PreviewRequest::for_entry(entry, pixels))
            .flatten();
        if !uri.starts_with("file:") && preview.is_none() {
            return;
        }
        let cell = self.downgrade();
        let lookup = glib::spawn_future_local(async move {
            let mut texture = custom_icon(&uri).await;
            if texture.is_none() {
                if let Some(request) = preview {
                    texture = thumbnails::preview(request).await;
                }
            }
            if let (Some(cell), Some(texture)) = (cell.upgrade(), texture) {
                let imp = cell.imp();
                imp.custom_icon.set_paintable(Some(&texture));
                imp.custom_icon.set_visible(true);
                imp.image.set_opacity(0.0);
                imp.picture_lookup.take();
            }
        });
        imp.picture_lookup.replace(Some(lookup));
    }

    /// Drops the lookup of a custom icon or preview still running, as for
    /// a row GTK unbinds when it scrolls away.
    pub(crate) fn cancel_picture_lookup(&self) {
        if let Some(lookup) = self.imp().picture_lookup.take() {
            lookup.abort();
        }
    }

    /// Whether the cell shows a custom icon or a preview, for tests.
    #[cfg(test)]
    pub(crate) fn shows_custom_icon(&self) -> bool {
        self.imp().custom_icon.is_visible()
    }
}

/// The custom icon of the local item at `uri`, if it has one.
async fn custom_icon(uri: &str) -> Option<gdk::Texture> {
    if !uri.starts_with("file:") {
        return None;
    }
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(CUSTOM_ICON, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW)
        .await
        .ok()?;
    let icon_uri = info
        .attribute_string(CUSTOM_ICON)
        .filter(|icon| !icon.is_empty())?;
    icon_texture(icon_uri.to_string()).await
}

/// The picture at `uri` at icon size, if it is a local image of a
/// sensible size.
fn decode(uri: &str) -> Option<gdk::Texture> {
    let path = gio::File::for_uri(uri).path()?;
    let size = std::fs::metadata(&path).ok()?.len();
    if size > MAX_ICON_BYTES {
        return None;
    }
    let pixbuf = Pixbuf::from_file_at_scale(&path, ICON_PIXELS, ICON_PIXELS, true).ok()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}
