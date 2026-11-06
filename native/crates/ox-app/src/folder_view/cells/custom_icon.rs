// SPDX-License-Identifier: AGPL-3.0-only
//! Custom icons (PROP-016): an item whose `GVfs` metadata names a picture
//! (`metadata::custom-icon`, which Files reads and writes too) shows that
//! picture in place of its art in the details and icon views. The lookup
//! runs when a cell is bound, at low priority, like a thumbnail lookup,
//! and a picture that arrives after the cell shows another item is
//! dropped. Pictures are decoded once at icon size and kept, so scrolling
//! back to an item does not decode its picture again. An item without a
//! custom icon shows its preview in the same place, when it has one
//! ([`crate::thumbnails`]). A folder without one shows the picture chosen
//! for it ([`super::folder_picture`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use super::folder_picture::{FOLDER_PICTURE, SMALLEST_PICTURE_ICON};
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
/// How many custom pictures are decoded at once, including workers whose
/// rows scrolled away before their blocking decoder could finish.
const DECODING_AT_ONCE: usize = 4;

thread_local! {
    /// Decoded pictures by their URI.
    static DECODED: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
    static DECODING: Rc<thumbnails::Slots> = Rc::new(thumbnails::Slots::new(DECODING_AT_ONCE));
}

/// The decoded picture at `icon_uri`, decoding it off the main thread the
/// first time.
pub(crate) async fn icon_texture(icon_uri: String) -> Option<gdk::Texture> {
    if let Some(texture) = DECODED.with_borrow(|decoded| decoded.get(&icon_uri).cloned()) {
        return Some(texture);
    }
    let uri = icon_uri.clone();
    let texture = DECODING
        .with(Rc::clone)
        .blocking(move |_| decode(&uri))
        .await
        .flatten()?;
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
    /// has none, a folder's picture (PROP-037): on the folder on a tile of
    /// Medium icons or larger, in the icon's place on smaller ones and in
    /// rows; or, where `previews` allow one, its preview (VIEW-057).
    /// A lookup still running for the item shown before is dropped.
    pub(crate) fn look_up_picture(&self, item: &FileItem, previews: PreviewPolicy) {
        self.cancel_picture_lookup();
        let imp = self.imp();
        imp.custom_icon.set_visible(false);
        imp.folder_picture.show_texture(None);
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
        let is_folder = entry.is_dir;
        let on_the_folder = imp.is_tile.get() && imp.icon_size.get() >= SMALLEST_PICTURE_ICON;
        let cell = self.downgrade();
        let lookup = glib::spawn_future_local(async move {
            let own = own_pictures(&uri).await;
            let mut texture = match own.icon {
                Some(icon) => icon_texture(icon).await,
                None => None,
            };
            // A folder's picture sits on the folder at Medium icons and
            // larger, and takes the small icon's place in Details, List
            // and Small icons, as Windows draws it there.
            if let Some(picture) = own.folder_picture.filter(|_| texture.is_none() && is_folder) {
                if on_the_folder {
                    if let (Some(cell), Some(picture)) = (cell.upgrade(), icon_texture(picture).await) {
                        cell.imp().folder_picture.show_texture(Some(&picture));
                        cell.imp().picture_lookup.take();
                    }
                    return;
                }
                texture = icon_texture(picture).await;
            }
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

    /// Whether the cell shows its folder's picture on the folder, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn shows_folder_picture(&self) -> bool {
        self.imp().folder_picture.shows_picture()
    }

    /// Whether the cell shows a custom icon or a preview, for tests.
    #[cfg(test)]
    pub(crate) fn shows_custom_icon(&self) -> bool {
        self.imp().custom_icon.is_visible()
    }
}

/// The pictures a local item names for itself: its custom icon and, for
/// a folder, the picture on its icon.
#[derive(Debug, Default)]
struct OwnPictures {
    icon: Option<String>,
    folder_picture: Option<String>,
}

/// The custom icon and folder picture of the local item at `uri`, read
/// in one query.
async fn own_pictures(uri: &str) -> OwnPictures {
    if !uri.starts_with("file:") {
        return OwnPictures::default();
    }
    let file = gio::File::for_uri(uri);
    let Ok(info) = file
        .query_info_future(
            &format!("{CUSTOM_ICON},{FOLDER_PICTURE}"),
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::LOW,
        )
        .await
    else {
        return OwnPictures::default();
    };
    let named = |key: &str| {
        info.attribute_string(key)
            .filter(|picture| !picture.is_empty())
            .map(|picture| picture.to_string())
    };
    OwnPictures {
        icon: named(CUSTOM_ICON),
        folder_picture: named(FOLDER_PICTURE),
    }
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
