// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view's zoom levels and the cells they need.
//!
//! Dolphin zooms icons through fixed sizes from 16 to 256 pixels
//! (`ZoomLevelInfo`); Windows Explorer names four of them in its Layout menu
//! (Small, Medium, Large and Extra large icons, Ctrl+Shift+4 to 1), at
//! Windows 11's sizes: 16, 48, 96 and 256 pixels (owner decision
//! 2026-11-06; the web app's Large icons were 56). Every level is an
//! [`IconSize`]; the named ones keep the keys and CSS classes the icon view
//! had before it zoomed (VIEW-005, VIEW-010).

use crate::i18n::message_id;
use crate::text_size::{self, TextSize};

/// The icon edge the web's 135 × 130 grid cell was made for: its Large
/// icons, 56 pixels. Cells for other icons grow or shrink from it.
const WEB_TILE_ICON: i32 = 56;

/// A web tile's width beyond its icon: the 135-pixel `gridWidth` less
/// [`WEB_TILE_ICON`]. Larger icons widen the tile by as much as the icon
/// grows.
const TILE_WIDTH_BEYOND_ICON: i32 = 135 - WEB_TILE_ICON;

/// One zoom level: its icon edge, its action key and CSS class, and the
/// Layout menu's name for it.
#[derive(Debug)]
struct Level {
    pixels: i32,
    key: &'static str,
    css_class: &'static str,
    label: Option<&'static str>,
}

/// Every level, smallest first.
const LEVELS: [Level; 14] = [
    named_level(16, "small", "icons-small", message_id("Small icons")),
    plain_level(22, "icons-22"),
    plain_level(28, "icons-28"),
    plain_level(32, "icons-32"),
    plain_level(40, "icons-40"),
    named_level(48, "medium", "icons-medium", message_id("Medium icons")),
    plain_level(56, "icons-56"),
    plain_level(64, "icons-64"),
    plain_level(80, "icons-80"),
    named_level(96, "large", "icons-large", message_id("Large icons")),
    plain_level(128, "icons-128"),
    plain_level(160, "icons-160"),
    plain_level(192, "icons-192"),
    named_level(
        256,
        "extra-large",
        "icons-extra-large",
        message_id("Extra large icons"),
    ),
];

/// A level the Layout menu does not name, keyed by its class.
const fn plain_level(pixels: i32, key: &'static str) -> Level {
    Level {
        pixels,
        key,
        css_class: key,
        label: None,
    }
}

/// One of Explorer's four named icon layouts.
const fn named_level(pixels: i32, key: &'static str, css_class: &'static str, label: &'static str) -> Level {
    Level {
        pixels,
        key,
        css_class,
        label: Some(label),
    }
}

/// An icon-view zoom level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct IconSize(usize);

impl IconSize {
    /// Explorer's "Small icons" (Ctrl+Shift+4), 16 pixels.
    pub(crate) const SMALL: IconSize = IconSize(0);
    /// "Medium icons" (Ctrl+Shift+3), 48 pixels.
    pub(crate) const MEDIUM: IconSize = IconSize(5);
    /// "Large icons" (Ctrl+Shift+2), 96 pixels, the Python app's only
    /// icon view, whose icons were 56.
    pub(crate) const LARGE: IconSize = IconSize(9);
    /// "Extra large icons" (Ctrl+Shift+1), 256 pixels.
    pub(crate) const EXTRA_LARGE: IconSize = IconSize(13);

    /// Explorer's named sizes, largest first, as its Layout menu lists them.
    pub(crate) const NAMED: [IconSize; 4] = [
        IconSize::EXTRA_LARGE,
        IconSize::LARGE,
        IconSize::MEDIUM,
        IconSize::SMALL,
    ];

    /// Every zoom level, smallest first.
    pub(crate) fn levels() -> impl DoubleEndedIterator<Item = IconSize> {
        (0..LEVELS.len()).map(IconSize)
    }

    /// The largest level, for tests.
    #[cfg(test)]
    pub(crate) const LARGEST: IconSize = IconSize(LEVELS.len() - 1);

    const fn level(self) -> &'static Level {
        &LEVELS[self.0]
    }

    /// Icon edge in logical pixels.
    pub(crate) const fn pixels(self) -> i32 {
        self.level().pixels
    }

    /// The Layout menu's name, for Explorer's four named sizes.
    pub(crate) fn label(self) -> Option<&'static str> {
        self.level().label.map(ox_core::i18n::gettext_static)
    }

    /// Action-state key: `large` for a named size, `icons-64` otherwise.
    pub(crate) const fn as_str(self) -> &'static str {
        self.level().key
    }

    /// The size for an action-state key.
    pub(crate) fn from_key(key: &str) -> Option<IconSize> {
        Self::levels().find(|size| size.as_str() == key)
    }

    /// The level closest to `pixels`, for a size saved in settings.
    pub(crate) fn nearest(pixels: u32) -> IconSize {
        let pixels = i32::try_from(pixels).unwrap_or(i32::MAX);
        Self::levels()
            .min_by_key(|size| (size.pixels() - pixels).abs())
            .unwrap_or(IconSize::LARGE)
    }

    /// The level at `index` from the smallest, clamped to the levels, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn at_index(index: usize) -> IconSize {
        IconSize(index.min(LEVELS.len() - 1))
    }

    /// CSS class that sizes the tiles for these icons.
    pub(crate) const fn css_class(self) -> &'static str {
        self.level().css_class
    }

    /// Every CSS class [`Self::css_class`] gives.
    pub(crate) fn css_classes() -> impl Iterator<Item = &'static str> {
        LEVELS.iter().map(|level| level.css_class)
    }
}

/// An icon-view cell in pixels: a tile plus the gap to its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellSize {
    /// The narrowest a cell may be.
    pub width: i32,
    /// The row pitch.
    pub height: i32,
}

impl CellSize {
    /// How many columns of these cells a pane `pane_width` pixels wide
    /// shows: `max(1, floor(clientWidth / gridWidth))`, as `renderRows` in
    /// app.js counts them. The tiles then share the pane's width less its
    /// 20 pixels of inset, so they can be a little narrower than a cell.
    ///
    /// GTK keeps tiles for about thirty rows of `max-columns` alive, so the
    /// window sets exactly this many columns rather than a generous cap,
    /// which made every listing build thousands of tiles.
    pub(crate) fn columns_in(self, pane_width: i32) -> u32 {
        let columns = pane_width / self.width.max(1);
        u32::try_from(columns.max(1)).unwrap_or(1)
    }
}

/// The cell of tiles of `size` at `text_size`: `gridWidth` ×
/// `gridRow` from `metrics()` in text-size.js for the web's 56-pixel
/// icons (135 × 130 at 100%), widened and heightened with the icon for
/// the other sizes, which the Python app does not have.
pub(crate) fn cell_size(size: IconSize, text_size: TextSize) -> CellSize {
    let metrics = text_size::metrics(text_size);
    let icon_growth = size.pixels() - WEB_TILE_ICON;
    let width_for_icon = size.pixels() + TILE_WIDTH_BEYOND_ICON;
    CellSize {
        width: metrics.grid_width.max(width_for_icon),
        height: metrics.grid_row + icon_growth,
    }
}

/// The icon edge of Tiles: Windows 11's tiles show 48-pixel icons and
/// thumbnails (VIEW-071).
pub(crate) const TILES_ICON: i32 = 48;

/// The icon edge of Content's rows (VIEW-071).
pub(crate) const CONTENT_ICON: i32 = 32;

/// The height of a line of the tiles' and Content's text at `scale`: the
/// icon view's 0.95 line height on 13-pixel text.
fn text_line(scale: f64) -> f64 {
    17.0 * scale
}

/// A Tiles cell at `text_size`: 256 pixels wide at 100%, about the 250 of
/// Windows 11's tiles and the gap to the next, growing with the text; as
/// tall as its three lines of text (name, type and size) or its icon,
/// with 8 pixels of padding above and below and the 2-pixel gap.
pub(crate) fn tiles_cell(text_size: TextSize) -> CellSize {
    let scale = text_size::metrics(text_size).scale;
    CellSize {
        width: text_size::ceil_pixels(200.0 * scale + 56.0).max(256),
        height: text_size::ceil_pixels(3.0 * text_line(scale) + 18.0).max(TILES_ICON + 18),
    }
}

/// The height of a Content row at `text_size`: two lines of text with 12
/// pixels above and below, 58 pixels at 100%, as Windows 11's rows are
/// about 56 to 60, and the 1-pixel line under it.
pub(crate) fn content_row(text_size: TextSize) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    text_size::ceil_pixels(2.0 * text_line(scale) + 24.0).max(58)
}

/// The height of a compact-view item at `text_size`, as Explorer's List
/// layout packs its names closer than the details rows: 26 pixels at
/// 100%, growing with the text (the web's 38-pixel details row less its
/// 12 pixels of roomy padding).
pub(crate) fn compact_row(text_size: TextSize) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    text_size::ceil_pixels(24.0 * scale + 2.0).max(26)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-005
    #[test]
    fn icon_sizes_round_trip_and_shrink() {
        for size in IconSize::levels() {
            assert_eq!(IconSize::from_key(size.as_str()), Some(size));
        }
        let pixels: Vec<i32> = IconSize::NAMED.iter().map(|size| size.pixels()).collect();
        assert!(pixels.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(pixels, [256, 96, 48, 16], "Windows 11's sizes");
    }

    /// The levels run from 16 to 256 pixels, growing; a saved size snaps to
    /// the nearest level.
    #[test]
    fn zoom_levels_run_from_16_to_256_pixels() {
        let pixels: Vec<i32> = IconSize::levels().map(IconSize::pixels).collect();
        assert_eq!((pixels[0], pixels[pixels.len() - 1]), (16, 256));
        assert!(pixels.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(IconSize::nearest(100), IconSize::LARGE);
        assert_eq!(
            IconSize::nearest(56).pixels(),
            56,
            "the web's Large icons stay as saved"
        );
        assert_eq!(IconSize::at_index(99), IconSize::LARGEST);
    }

    /// 56-pixel tiles keep the web's grid cells; Large icons' 96-pixel
    /// ones are 40 pixels wider and taller.
    ///
    /// parity: VIEW-005
    #[test]
    fn large_icon_cells_are_the_web_grid_cells() {
        let web_icons = IconSize::nearest(56);
        let cell = cell_size(web_icons, TextSize::from_percent(100));
        assert_eq!(
            cell,
            CellSize {
                width: 135,
                height: 130
            }
        );
        let larger_text = cell_size(web_icons, TextSize::from_percent(150));
        assert_eq!(
            larger_text,
            CellSize {
                width: 180,
                height: 153
            }
        );
        assert_eq!(
            cell_size(IconSize::LARGE, TextSize::from_percent(100)),
            CellSize {
                width: 175,
                height: 170
            }
        );
        assert!(cell_size(IconSize::EXTRA_LARGE, TextSize::from_percent(100)).width > cell.width);
    }

    /// Tiles are 256 pixels wide at 100%, about Windows 11's 250 and the
    /// gap, and hold three lines of text; Content's rows are 58 pixels
    /// tall; both grow with the text.
    ///
    /// parity: VIEW-071
    #[test]
    fn tiles_and_content_rows_have_windows_sizes_and_grow_with_the_text() {
        let normal = TextSize::from_percent(100);
        let larger = TextSize::from_percent(150);
        assert_eq!(
            tiles_cell(normal),
            CellSize {
                width: 256,
                height: 69
            }
        );
        assert!(tiles_cell(larger).width > 256 && tiles_cell(larger).height > 69);
        assert_eq!(tiles_cell(normal).columns_in(1000), 3);
        assert_eq!(content_row(normal), 58);
        assert!(content_row(larger) > 58);
    }

    /// A pane width and the columns `renderRows` gives it.
    struct ColumnCase {
        pane_width: i32,
        columns: u32,
    }

    /// parity: VIEW-005
    #[test]
    fn grid_columns_follow_the_width_as_render_rows_counts_them() {
        let large_icon_cell = cell_size(IconSize::nearest(56), TextSize::from_percent(100));
        assert_eq!(large_icon_cell.width, 135, "the web's gridWidth");
        let cases = [
            ColumnCase {
                pane_width: 0,
                columns: 1,
            },
            ColumnCase {
                pane_width: 134,
                columns: 1,
            },
            ColumnCase {
                pane_width: 270,
                columns: 2,
            },
            ColumnCase {
                pane_width: 944,
                columns: 6,
            },
            ColumnCase {
                pane_width: 962,
                columns: 7,
            },
        ];
        for case in cases {
            assert_eq!(
                large_icon_cell.columns_in(case.pane_width),
                case.columns,
                "{} pixels",
                case.pane_width
            );
        }
    }
}
