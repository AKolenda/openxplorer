// SPDX-License-Identifier: AGPL-3.0-only
//! Folder pictures (PROP-037): the picture the Customize tab's "Choose
//! File..." puts on a folder's icon, as Windows shows it in its larger
//! icon views. It is kept as `GVfs` metadata ([`FOLDER_PICTURE`]) in the
//! user's metadata store, never as a `desktop.ini` or `folder.jpg` in the
//! folder itself. The picture sits on the folder's front, filling a card
//! with a light edge, at Medium icons and larger; in Details, List and
//! Small icons the picture itself takes the small icon's place, as
//! Windows 11 draws it there. A custom icon (PROP-016) replaces the whole
//! folder, picture and all, as Windows' Change Icon... does.
//!
//! [`folder_picture_of`] is the hook for the details pane, whose preview
//! shows a folder's picture large, as Windows' Details pane does.

use std::cell::RefCell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene};

/// The metadata key the folder's picture is kept under: the app's own,
/// as Files has no such key.
pub(crate) const FOLDER_PICTURE: &str = "metadata::openxplorer-folder-picture";

/// The picture chosen for the local folder at `uri` on its Customize
/// tab, as a location, or `None` when it has none or is not local. Read
/// off the main thread at low priority; the caller decodes the picture.
///
/// The details pane calls this for a selected folder, so its preview
/// shows the picture large and centered in place of the folder art, as
/// Windows' Details pane does (PROP-037).
pub(crate) async fn folder_picture_of(uri: &str) -> Option<String> {
    if !uri.starts_with("file:") {
        return None;
    }
    let info = gio::File::for_uri(uri)
        .query_info_future(FOLDER_PICTURE, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW)
        .await
        .ok()?;
    info.attribute_string(FOLDER_PICTURE)
        .filter(|picture| !picture.is_empty())
        .map(|picture| picture.to_string())
}

/// The smallest icon that shows a folder's picture on the folder: Medium
/// icons.
pub(crate) const SMALLEST_PICTURE_ICON: i32 = 40;

/// The card on a folder icon of 32 units, centered across it: its top,
/// width and height. It covers the top of the front panel (y 11 to 30 of
/// the bundled folder art) and rises a little above it, as a picture
/// tucked into the folder.
const CARD: (f32, f32, f32) = (9.0, 20.0, 16.0);

/// The card's top, width and height on an icon `icon_size` pixels square.
#[expect(
    clippy::cast_possible_truncation,
    reason = "icon sizes are at most 256 pixels"
)]
pub(crate) fn card_bounds(icon_size: i32) -> (i32, i32, i32) {
    #[expect(clippy::cast_precision_loss, reason = "icon sizes are small")]
    let unit = icon_size as f32 / 32.0;
    let (top, width, height) = CARD;
    let pixels = |measure: f32| (measure * unit).round() as i32;
    (pixels(top), pixels(width), pixels(height))
}

mod imp {
    use super::{gdk, glib, graphene, RefCell};
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::FolderPicture`].
    #[derive(Debug, Default)]
    pub(crate) struct FolderPicture {
        /// The picture, drawn to fill the card.
        pub(super) texture: RefCell<Option<gdk::Texture>>,
        /// The card's width and height.
        pub(super) size: std::cell::Cell<(i32, i32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderPicture {
        const NAME: &'static str = "OxFolderPicture";
        type Type = super::FolderPicture;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("folderpicture");
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for FolderPicture {}

    impl WidgetImpl for FolderPicture {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.size.get();
            let edge = if orientation == gtk::Orientation::Horizontal {
                width
            } else {
                height
            };
            (edge, edge, -1, -1)
        }

        /// The picture scaled to cover the card, centered and cut to it,
        /// as Windows crops a folder picture.
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(texture) = self.texture.borrow().clone() else {
                return;
            };
            let widget = self.obj();
            #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            #[expect(clippy::cast_precision_loss, reason = "picture sizes are small")]
            let (picture_width, picture_height) = (texture.width() as f32, texture.height() as f32);
            if width <= 0.0 || height <= 0.0 || picture_width <= 0.0 || picture_height <= 0.0 {
                return;
            }
            let scale = (width / picture_width).max(height / picture_height);
            let (drawn_width, drawn_height) = (picture_width * scale, picture_height * scale);
            let card = graphene::Rect::new(0.0, 0.0, width, height);
            snapshot.push_clip(&card);
            snapshot.append_scaled_texture(
                &texture,
                gtk::gsk::ScalingFilter::Trilinear,
                &graphene::Rect::new(
                    (width - drawn_width) / 2.0,
                    (height - drawn_height) / 2.0,
                    drawn_width,
                    drawn_height,
                ),
            );
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A folder's picture, on the folder's icon.
    pub(crate) struct FolderPicture(ObjectSubclass<imp::FolderPicture>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FolderPicture {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl FolderPicture {
    /// Places the card on a folder icon `icon_size` pixels square, drawn
    /// centered at the top of the overlay that holds both.
    pub(crate) fn place_on_icon(&self, icon_size: i32) {
        let (top, width, height) = card_bounds(icon_size);
        self.imp().size.set((width, height));
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Start);
        self.set_margin_top(top);
        self.add_css_class("folder-picture");
        self.queue_resize();
    }

    /// Shows `texture` on the card, or hides the card for `None`.
    pub(crate) fn show_texture(&self, texture: Option<&gdk::Texture>) {
        self.imp().texture.replace(texture.cloned());
        self.set_visible(texture.is_some());
        self.queue_draw();
    }

    /// Whether the card shows a picture, for tests.
    #[cfg(test)]
    pub(crate) fn shows_picture(&self) -> bool {
        self.is_visible() && self.imp().texture.borrow().is_some()
    }
}
