// SPDX-License-Identifier: AGPL-3.0-only
//! [`ArtImage`]: a widget that shows an [`Art`] from real icons.
//!
//! Replaces the SVG compositions of `zipFolderIcon`, `fileIcon` and
//! `networkIcon` in `desktop/ui/app.js`. Nothing is drawn: the pictures are
//! bundled icons in `gtk::Image`s, layered with a `gtk::Overlay`, and the
//! green network bar and the disc behind the red cross are small boxes the
//! skin colours (`resources/skin/icons.css`). [`compose`] decides where
//! each piece goes.
//!
//! [`compose`]: super::composition::compose

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::art::Art;
use super::composition::{compose, Badge, Corner, NetworkBar, Picture};

/// The CSS name of the widget, for the skin's rules about icons.
const CSS_NAME: &str = "art";

mod imp {
    use std::cell::Cell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::super::art::Art;

    /// Private state of [`super::ArtImage`]: its pieces, laid out as
    /// picture, stem and bar from top to bottom, with the badge's disc and
    /// the badge over them.
    #[derive(Debug, Default)]
    pub(crate) struct ArtImage {
        /// Lays the badge and its disc over the column.
        pub(super) overlay: gtk::Overlay,
        /// The picture above the stem and the bar.
        pub(super) column: gtk::Box,
        /// The main picture.
        pub(super) picture: gtk::Image,
        /// The short vertical line of the network bar.
        pub(super) stem: gtk::Box,
        /// The horizontal line of the network bar.
        pub(super) bar: gtk::Box,
        /// The white disc behind the red cross, which shows through the
        /// cross's cut-out lines.
        pub(super) plate: gtk::Box,
        /// The small icon over a bottom corner.
        pub(super) badge: gtk::Image,
        /// What is shown, and at which size, so showing it again is free.
        pub(super) shown: Cell<Option<(Art, i32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArtImage {
        const NAME: &'static str = "OxArtImage";
        type Type = super::ArtImage;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name(super::CSS_NAME);
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for ArtImage {
        fn constructed(&self) {
            self.parent_constructed();
            let art = self.obj();
            art.set_halign(gtk::Align::Center);
            art.set_valign(gtk::Align::Center);
            self.assemble();
            self.overlay.set_parent(&*art);
        }

        fn dispose(&self) {
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for ArtImage {}

    impl ArtImage {
        /// Puts the pieces together; which of them show, and how large,
        /// is decided for each art.
        fn assemble(&self) {
            self.column.set_orientation(gtk::Orientation::Vertical);
            self.column.append(&self.picture);
            self.column.append(&self.stem);
            self.column.append(&self.bar);
            self.picture.set_halign(gtk::Align::Center);
            self.stem.set_halign(gtk::Align::Center);
            self.bar.set_halign(gtk::Align::Center);
            self.stem.add_css_class("network-stem");
            self.bar.add_css_class("network-bar");
            self.plate.add_css_class("badge-plate");
            self.plate.set_valign(gtk::Align::End);
            self.badge.set_valign(gtk::Align::End);
            // The overlay keeps its natural size, so a badge sits in the
            // art's corner however much room the widget is given.
            self.overlay.set_halign(gtk::Align::Center);
            self.overlay.set_valign(gtk::Align::Center);
            self.overlay.set_child(Some(&self.column));
            self.overlay.add_overlay(&self.plate);
            self.overlay.add_overlay(&self.badge);
        }
    }
}

glib::wrapper! {
    /// An item's or place's icon: one icon, or several layered, as
    /// [`Art`] describes it.
    pub(crate) struct ArtImage(ObjectSubclass<imp::ArtImage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ArtImage {
    /// An image that shows nothing until [`ArtImage::show`] is called.
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ArtImage {
    /// An image of `art`, `size` logical pixels square.
    pub(crate) fn new(art: Art, size: i32) -> Self {
        let image: Self = glib::Object::new();
        image.show(art, size);
        image
    }

    /// Shows `art` at `size` logical pixels, replacing what was shown.
    pub(crate) fn show(&self, art: Art, size: i32) {
        let imp = self.imp();
        if imp.shown.get() == Some((art, size)) {
            return;
        }
        imp.shown.set(Some((art, size)));
        let composition = compose(art, size);
        self.show_picture(&composition.picture);
        self.show_network_bar(composition.network_bar, size);
        self.show_badge(composition.badge);
    }

    /// What the image shows, for tests.
    #[cfg(test)]
    pub(crate) fn art(&self) -> Option<Art> {
        self.imp().shown.get().map(|(art, _)| art)
    }

    fn show_picture(&self, picture: &Picture) {
        let image = &self.imp().picture;
        super::set_icon(image, picture.icon, picture.size);
        image.set_css_classes(&picture.classes);
    }

    /// Shows the stem and bar under the picture, the bar `size` wide, or
    /// hides them.
    fn show_network_bar(&self, network_bar: Option<NetworkBar>, size: i32) {
        let imp = self.imp();
        let Some(network_bar) = network_bar else {
            imp.stem.set_visible(false);
            imp.bar.set_visible(false);
            return;
        };
        imp.stem
            .set_size_request(network_bar.stem_width, network_bar.stem_height);
        imp.bar.set_size_request(size, network_bar.bar_height);
        imp.stem.set_visible(true);
        imp.bar.set_visible(true);
    }

    /// Shows `badge` in its corner, over its disc when it has one, or
    /// hides both.
    fn show_badge(&self, badge: Option<Badge>) {
        let imp = self.imp();
        let Some(badge) = badge else {
            imp.badge.set_visible(false);
            imp.plate.set_visible(false);
            return;
        };
        super::set_icon(&imp.badge, badge.icon, badge.size);
        imp.badge.set_css_classes(&[badge.class]);
        imp.badge.set_halign(corner_alignment(badge.corner));
        imp.badge.set_margin_bottom(badge.raised_by);
        imp.badge.set_visible(true);
        self.show_plate(badge);
    }

    /// Centres the disc of `badge`, if it has one, behind it.
    fn show_plate(&self, badge: Badge) {
        let plate = &self.imp().plate;
        let Some(plate_size) = badge.plate else {
            plate.set_visible(false);
            return;
        };
        let inset = (badge.size - plate_size) / 2;
        plate.set_size_request(plate_size, plate_size);
        plate.set_halign(corner_alignment(badge.corner));
        plate.set_margin_start(inset);
        plate.set_margin_end(inset);
        plate.set_margin_bottom(badge.raised_by + inset);
        plate.set_visible(true);
    }
}

/// The horizontal alignment that puts a badge in `corner`.
const fn corner_alignment(corner: Corner) -> gtk::Align {
    match corner {
        Corner::BottomLeft => gtk::Align::Start,
        Corner::BottomRight => gtk::Align::End,
    }
}

#[cfg(test)]
mod tests {
    use ox_core::places::NetworkKind;

    use super::*;
    use crate::icons::art::Connection;
    use crate::icons::{FileType, Icon, Tint};

    /// What an [`ArtImage`] shows now: its picture, and whether the network
    /// bar and a badge show.
    #[derive(Debug, PartialEq, Eq)]
    struct Shown {
        picture: Option<String>,
        size: i32,
        has_network_bar: bool,
        has_badge: bool,
    }

    impl Shown {
        /// `picture` at `size` pixels, with nothing else.
        fn alone(picture: Icon, size: i32) -> Self {
            Self {
                picture: Some(picture.name().to_owned()),
                size,
                has_network_bar: false,
                has_badge: false,
            }
        }

        /// What `image` shows now.
        fn of(image: &ArtImage) -> Self {
            let imp = image.imp();
            Self {
                picture: imp.picture.icon_name().map(String::from),
                size: imp.picture.pixel_size(),
                has_network_bar: imp.bar.is_visible(),
                has_badge: imp.badge.is_visible(),
            }
        }
    }

    /// parity: LOOK-015
    #[gtk::test]
    fn a_file_shows_the_colour_icon_of_its_type_alone() {
        let image = ArtImage::new(Art::File(FileType::Spreadsheet), 21);
        assert_eq!(Shown::of(&image), Shown::alone(Icon::TableColor20, 21));
    }

    /// parity: LOOK-016
    #[gtk::test]
    fn a_share_stands_on_the_bar_and_a_disconnected_drive_shows_the_cross() {
        let image = ArtImage::new(Art::SHARE, 16);
        let share = Shown {
            has_network_bar: true,
            ..Shown::alone(Icon::FileFolder, 12)
        };
        assert_eq!(Shown::of(&image), share);
        let drive = Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected);
        image.show(drive, 16);
        let disconnected = Shown {
            has_network_bar: true,
            has_badge: true,
            ..Shown::alone(Icon::HardDrive, 12)
        };
        assert_eq!(Shown::of(&image), disconnected);
        assert_eq!(
            image.imp().badge.icon_name().as_deref(),
            Some(Icon::DismissCircleFilled.name())
        );
        assert!(image.imp().plate.is_visible(), "the cross is white on its disc");
        assert_eq!(
            image.imp().badge.halign(),
            gtk::Align::Start,
            "Windows puts it bottom-left"
        );
    }

    /// parity: ARC-001
    #[gtk::test]
    fn showing_other_art_replaces_every_piece() {
        let image = ArtImage::new(Art::ZipFolder, 21);
        assert!(image.imp().badge.is_visible(), "the zip badge");
        assert!(!image.imp().plate.is_visible(), "the zip badge has no disc");
        image.show(Art::TintedGlyph(Icon::Home, Tint::Home), 18);
        assert_eq!(Shown::of(&image), Shown::alone(Icon::Home, 18));
        assert!(image.imp().picture.has_css_class("tint-home"));
        image.show(Art::Glyph(Icon::HardDrive), 18);
        assert!(
            !image.imp().picture.has_css_class("tint-home"),
            "a new art drops the old tint"
        );
        assert_eq!(image.art(), Some(Art::Glyph(Icon::HardDrive)));
    }

    #[gtk::test]
    fn the_art_keeps_its_size_however_much_room_it_is_given() {
        let image = ArtImage::new(Art::SHARE, 19);
        let (_, natural_width, _, _) = image.measure(gtk::Orientation::Horizontal, -1);
        let (_, natural_height, _, _) = image.measure(gtk::Orientation::Vertical, -1);
        assert_eq!((natural_width, natural_height), (19, 19));
    }
}
