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
//! Most icons are a single picture, so the bar and the badge are only made
//! the first time an image shows a network location or a badge: a list
//! cell that only ever shows files holds three widgets, not seven. Like
//! every icon, an `ArtImage` is hidden from assistive technology (see
//! [`crate::icons`]).
//!
//! [`compose`]: super::composition::compose

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::art::Art;
use super::composition::{compose, Badge, Corner, NetworkBar, Picture};

/// The CSS name of the widget, for the skin's rules about icons.
const CSS_NAME: &str = "art";

/// What an image shows, so showing the same again is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShownArt {
    art: Art,
    /// The icon's edge in logical pixels.
    size: i32,
}

/// The two boxes of the Windows-style network bar: a short stem under the
/// picture and a bar as wide as the icon.
#[derive(Debug)]
struct NetworkBarPieces {
    stem: gtk::Box,
    bar: gtk::Box,
}

impl NetworkBarPieces {
    fn new() -> Self {
        Self {
            stem: decorative_box("network-stem"),
            bar: decorative_box("network-bar"),
        }
    }

    /// Sizes the stem and the bar as `network_bar` says, the bar `width`
    /// pixels wide, and shows them.
    fn show_bar(&self, network_bar: NetworkBar, width: i32) {
        self.stem
            .set_size_request(network_bar.stem_width, network_bar.stem_height);
        self.bar.set_size_request(width, network_bar.bar_height);
        self.stem.set_visible(true);
        self.bar.set_visible(true);
    }

    fn hide(&self) {
        self.stem.set_visible(false);
        self.bar.set_visible(false);
    }
}

/// The small icon over a bottom corner, and the white disc behind a red
/// cross, which shows through the cross's cut-out lines.
#[derive(Debug)]
struct BadgePieces {
    plate: gtk::Box,
    badge: gtk::Image,
}

impl BadgePieces {
    fn new() -> Self {
        let plate = decorative_box("badge-plate");
        plate.set_valign(gtk::Align::End);
        let badge = super::decorative_image();
        badge.set_valign(gtk::Align::End);
        Self { plate, badge }
    }

    /// Shows `badge` in its corner, over its disc when it has one.
    fn show_badge(&self, badge: Badge) {
        super::set_icon(&self.badge, badge.icon, badge.size);
        self.badge.set_css_classes(&[badge.class]);
        self.badge.set_halign(corner_alignment(badge.corner));
        self.badge.set_margin_bottom(badge.raised_by);
        self.badge.set_visible(true);
        self.show_plate(badge);
    }

    /// Centres the disc of `badge`, if it has one, behind it.
    fn show_plate(&self, badge: Badge) {
        let Some(plate_size) = badge.plate else {
            self.plate.set_visible(false);
            return;
        };
        let inset = (badge.size - plate_size) / 2;
        self.plate.set_size_request(plate_size, plate_size);
        self.plate.set_halign(corner_alignment(badge.corner));
        self.plate.set_margin_start(inset);
        self.plate.set_margin_end(inset);
        self.plate.set_margin_bottom(badge.raised_by + inset);
        self.plate.set_visible(true);
    }

    fn hide(&self) {
        self.badge.set_visible(false);
        self.plate.set_visible(false);
    }
}

/// A box with the CSS class `class`, centred under the picture and hidden
/// from assistive technology like the icon it belongs to.
fn decorative_box(class: &str) -> gtk::Box {
    gtk::Box::builder()
        .halign(gtk::Align::Center)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .css_classes([class])
        .build()
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{BadgePieces, NetworkBarPieces, ShownArt};

    /// Private state of [`super::ArtImage`]: the picture in a column that
    /// holds the stem and the bar under it once they are needed, in an
    /// overlay that lays the badge and its disc over them.
    #[derive(Debug)]
    pub(crate) struct ArtImage {
        /// Lays the badge and its disc over the column.
        pub(super) overlay: gtk::Overlay,
        /// The picture above the stem and the bar.
        pub(super) column: gtk::Box,
        /// The main picture.
        pub(super) picture: gtk::Image,
        /// Made the first time a network location is shown.
        pub(super) network_bar: OnceCell<NetworkBarPieces>,
        /// Made the first time a badge is shown.
        pub(super) badge: OnceCell<BadgePieces>,
        /// What is shown now, if anything.
        pub(super) shown: Cell<Option<ShownArt>>,
    }

    impl Default for ArtImage {
        fn default() -> Self {
            Self {
                overlay: gtk::Overlay::builder()
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build(),
                column: gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build(),
                picture: crate::icons::decorative_image(),
                network_bar: OnceCell::new(),
                badge: OnceCell::new(),
                shown: Cell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArtImage {
        const NAME: &'static str = "OxArtImage";
        type Type = super::ArtImage;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name(super::CSS_NAME);
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for ArtImage {
        fn constructed(&self) {
            self.parent_constructed();
            let art = self.obj();
            art.set_halign(gtk::Align::Center);
            art.set_valign(gtk::Align::Center);
            self.column.append(&self.picture);
            // The overlay keeps its natural size, so a badge sits in the
            // art's corner however much room the widget is given.
            self.overlay.set_halign(gtk::Align::Center);
            self.overlay.set_valign(gtk::Align::Center);
            self.overlay.set_child(Some(&self.column));
            self.overlay.set_parent(&*art);
        }

        fn dispose(&self) {
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for ArtImage {}

    impl ArtImage {
        /// The stem and the bar, added under the picture the first time
        /// they are needed.
        pub(super) fn network_bar_pieces(&self) -> &NetworkBarPieces {
            self.network_bar.get_or_init(|| {
                let pieces = NetworkBarPieces::new();
                self.column.append(&pieces.stem);
                self.column.append(&pieces.bar);
                pieces
            })
        }

        /// The badge and its disc, laid over the column the first time
        /// they are needed; the disc first, so the badge covers it.
        pub(super) fn badge_pieces(&self) -> &BadgePieces {
            self.badge.get_or_init(|| {
                let pieces = BadgePieces::new();
                self.overlay.add_overlay(&pieces.plate);
                self.overlay.add_overlay(&pieces.badge);
                pieces
            })
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
    /// An image that shows nothing until [`ArtImage::set_art`] is called.
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ArtImage {
    /// An image of `art`, `size` logical pixels square.
    pub(crate) fn new(art: Art, size: i32) -> Self {
        let image = Self::default();
        image.set_art(art, size);
        image
    }

    /// Shows `art` at `size` logical pixels, replacing what was shown.
    pub(crate) fn set_art(&self, art: Art, size: i32) {
        let imp = self.imp();
        let wanted = ShownArt { art, size };
        if imp.shown.get() == Some(wanted) {
            return;
        }
        imp.shown.set(Some(wanted));
        let composition = compose(art, size);
        self.show_picture(&composition.picture);
        self.show_network_bar(composition.network_bar, size);
        self.show_badge(composition.badge);
    }

    /// What the image shows, for tests.
    #[cfg(test)]
    pub(crate) fn art(&self) -> Option<Art> {
        self.imp().shown.get().map(|shown| shown.art)
    }

    fn show_picture(&self, picture: &Picture) {
        let image = &self.imp().picture;
        super::set_icon(image, picture.icon, picture.size);
        image.set_css_classes(picture.classes);
    }

    /// Shows the stem and bar under the picture, the bar `size` wide, or
    /// hides them.
    fn show_network_bar(&self, network_bar: Option<NetworkBar>, size: i32) {
        let imp = self.imp();
        let Some(network_bar) = network_bar else {
            // Nothing to hide if no network location was ever shown.
            if let Some(pieces) = imp.network_bar.get() {
                pieces.hide();
            }
            return;
        };
        imp.network_bar_pieces().show_bar(network_bar, size);
    }

    /// Shows `badge` in its corner, or hides the badge.
    fn show_badge(&self, badge: Option<Badge>) {
        let imp = self.imp();
        let Some(badge) = badge else {
            // Nothing to hide if no badge was ever shown.
            if let Some(pieces) = imp.badge.get() {
                pieces.hide();
            }
            return;
        };
        imp.badge_pieces().show_badge(badge);
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
            let network_bar = imp.network_bar.get();
            let badge = imp.badge.get();
            Self {
                picture: imp.picture.icon_name().map(String::from),
                size: imp.picture.pixel_size(),
                has_network_bar: network_bar.is_some_and(|pieces| pieces.bar.is_visible()),
                has_badge: badge.is_some_and(|pieces| pieces.badge.is_visible()),
            }
        }
    }

    /// The badge of `image`, which has shown one.
    fn badge_pieces(image: &ArtImage) -> &BadgePieces {
        image.imp().badge.get().expect("the image has shown a badge")
    }

    /// parity: LOOK-015
    #[gtk::test]
    fn a_file_shows_the_colour_icon_of_its_type_alone() {
        let image = ArtImage::new(Art::File(FileType::Spreadsheet), 21);
        assert_eq!(Shown::of(&image), Shown::alone(Icon::TableColor20, 21));
    }

    #[gtk::test]
    fn a_single_picture_makes_no_bar_or_badge() {
        let image = ArtImage::new(Art::Folder, 21);
        assert!(image.imp().network_bar.get().is_none(), "no stem or bar");
        assert!(image.imp().badge.get().is_none(), "no badge or disc");
        image.set_art(Art::File(FileType::Text), 21);
        assert!(image.imp().network_bar.get().is_none(), "still no stem or bar");
        assert!(image.imp().badge.get().is_none(), "still no badge or disc");
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
        image.set_art(drive, 16);
        let disconnected = Shown {
            has_network_bar: true,
            has_badge: true,
            ..Shown::alone(Icon::HardDrive, 12)
        };
        assert_eq!(Shown::of(&image), disconnected);
        let cross = badge_pieces(&image);
        assert_eq!(
            cross.badge.icon_name().as_deref(),
            Some(Icon::DismissCircleFilled.name())
        );
        assert!(cross.plate.is_visible(), "the cross is white on its disc");
        assert_eq!(
            cross.badge.halign(),
            gtk::Align::Start,
            "Windows puts it bottom-left"
        );
    }

    /// parity: ARC-001
    #[gtk::test]
    fn showing_other_art_replaces_every_piece() {
        let image = ArtImage::new(Art::ZipFolder, 21);
        assert!(badge_pieces(&image).badge.is_visible(), "the zip badge");
        assert!(
            !badge_pieces(&image).plate.is_visible(),
            "the zip badge has no disc"
        );
        image.set_art(Art::TintedGlyph(Icon::Home, Tint::Home), 18);
        assert_eq!(Shown::of(&image), Shown::alone(Icon::Home, 18));
        assert!(image.imp().picture.has_css_class("tint-home"));
        image.set_art(Art::Glyph(Icon::HardDrive), 18);
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

    /// app.js marks its icons `aria-hidden`; the row, tab or card around an
    /// icon names it.
    #[gtk::test]
    fn no_piece_of_the_art_is_announced_to_screen_readers() {
        let image = ArtImage::new(
            Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected),
            16,
        );
        let imp = image.imp();
        let bar = imp.network_bar.get().expect("a network location has the bar");
        let badge = badge_pieces(&image);
        let pieces: [&gtk::Widget; 8] = [
            image.upcast_ref(),
            imp.overlay.upcast_ref(),
            imp.column.upcast_ref(),
            imp.picture.upcast_ref(),
            bar.stem.upcast_ref(),
            bar.bar.upcast_ref(),
            badge.plate.upcast_ref(),
            badge.badge.upcast_ref(),
        ];
        for piece in pieces {
            let role = piece.accessible_role();
            assert_eq!(
                role,
                gtk::AccessibleRole::Presentation,
                "{}",
                piece.type_().name()
            );
        }
    }
}
