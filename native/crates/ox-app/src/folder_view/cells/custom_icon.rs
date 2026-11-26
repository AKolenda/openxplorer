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
//! for it ([`super::folder_picture`]). Last comes a picture the user chose
//! in File icons for the item's kind (LOOK-029).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use super::folder_picture::{FOLDER_PICTURE, SMALLEST_PICTURE_ICON};
use super::FileCell;
use crate::folder_view::item::FileItem;
use crate::icons::Art;
use crate::picture_guard::{self, Limits};
use crate::thumbnails::folder_preview::{
    contents_need_checking, folder_art, kept_folder_art, FolderArt, FolderRequest, KnownArt,
};
use crate::thumbnails::{self, OnScreen, PreviewPolicy, PreviewRequest};

/// The metadata key Files stores a custom icon under.
pub(crate) const CUSTOM_ICON: &str = "metadata::custom-icon";
/// Pictures larger than this are not decoded for an icon.
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;
/// The size a picture is decoded at: the largest icon the views draw.
const ICON_PIXELS: i32 = 256;
/// How many decoded pictures are kept.
const MAX_CACHED_ICONS: usize = 256;
/// How long a tile whose thumbnail the cache holds waits for it without
/// its type icon: a read of the cache takes a few milliseconds, a busy
/// disk longer.
const ICON_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
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
        // A wait for the item shown before ends with it.
        imp.icon_wait.set(imp.icon_wait.get().wrapping_add(1));
        imp.binding.set(imp.binding.get().wrapping_add(1));
        imp.custom_icon.set_visible(false);
        imp.folder_picture.show_texture(None);
        imp.folder_contents.show_art(None);
        imp.image.set_opacity(1.0);
        let entry = item.entry();
        let uri = entry.uri.clone();
        let started = self.start_preview(entry, previews);
        let kind_picture = crate::icons::file_icons::picture_for(entry);
        let contents = self
            .start_folder_contents(entry, previews)
            .filter(|_| kind_picture.is_none());
        if !uri.starts_with("file:") && !started.wanted && kind_picture.is_none() {
            return;
        }
        let is_folder = entry.is_dir;
        let on_the_folder = imp.is_tile.get() && imp.icon_size.get() >= SMALLEST_PICTURE_ICON;
        let cell = self.downgrade();
        let PreviewStart {
            shown_at_once,
            reading,
            own_found,
            ..
        } = started;
        let lookup = glib::spawn_future_local(async move {
            let own = own_pictures(&uri).await;
            let mut texture = match own.icon {
                Some(icon) => icon_texture(icon).await,
                None => None,
            };
            // Only a custom icon that could be drawn takes the preview's
            // place; one missing or damaged leaves it.
            own_found.set(texture.is_some());
            // A folder's picture sits on the folder at Medium icons and
            // larger, and takes the small icon's place in Details, List
            // and Small icons, as Windows draws it there.
            if let Some(picture) = own.folder_picture.filter(|_| texture.is_none() && is_folder) {
                if on_the_folder {
                    if let (Some(cell), Some(picture)) = (cell.upgrade(), icon_texture(picture).await) {
                        // A folder's own picture takes its contents' place.
                        cell.hide_folder_contents();
                        cell.imp().folder_picture.show_texture(Some(&picture));
                        cell.imp().picture_lookup.take();
                    }
                    return;
                }
                texture = icon_texture(picture).await;
            }
            // After the folder's own pictures, its contents, unless they
            // were drawn at once.
            if let Some(request) = contents
                .filter(|_| texture.is_none())
                .and_then(FolderContentsStart::wanted)
            {
                let art = folder_art(request).await;
                if let Some(cell) = cell.upgrade() {
                    if let Some(art) = art {
                        thumbnails::trace::event("folder-shown", &uri);
                        cell.show_folder_contents(&art);
                    } else {
                        // Art drawn at once from memory that is no
                        // longer there.
                        cell.hide_folder_contents();
                    }
                    cell.imp().picture_lookup.take();
                }
                return;
            }
            if texture.is_none() {
                if let Some(shown) = shown_at_once {
                    // Already shown, and still the item's picture.
                    if let Some(cell) = cell.upgrade() {
                        cell.imp().picture_lookup.take();
                    }
                    drop(shown);
                    return;
                }
                if let Some(reading) = reading {
                    // Shown as soon as it was read.
                    let read = reading.finish().await;
                    if let Some(cell) = cell.upgrade() {
                        if read.is_none() {
                            // No picture after all: the type icon it waited
                            // without.
                            cell.imp().image.set_opacity(1.0);
                        }
                        if read.is_some() || kind_picture.is_none() {
                            cell.imp().picture_lookup.take();
                            return;
                        }
                    }
                }
            }
            if let Some(picture) = kind_picture.filter(|_| texture.is_none()) {
                texture = icon_texture(picture).await;
            }
            let Some(cell) = cell.upgrade() else {
                return;
            };
            if let Some(texture) = texture {
                thumbnails::trace::event("shown", &uri);
                cell.show_picture(&texture);
            } else if !cell.imp().folder_contents.is_visible() {
                // No picture after all: the type icon it waited without.
                cell.imp().image.set_opacity(1.0);
            }
            cell.imp().picture_lookup.take();
        });
        imp.picture_lookup.replace(Some(lookup));
    }

    /// Starts showing `entry`'s preview, where `previews` allow one: from
    /// memory on this very frame, as Windows draws a thumbnail it has
    /// cached, or else read (or made) while the item's own pictures are
    /// looked up, which takes a while for the many tiles of one layout, and
    /// shown as soon as it is read unless a custom icon was found first.
    fn start_preview(&self, entry: &ox_core::entry::Entry, previews: PreviewPolicy) -> PreviewStart {
        let uri = entry.uri.clone();
        let pixels = self.imp().icon_size.get() * self.scale_factor().max(1);
        let preview = previews.request(entry, pixels).map(|request| {
            let cell = self.downgrade();
            PreviewRequest {
                on_screen: OnScreen::new(move || cell.upgrade().is_some_and(|cell| cell.is_on_screen())),
                ..request
            }
        });
        if preview.is_some() {
            thumbnails::trace::event("bind", &uri);
        }
        let shown_at_once = preview
            .as_ref()
            .and_then(thumbnails::kept_preview)
            .filter(is_big_enough_to_see);
        if let Some(texture) = &shown_at_once {
            thumbnails::trace::event("shown-at-bind", &uri);
            self.show_picture(texture);
        } else if preview.as_ref().is_some_and(thumbnails::probably_cached) {
            // The cache holds its thumbnail, which takes a moment to read:
            // the tile waits for it without its type icon, rather than
            // showing an icon replaced a moment later, for a moment at
            // most.
            self.hide_type_icon_for_a_moment(&uri);
        }
        let wanted = preview.is_some();
        let own_found = Rc::new(Cell::new(false));
        let reading = preview.filter(|_| shown_at_once.is_none()).map(|request| {
            let (cell, own_found) = (self.downgrade(), Rc::clone(&own_found));
            let binding = self.imp().binding.get();
            Reading(Some(glib::spawn_future_local(async move {
                let texture = thumbnails::preview(request).await.filter(is_big_enough_to_see)?;
                // Shown only on a cell that still shows this item.
                let same_item = |cell: &FileCell| cell.imp().binding.get() == binding;
                if let Some(cell) = cell.upgrade().filter(|cell| same_item(cell) && !own_found.get()) {
                    thumbnails::trace::event("shown", &uri);
                    cell.show_picture(&texture);
                }
                Some(texture)
            })))
        });
        PreviewStart {
            wanted,
            shown_at_once,
            reading,
            own_found,
        }
    }

    /// Starts showing the contents of `entry`, a folder, on its icon,
    /// where `previews` allow it and the view draws a folder large enough
    /// (Medium icons and larger, and Tiles, as Windows): from memory on
    /// this very frame when they are known, or else to be read once the
    /// folder's own pictures are looked up. `None` for an item that shows
    /// no contents.
    fn start_folder_contents(
        &self,
        entry: &ox_core::entry::Entry,
        previews: PreviewPolicy,
    ) -> Option<FolderContentsStart> {
        let imp = self.imp();
        let large_enough = (imp.is_tile.get() && imp.icon_size.get() >= SMALLEST_PICTURE_ICON)
            || (imp.shows_item_lines.get() && !imp.is_content_row.get());
        if !large_enough || Art::for_entry(entry) != Art::Folder {
            return None;
        }
        let pixels = imp.icon_size.get() * self.scale_factor().max(1);
        let cell = self.downgrade();
        let request = FolderRequest {
            on_screen: OnScreen::new(move || cell.upgrade().is_some_and(|cell| cell.is_on_screen())),
            ..previews.folder_request(entry, pixels)?
        };
        match kept_folder_art(&request) {
            // Drawn at once from memory; read again in the background once
            // its items were read a while ago, as a picture overwritten in
            // place leaves its folder's time as it was.
            Some(KnownArt::Drawn(art)) => {
                thumbnails::trace::event("folder-shown-at-bind", &entry.uri);
                self.show_folder_contents(&art);
                if contents_need_checking(&request) {
                    Some(FolderContentsStart::Wanted(request))
                } else {
                    Some(FolderContentsStart::Known)
                }
            }
            Some(KnownArt::Plain) => Some(FolderContentsStart::Known),
            None => Some(FolderContentsStart::Wanted(request)),
        }
    }

    /// Shows `art` of the folder's contents in place of its plain icon.
    fn show_folder_contents(&self, art: &FolderArt) {
        let imp = self.imp();
        imp.folder_contents.show_art(Some(art));
        imp.image.set_opacity(0.0);
    }

    /// Shows the folder's plain icon again.
    fn hide_folder_contents(&self) {
        let imp = self.imp();
        if imp.folder_contents.is_visible() {
            imp.folder_contents.show_art(None);
            imp.image.set_opacity(1.0);
        }
    }

    /// Whether any of the cell is inside the scrolled view that shows it:
    /// GTK binds many cells beyond the view's edges. A cell bound but not
    /// laid out yet, which has no size, is not on the screen yet: the cells
    /// of a layout are all bound before any is given its place.
    fn is_on_screen(&self) -> bool {
        if !self.is_mapped() {
            return false;
        }
        let Some(view) = self.ancestor(gtk::ScrolledWindow::static_type()) else {
            return true;
        };
        let Some(bounds) = self.compute_bounds(&view) else {
            return false;
        };
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return false;
        }
        #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
        let (width, height) = (view.width() as f32, view.height() as f32);
        bounds.x() < width
            && bounds.y() < height
            && bounds.x() + bounds.width() > 0.0
            && bounds.y() + bounds.height() > 0.0
    }

    /// Shows `texture` in place of the type icon, and of a folder's
    /// contents.
    fn show_picture(&self, texture: &gdk::Texture) {
        let imp = self.imp();
        imp.folder_contents.show_art(None);
        imp.custom_icon.set_paintable(Some(texture));
        imp.custom_icon.set_visible(true);
        imp.image.set_opacity(0.0);
    }

    /// Hides the type icon while a thumbnail the cache holds is read, and
    /// shows it again after [`ICON_WAIT`] if the thumbnail has not come,
    /// as the disk may be busy.
    fn hide_type_icon_for_a_moment(&self, uri: &str) {
        let imp = self.imp();
        imp.image.set_opacity(0.0);
        thumbnails::trace::event("icon-held", uri);
        let generation = imp.icon_wait.get().wrapping_add(1);
        imp.icon_wait.set(generation);
        let cell = self.downgrade();
        let uri = uri.to_owned();
        glib::timeout_add_local_once(ICON_WAIT, move || {
            let Some(cell) = cell.upgrade() else {
                return;
            };
            let imp = cell.imp();
            if imp.icon_wait.get() == generation && !imp.custom_icon.is_visible() {
                thumbnails::trace::event("icon-revealed", &uri);
                imp.image.set_opacity(1.0);
            }
        });
    }

    /// Drops the lookup of a custom icon or preview still running, as for
    /// a row GTK unbinds when it scrolls away.
    pub(crate) fn cancel_picture_lookup(&self) {
        let imp = self.imp();
        imp.binding.set(imp.binding.get().wrapping_add(1));
        if let Some(lookup) = imp.picture_lookup.take() {
            lookup.abort();
        }
    }

    /// Whether the cell shows its folder's picture on the folder, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn shows_folder_picture(&self) -> bool {
        self.imp().folder_picture.shows_picture()
    }

    /// Whether a picture of its contents peeks out of the folder the cell
    /// shows, and whether a page does; `None` while it shows none, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn shown_folder_contents(&self) -> Option<(bool, bool)> {
        self.imp().folder_contents.shown()
    }

    /// Whether the cell shows a custom icon or a preview, for tests.
    #[cfg(test)]
    pub(crate) fn shows_custom_icon(&self) -> bool {
        self.imp().custom_icon.is_visible()
    }
}

/// How a cell's preview started ([`FileCell::start_preview`]).
struct PreviewStart {
    /// The item gets a preview at all.
    wanted: bool,
    /// The preview kept in memory, shown on the frame the cell was bound.
    shown_at_once: Option<gdk::Texture>,
    /// The preview being read or made.
    reading: Option<Reading>,
    /// Set once the item's own picture (a custom icon) is found, which
    /// then takes the preview's place.
    own_found: Rc<Cell<bool>>,
}

/// How the previews of a folder's contents started for a cell.
enum FolderContentsStart {
    /// Known at once: drawn, or known to stay plain.
    Known,
    /// To be read.
    Wanted(FolderRequest),
}

impl FolderContentsStart {
    /// The request still to be read, if any.
    fn wanted(self) -> Option<FolderRequest> {
        match self {
            Self::Known => None,
            Self::Wanted(request) => Some(request),
        }
    }
}

/// A preview being read for a cell; stopped when the cell no longer
/// wants it, as when it is bound to another item.
struct Reading(Option<glib::JoinHandle<Option<gdk::Texture>>>);

impl Reading {
    /// The preview, once read.
    /// The preview, once read. Dropped while it waits, as an aborted
    /// lookup drops it, it stops the reading too: a glib task whose handle
    /// is dropped would go on, detached.
    async fn finish(mut self) -> Option<gdk::Texture> {
        let read = self.0.as_mut()?.await.ok().flatten();
        self.0 = None;
        read
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(reading) = self.0.take() {
            reading.abort();
        }
    }
}

/// Fewest pixels across the shorter side of a thumbnail the cell shows.
const THINNEST_THUMBNAIL: i32 = 4;
/// Fewest pixels across the longer side of a thumbnail the cell shows.
const SMALLEST_THUMBNAIL: i32 = 8;

/// Whether a picture's thumbnail `texture` is large enough to stand for
/// it. One smaller, such as the 1×1 white picture New > Bitmap image
/// makes, would leave its row looking iconless on a light background, so
/// the item keeps its type's icon instead.
fn is_big_enough_to_see(texture: &gdk::Texture) -> bool {
    let (width, height) = (texture.width(), texture.height());
    width.min(height) >= THINNEST_THUMBNAIL && width.max(height) >= SMALLEST_THUMBNAIL
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
    let file = gio::File::for_path(&path);
    let decoded =
        picture_guard::decode_to_fit(&file, ICON_PIXELS, ICON_PIXELS, Limits::DISPLAY, None).ok()?;
    Some(gdk::Texture::for_pixbuf(&decoded.pixbuf))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A tile's lookup dropped while it waits for the preview being read,
    /// as when GTK binds the tile to another item, stops that reading too:
    /// a reading left running would paint the old item's picture on the
    /// tile.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_lookup_dropped_while_it_waits_stops_its_reading() {
        let context = glib::MainContext::default();
        let finished = Rc::new(Cell::new(false));
        let flag = Rc::clone(&finished);
        let reading = Reading(Some(glib::spawn_future_local(async move {
            glib::timeout_future(Duration::from_millis(50)).await;
            flag.set(true);
            None
        })));
        let lookup = glib::spawn_future_local(async move { reading.finish().await });
        for _ in 0..5 {
            context.iteration(false);
        }

        // As `cancel_picture_lookup` does: aborted, and its handle dropped.
        lookup.abort();
        drop(lookup);
        context.block_on(glib::timeout_future(Duration::from_millis(200)));

        assert!(!finished.get(), "the reading stopped with its lookup");
    }
}
