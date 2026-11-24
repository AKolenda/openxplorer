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
    if size == IconSize::SMALL {
        // One line per item, as tall as List's, in columns Windows 11's
        // distance apart.
        return CellSize {
            width: text_size::ceil_pixels(SMALL_ICONS_COLUMN * metrics.scale),
            height: compact_row(text_size) - 1,
        };
    }
    let (pitch, _) = windows_spacing(size.pixels());
    CellSize {
        width: text_size::ceil_pixels(pitch * metrics.scale.max(1.0)),
        // The icon, two lines of name and the box's 2 pixels above and
        // below: 289 pixels from one row to the next in Extra large at
        // 100%, as Windows 11 spaces them.
        height: size.pixels()
            + text_size::ceil_pixels(2.0 * tile_name_line(metrics.scale))
            + 2 * TILE_BOX_PADDING,
    }
}

/// Pixels between an icon item's box and its icon above, and its name
/// below, as Windows 11 draws them.
pub(crate) const TILE_BOX_PADDING: i32 = 2;

/// The distance between two lines of an icon's name at `scale`: 14.5
/// pixels at 100%, so a box with a two-line name stands 289 pixels high
/// in Extra large, as Windows 11 spaces its rows.
pub(crate) fn tile_name_line(scale: f64) -> f64 {
    14.5 * scale
}

/// Windows 11's distance from one icon column to the next, and the gap
/// between two items' boxes, at 100%, for icons `pixels` wide: measured
/// on its Extra large (291 and 24), Large (109 and 8) and Medium (75 and
/// 5) captures at 1920x1080, and in between and below drawn on the line
/// through them. Windows keeps that distance whatever the window's
/// width, with the rest of the width left free at the right.
pub(crate) fn windows_spacing(pixels: i32) -> (f64, f64) {
    const MEASURED: [(f64, f64, f64); 3] = [(48.0, 75.0, 5.0), (96.0, 109.0, 8.0), (256.0, 291.0, 24.0)];
    let pixels = f64::from(pixels);
    let (low, high) = if pixels <= MEASURED[1].0 {
        (MEASURED[0], MEASURED[1])
    } else {
        (MEASURED[1], MEASURED[2])
    };
    let along = (pixels - low.0) / (high.0 - low.0);
    let pitch = low.1 + along * (high.1 - low.1);
    let gap = (low.2 + along * (high.2 - low.2)).max(4.0);
    (pitch.max(pixels + gap), gap)
}

/// The padding left and right of an icon item's content in its box:
/// what Windows 11's box holds beside the icon (267 pixels for 256 in
/// Extra large, 101 for 96 in Large, 70 for 48 in Medium), at least 2.
pub(crate) fn icon_side_padding(size: IconSize) -> i32 {
    let (pitch, gap) = windows_spacing(size.pixels());
    #[expect(clippy::cast_possible_truncation, reason = "a box of a few hundred pixels")]
    let beside = ((pitch - gap - f64::from(size.pixels())) / 2.0).floor() as i32;
    beside.max(2)
}

/// The margins left and right of an icon item's box, which
/// make Windows 11's gap between two boxes ([`windows_spacing`]).
pub(crate) fn icon_side_margins(size: IconSize) -> (i32, i32) {
    let (_, gap) = windows_spacing(size.pixels());
    #[expect(clippy::cast_possible_truncation, reason = "a gap of a few pixels")]
    let gap = gap.round() as i32;
    (gap / 2, gap - gap / 2)
}

/// Pixels from one Small icons column to the next at 100%, as Windows 11
/// spaces them (measured in the Windows 11 VM at 1920x1080, 100% scale):
/// the items fill rows left to right and the view scrolls down.
pub(crate) const SMALL_ICONS_COLUMN: f64 = 310.0;

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

/// Pixels of a group heading's row in Tiles and Content beside its title:
/// 5 pixels of padding above and below it, and the 8-pixel gap before
/// every heading (VIEW-022, VIEW-071; `skin/folder-views.css`).
pub(crate) const HEADING_CHROME: i32 = 5 + 5 + 8;

/// The height of a group heading's row in Tiles and Content at
/// `text_size`: one line of title with its padding and the gap before it,
/// 35 pixels at 100%.
pub(crate) fn group_heading_row(text_size: TextSize) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    text_size::ceil_pixels(text_line(scale)) + HEADING_CHROME
}

/// Pixels of a Tiles cell beside its content: 8 pixels of padding above
/// and below the icon and lines, and a 3-pixel margin above and below, so
/// rows of tiles are 6 pixels apart (VIEW-071).
pub(crate) const TILES_CHROME: i32 = 8 + 8 + 3 + 3;

/// Pixels of a Content row beside its two lines: 6 pixels of padding above
/// and 6 below, and the 1-pixel line under it (VIEW-071).
pub(crate) const CONTENT_CHROME: i32 = 6 + 6 + 1;

/// A Tiles cell at `text_size`: 254 pixels wide at 100%, Windows 11's
/// tile and the gap to the next, growing with the text; as tall as its three lines of text
/// (name, type and size, a pixel apart) or its icon, with its padding and
/// margins, 75 pixels at 100%.
pub(crate) fn tiles_cell(text_size: TextSize) -> CellSize {
    let scale = text_size::metrics(text_size).scale;
    CellSize {
        // 254 pixels apart at 100%, as Windows 11's tiles, and as much more
        // as the text is larger, as Windows' scaling grows them.
        width: windows_scaled(254.0, scale),
        height: text_size::ceil_pixels(3.0 * text_line(scale) + 2.0).max(TILES_ICON) + TILES_CHROME,
    }
}

/// The height of a Content row at `text_size`: two lines of text a pixel
/// apart, with its padding and the line under it, 48 pixels at 100%, as
/// Windows 11's rows are, with room for the 32-pixel icon.
pub(crate) fn content_row(text_size: TextSize) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    let fitted = text_size::ceil_pixels(2.0 * text_line(scale) + 1.0).max(CONTENT_ICON) + CONTENT_CHROME;
    // Larger text spaces the rows as far apart as Windows' scaling does.
    fitted.max(windows_scaled(48.0, scale))
}

/// `pixels` of Windows 11's layout at 100%, scaled as its display scaling
/// scales them, to the nearest pixel, at `scale` or 1 when smaller: the
/// steps below 100% keep the 100% spacing.
fn windows_scaled(pixels: f64, scale: f64) -> i32 {
    text_size::ceil_pixels(pixels * scale.max(1.0) - 0.5)
}

/// The distance from one Details row to the next in Compact view
/// (VIEW-067) at `text_size`: 21 pixels at 100%, as Windows 11's Compact
/// view spaces them, and as many times that as the text is larger.
pub(crate) fn compact_density_details_row(text_size: TextSize) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    windows_scaled(21.0, scale)
}

/// Pixels of a Tiles cell beside its content in Compact view: 2 pixels of
/// padding above and below the icon and lines, and a 5-pixel gap below,
/// so a tile is 52 pixels tall and rows are 57 pixels apart at 100%, as
/// in Windows 11's Compact view (VIEW-067, measured on its capture).
pub(crate) const COMPACT_TILES_CHROME: i32 = 2 + 2 + 5;

/// The height of a line of Tiles' and Content's text in Compact view at
/// `text_size`: 16 pixels at 100%, Windows 11's pitch of the tiles' lines,
/// growing with the text. The stylesheet sets it as the lines' height
/// (`theme/fonts.rs`), so the rows are as far apart whatever the font's
/// own line height: Noto Sans' is 16 pixels at 12, the distribution
/// checks' fallback font's 14.
pub(crate) fn compact_density_line(text_size: TextSize) -> i32 {
    text_size::ceil_pixels(16.0 * text_size::metrics(text_size).scale)
}

/// A row of Tiles in Compact view at `text_size`: three lines on a
/// 16-pixel pitch beside the 48-pixel icon, with [`COMPACT_TILES_CHROME`].
pub(crate) fn compact_density_tiles_row(text_size: TextSize) -> i32 {
    let fitted = (3 * compact_density_line(text_size)).max(TILES_ICON) + COMPACT_TILES_CHROME;
    fitted.max(windows_scaled(57.0, text_size::metrics(text_size).scale))
}

/// Pixels of a group heading's row in Tiles beside its title in Compact
/// view: no gap above, 4 pixels of padding above and below the title,
/// room inside its hover box, and a 1-pixel gap under it
/// (`theme/fonts.rs`), so a heading over
/// one row of tiles stands 83 pixels above the next at 100%, as in
/// Windows 11's Compact view.
pub(crate) const COMPACT_TILES_HEADING_CHROME: i32 = 4 + 4 + 1;

/// Pixels of a group heading's row in Content beside its title in Compact
/// view: a 2-pixel gap above, 5 pixels of padding above and below the
/// title, as in the usual view, and a 3-pixel gap under it, so its title
/// stands as far from the items' names as in Windows 11's Compact view.
pub(crate) const COMPACT_CONTENT_HEADING_CHROME: i32 = 2 + 5 + 5 + 3;

/// The height of a group heading's row in Compact view at `text_size`, in
/// Content or else in Tiles: one line of title with
/// [`COMPACT_CONTENT_HEADING_CHROME`] or [`COMPACT_TILES_HEADING_CHROME`].
pub(crate) fn compact_density_heading_row(text_size: TextSize, content: bool) -> i32 {
    let scale = text_size::metrics(text_size).scale;
    let chrome = if content {
        COMPACT_CONTENT_HEADING_CHROME
    } else {
        COMPACT_TILES_HEADING_CHROME
    };
    text_size::ceil_pixels(text_line(scale)) + chrome
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

    /// The icon sizes stand as far apart as Windows 11's, measured on its
    /// Extra large, Large and Medium captures at 100% (1920x1080): 291,
    /// 109 and 75 pixels from column to column, their boxes 24, 8 and 5
    /// pixels apart, and rows as tall as the icon, two lines of name and
    /// 2 pixels above and below (289 in Extra large). Larger text widens
    /// and heightens them.
    ///
    /// parity: VIEW-005
    #[test]
    fn icon_cells_are_spaced_as_windows_11_spaces_them() {
        let normal = TextSize::from_percent(100);
        let widths: Vec<i32> = [IconSize::EXTRA_LARGE, IconSize::LARGE, IconSize::MEDIUM]
            .iter()
            .map(|size| cell_size(*size, normal).width)
            .collect();
        assert_eq!(widths, [291, 109, 75]);
        assert_eq!(cell_size(IconSize::EXTRA_LARGE, normal).height, 289);
        let margins: Vec<(i32, i32)> = [IconSize::EXTRA_LARGE, IconSize::LARGE, IconSize::MEDIUM]
            .iter()
            .map(|size| icon_side_margins(*size))
            .collect();
        assert_eq!(margins, [(12, 12), (4, 4), (2, 3)]);
        let larger = TextSize::from_percent(150);
        assert!(cell_size(IconSize::LARGE, larger).width > 109);
        assert!(cell_size(IconSize::LARGE, larger).height > cell_size(IconSize::LARGE, normal).height);
        // Only the name grows with the text; the icon keeps its size.
        assert_eq!(
            cell_size(IconSize::LARGE, larger).height - cell_size(IconSize::LARGE, normal).height,
            text_size::ceil_pixels(2.0 * tile_name_line(1.5))
                - text_size::ceil_pixels(2.0 * tile_name_line(1.0))
        );
        // The sizes between follow the line through the measured ones.
        let (pitch, gap) = windows_spacing(64);
        assert!(pitch > 75.0 && pitch < 109.0 && (5.0..=8.0).contains(&gap));
    }

    /// Tiles are 254 pixels apart at 100%, as Windows 11's (246-pixel boxes
    /// 8 pixels apart, measured on its Tiles capture), and hold three lines of text; Content's rows are 48 pixels
    /// tall, as Windows 11's; both grow with the text.
    ///
    /// parity: VIEW-071
    #[test]
    fn tiles_and_content_rows_have_windows_sizes_and_grow_with_the_text() {
        let normal = TextSize::from_percent(100);
        let larger = TextSize::from_percent(150);
        assert_eq!(
            tiles_cell(normal),
            CellSize {
                width: 254,
                height: 75
            }
        );
        assert!(tiles_cell(larger).width > 254 && tiles_cell(larger).height > 75);
        assert_eq!(tiles_cell(normal).columns_in(1000), 3);
        assert_eq!(content_row(normal), 48);
        assert!(content_row(larger) > 48);
    }

    /// A pane width and the columns `renderRows` gives it.
    struct ColumnCase {
        pane_width: i32,
        columns: u32,
    }

    /// parity: VIEW-005
    #[test]
    fn grid_columns_follow_the_width_as_render_rows_counts_them() {
        let large_icon_cell = cell_size(IconSize::LARGE, TextSize::from_percent(100));
        assert_eq!(large_icon_cell.width, 109, "Windows 11's Large icons");
        let cases = [
            ColumnCase {
                pane_width: 0,
                columns: 1,
            },
            ColumnCase {
                pane_width: 108,
                columns: 1,
            },
            ColumnCase {
                pane_width: 218,
                columns: 2,
            },
            ColumnCase {
                pane_width: 762,
                columns: 6,
            },
            ColumnCase {
                pane_width: 763,
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
