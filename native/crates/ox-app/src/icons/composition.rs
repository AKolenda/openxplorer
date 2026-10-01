// SPDX-License-Identifier: AGPL-3.0-only
//! Where the pieces of an [`Art`] go at one size: the picture, the badge
//! in a corner, and the stem and bar of the green network bar.
//!
//! Ports the geometry of `networkIcon` and `zipFolderIcon` in
//! `v2.0.0:desktop/ui/app.js` to the owner's rules for the Fluent icons (icon
//! mapping, 2026-09-27): at 16 pixels a network location's picture is 12
//! pixels on a 2 x 2 stem and a 16 x 2 bar, scaled in proportion at every
//! other size, and a badge is 9/16 of the icon. Computed without widgets,
//! so the geometry is tested on its own; [`super::ArtImage`] applies it.

use ox_core::places::KnownFolder;

use super::art::{Art, Connection, NetworkArt, NetworkPlace};
use super::tint::Tint;
use super::Icon;

/// The CSS class of a mapped drive's glyph, which the skin draws grey.
const MAPPED_DRIVE_CLASS: &str = "mapped-drive";

/// The CSS class of a server's glyph, which the skin draws in the share
/// blue of the current app (`#4b96c0`).
const SERVER_CLASS: &str = "server";

/// The CSS class of the ZIP badge.
const ZIP_BADGE_CLASS: &str = "zip-badge";

/// The CSS class of the red cross of a disconnected location.
const DISCONNECTED_BADGE_CLASS: &str = "disconnected-badge";

/// Every piece of an icon of one size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Composition {
    /// The main picture.
    pub(super) picture: Picture,
    /// The small icon over a corner, if any.
    pub(super) badge: Option<Badge>,
    /// The stem and bar under a network location's picture, if any.
    pub(super) network_bar: Option<NetworkBar>,
}

/// The main picture, centred, or at the top centre above a network bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Picture {
    /// What it shows.
    pub(super) icon: Icon,
    /// Its edge in logical pixels.
    pub(super) size: i32,
    /// CSS classes that colour a glyph.
    pub(super) classes: &'static [&'static str],
}

impl Picture {
    /// `icon` at `size` pixels, in its own colours or, for a glyph, the
    /// text colour.
    const fn plain(icon: Icon, size: i32) -> Self {
        Self::coloured(icon, size, &[])
    }

    /// `icon` at `size` pixels, a glyph coloured by the skin's `classes`.
    const fn coloured(icon: Icon, size: i32, classes: &'static [&'static str]) -> Self {
        Self { icon, size, classes }
    }
}

/// A small icon over a bottom corner of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Badge {
    /// What it shows.
    pub(super) icon: Icon,
    /// Its edge in logical pixels.
    pub(super) size: i32,
    /// Which bottom corner it covers.
    pub(super) corner: Corner,
    /// Pixels between its bottom edge and the icon's: the network bar's
    /// height, so the badge sits on the picture, not on the bar.
    pub(super) raised_by: i32,
    /// The CSS class that colours it.
    pub(super) class: &'static str,
    /// The edge of the white disc behind a red cross, which makes its
    /// cross white as Windows draws it; `None` for other badges.
    pub(super) plate: Option<i32>,
}

/// A bottom corner of an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Corner {
    /// Where Windows puts the red cross of a disconnected drive.
    BottomLeft,
    /// Where the zip badge goes.
    BottomRight,
}

/// The Windows-style network bar: a short stem centred under the picture
/// and a bar as wide as the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NetworkBar {
    /// The stem's width in logical pixels.
    pub(super) stem_width: i32,
    /// The stem's height.
    pub(super) stem_height: i32,
    /// The bar's height; it is as wide as the icon.
    pub(super) bar_height: i32,
}

impl NetworkBar {
    /// The bar under an icon `size` pixels high: the bar is an eighth of
    /// the icon, the picture three quarters, and the stem, as wide as the
    /// bar is high, takes the rest.
    fn for_size(size: i32) -> Self {
        let bar_height = ((size + 4) / 8).max(1);
        let picture = picture_on_bar(size);
        let stem_height = (size - picture - bar_height).max(1);
        Self {
            stem_width: bar_height,
            stem_height,
            bar_height,
        }
    }

    /// The stem and the bar together.
    const fn height(self) -> i32 {
        self.stem_height + self.bar_height
    }
}

/// The edge of the picture above the network bar: three quarters of the
/// icon, rounded.
const fn picture_on_bar(size: i32) -> i32 {
    (size * 3 + 2) / 4
}

/// A badge's edge: 9/16 of the icon, rounded.
const fn badge_size(size: i32) -> i32 {
    (size * 9 + 8) / 16
}

/// The pieces of `art` drawn `size` pixels high.
pub(super) fn compose(art: Art, size: i32) -> Composition {
    match art {
        Art::Glyph(icon) => plain(icon, size),
        Art::TintedGlyph(icon, tint) => Composition {
            picture: Picture::coloured(icon, size, tint.css_classes()),
            ..plain(icon, size)
        },
        Art::Folder => plain(Icon::FileFolder, size),
        Art::File(file_type) => plain(file_type.icon(size), size),
        Art::ZipFolder => Composition {
            badge: Some(zip_badge(size)),
            ..plain(Icon::FileFolder, size)
        },
        Art::Network(network) => on_network_bar(network, size),
    }
}

/// A picture that fills the icon, with nothing else.
fn plain(icon: Icon, size: i32) -> Composition {
    Composition {
        picture: Picture::plain(icon, size),
        badge: None,
        network_bar: None,
    }
}

/// The zip badge in the bottom-right corner of the folder.
fn zip_badge(size: i32) -> Badge {
    Badge {
        icon: Icon::FolderZip,
        size: badge_size(size),
        corner: Corner::BottomRight,
        raised_by: 0,
        class: ZIP_BADGE_CLASS,
        plate: None,
    }
}

/// A network location's picture on the stem and bar, with the red cross
/// when a share or mapped drive is disconnected.
fn on_network_bar(network: NetworkArt, size: i32) -> Composition {
    let bar = NetworkBar::for_size(size);
    let is_disconnected = network.connection == Connection::Disconnected;
    let badge =
        (is_disconnected && network.place.can_be_disconnected()).then(|| disconnected_badge(size, bar));
    Composition {
        picture: network_picture(network.place, picture_on_bar(size)),
        badge,
        network_bar: Some(bar),
    }
}

/// The picture, `size` pixels square, that a network location shows on
/// the bar.
fn network_picture(place: NetworkPlace, size: i32) -> Picture {
    match place {
        NetworkPlace::Share => Picture::plain(Icon::FileFolder, size),
        NetworkPlace::MappedDrive => Picture::coloured(Icon::HardDrive, size, &[MAPPED_DRIVE_CLASS]),
        NetworkPlace::Server => Picture::coloured(Icon::Server, size, &[SERVER_CLASS]),
        NetworkPlace::KnownFolder(folder) => known_folder_picture(folder, size),
    }
}

/// A standard folder on a network mount: its own glyph in its colour
/// (`networkIcon(19, glyph)` in app.js), or the folder for one without a
/// glyph.
fn known_folder_picture(folder: KnownFolder, size: i32) -> Picture {
    let Some(glyph) = Icon::for_known_folder(folder) else {
        return Picture::plain(Icon::FileFolder, size);
    };
    let classes: &'static [&'static str] = match Tint::for_known_folder(folder) {
        Some(tint) => tint.css_classes(),
        None => &[],
    };
    Picture::coloured(glyph, size, classes)
}

/// The red cross at the picture's bottom-left, on a white disc half its
/// size.
fn disconnected_badge(size: i32, bar: NetworkBar) -> Badge {
    let badge = badge_size(size);
    Badge {
        icon: Icon::DismissCircleFilled,
        size: badge,
        corner: Corner::BottomLeft,
        raised_by: bar.height(),
        class: DISCONNECTED_BADGE_CLASS,
        plate: Some((badge + 1) / 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::FileType;

    fn network(place: NetworkPlace, connection: Connection) -> Art {
        Art::Network(NetworkArt { place, connection })
    }

    /// parity: LOOK-016
    #[test]
    fn at_16_pixels_the_picture_is_12_on_a_2_by_2_stem_and_a_16_by_2_bar() {
        let share = compose(Art::SHARE, 16);
        assert_eq!(share.picture.icon, Icon::FileFolder);
        assert_eq!(share.picture.size, 12);
        let expected = NetworkBar {
            stem_width: 2,
            stem_height: 2,
            bar_height: 2,
        };
        assert_eq!(share.network_bar, Some(expected));
        assert_eq!(share.badge, None, "a connected share has no cross");
    }

    /// A network icon's size and its picture's, stem's and bar's.
    struct BarCase {
        size: i32,
        picture: i32,
        stem: i32,
        bar: i32,
    }

    /// parity: LOOK-016
    #[test]
    fn the_network_bar_scales_in_proportion() {
        let cases = [
            BarCase {
                size: 32,
                picture: 24,
                stem: 4,
                bar: 4,
            },
            BarCase {
                size: 48,
                picture: 36,
                stem: 6,
                bar: 6,
            },
            BarCase {
                size: 96,
                picture: 72,
                stem: 12,
                bar: 12,
            },
            BarCase {
                size: 21,
                picture: 16,
                stem: 2,
                bar: 3,
            },
        ];
        for case in cases {
            let composition = compose(Art::SHARE, case.size);
            let bar = composition.network_bar.expect("a share has the bar");
            assert_eq!(composition.picture.size, case.picture, "{} pixels", case.size);
            assert_eq!(
                (bar.stem_width, bar.bar_height),
                (case.bar, case.bar),
                "{} pixels",
                case.size
            );
            assert_eq!(bar.stem_height, case.stem, "{} pixels", case.size);
            assert_eq!(case.picture + bar.height(), case.size, "the pieces fill the icon");
        }
    }

    /// parity: LOOK-016
    #[test]
    fn a_disconnected_mapped_drive_shows_the_red_cross_on_a_grey_drive() {
        let drive = compose(network(NetworkPlace::MappedDrive, Connection::Disconnected), 16);
        assert_eq!(drive.picture.icon, Icon::HardDrive);
        assert_eq!(drive.picture.classes, [MAPPED_DRIVE_CLASS]);
        let cross = drive.badge.expect("a disconnected drive shows the cross");
        assert_eq!(cross.icon, Icon::DismissCircleFilled);
        assert_eq!(cross.corner, Corner::BottomLeft);
        assert_eq!((cross.size, cross.raised_by, cross.plate), (9, 4, Some(5)));
    }

    #[test]
    fn a_server_is_never_marked_disconnected() {
        let server = compose(network(NetworkPlace::Server, Connection::Disconnected), 19);
        assert_eq!(server.picture.icon, Icon::Server);
        assert_eq!(server.picture.classes, [SERVER_CLASS]);
        assert_eq!(server.badge, None);
    }

    /// parity: LOOK-016
    #[test]
    fn a_standard_folder_on_a_network_mount_keeps_its_tinted_glyph() {
        let place = NetworkPlace::KnownFolder(KnownFolder::Music);
        let music = compose(network(place, Connection::Connected), 19);
        assert_eq!(music.picture.icon, Icon::MusicNote);
        assert_eq!(music.picture.classes, ["tinted", "tint-music"]);
    }

    /// parity: ARC-001
    #[test]
    fn a_zip_archive_is_the_folder_with_the_zip_badge_in_its_corner() {
        let zip = compose(Art::ZipFolder, 21);
        assert_eq!(zip.picture.icon, Icon::FileFolder);
        assert_eq!(zip.picture.size, 21);
        let badge = zip.badge.expect("the zip badge");
        assert_eq!((badge.icon, badge.corner), (Icon::FolderZip, Corner::BottomRight));
        assert_eq!((badge.size, badge.raised_by, badge.plate), (12, 0, None));
        assert_eq!(zip.network_bar, None);
    }

    /// parity: LOOK-015
    #[test]
    fn a_glyph_fills_its_icon_in_the_text_colour_or_its_places_tint() {
        let drive = compose(Art::Glyph(Icon::HardDrive), 18);
        assert_eq!((drive.picture.icon, drive.picture.size), (Icon::HardDrive, 18));
        assert!(drive.picture.classes.is_empty(), "the text colour");
        let home = compose(Art::TintedGlyph(Icon::Home, Tint::Home), 18);
        assert_eq!(home.picture.classes, ["tinted", "tint-home"]);
        assert_eq!((home.badge, home.network_bar), (None, None));
    }

    #[test]
    fn a_file_fills_its_icon_with_the_design_for_its_size() {
        let notes = compose(Art::File(FileType::Text), 56);
        assert_eq!(notes.picture.icon, Icon::DocumentTextColor48);
        assert_eq!(notes.picture.size, 56);
        assert_eq!((notes.badge, notes.network_bar), (None, None));
    }
}
