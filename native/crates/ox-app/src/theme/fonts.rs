// SPDX-License-Identifier: AGPL-3.0-only
//! Font sizes and row heights that follow the text size.
//!
//! In the web interface every font size is `calc(Npx * var(--text-scale))`
//! and row metrics come from `metrics()` in text-size.js. GTK 4.14 CSS has
//! no variables, so this module generates those rules for the chosen size
//! from one table of base sizes (the values in `desktop/ui/style.css`).

use crate::folder_view::grid::{self, IconSize};
use crate::text_size::{self, TextSize};

/// The stylesheet for `text_size`: every font size, the bars, rows, menus
/// and tiles whose height follows the text, one rule per line.
pub(crate) fn css_for_text_size(text_size: TextSize) -> String {
    let metrics = text_size::metrics(text_size);
    let scale = metrics.scale;
    let mut rules: Vec<String> = Vec::new();
    rules.extend(FONT_SIZES.iter().map(|font| font.rule(scale)));
    rules.extend(SCALED_HEIGHTS.iter().map(|height| height.rule(scale)));
    rules.push(solid_frame_rule(scale));
    rules.push(details_row_rule(metrics.detail_row));
    rules.push(menu_rules(scale));
    rules.extend(IconSize::ALL.map(|icon_size| tile_rule(icon_size, text_size)));
    rules.join("\n") + "\n"
}

/// A selector's font size, in pixels at 100%.
#[derive(Debug)]
struct FontSize {
    selector: &'static str,
    pixels: f64,
}

impl FontSize {
    /// The rule for this font at `scale`.
    fn rule(&self, scale: f64) -> String {
        let size = self.pixels * scale;
        format!("{} {{ font-size: {size:.2}px; }}", self.selector)
    }
}

/// `selector`'s font is `pixels` high at 100%.
const fn font(selector: &'static str, pixels: f64) -> FontSize {
    FontSize { selector, pixels }
}

/// Every font size in the skin, as style.css sets them at 100%.
const FONT_SIZES: &[FontSize] = &[
    font("window.ox", 13.0),
    font(".ox-titlebar .tab label", 12.0),
    font(".address", 13.0),
    font(".address button.crumb", 12.0),
    font(".address entry", 13.0),
    font(".search-wrap entry", 12.0),
    font(".commandbar .text-command", 12.0),
    font(".sidebar list > row", 12.0),
    font(".sidebar-bottom button", 12.0),
    font("columnview.files", 12.0),
    font("columnview.files > header > button", 12.0),
    font("gridview.files", 12.0),
    font(".details .detail-header", 13.0),
    font(".details .detail-name", 16.0),
    font(".details .detail-type", 12.0),
    font(".details button.detail-button", 12.0),
    font(".details .detail-section", 12.0),
    font(".details .detail-key", 11.0),
    font(".details .detail-value", 11.0),
    font(".details .detail-note", 11.0),
    font(".statusbar", 11.0),
    font(".statusbar .status-mode", 10.0),
    font(".toast", 12.0),
    font(".landing .page-title", 24.0),
    font(".landing .page-subtitle", 12.0),
    font(".landing .section-title", 13.0),
    font(".landing .section-title button", 11.0),
    font(".landing .card-name", 12.0),
    font(".landing .drive-card .card-name", 13.0),
    font(".landing .card-sub", 11.0),
    font(".landing .connected", 10.0),
    font(".landing .quiet", 12.0),
    font(".landing .notice", 12.0),
    font(".landing .banner-hint", 12.0),
    font(".landing .primary", 12.0),
    font(".landing .network-manual button", 11.0),
    font(".landing .network-manual entry", 12.0),
    font(".landing .network-count", 11.0),
    font(".landing .discovery-note", 11.0),
    font(".empty-state .empty-title", 16.0),
    font(".empty-state", 12.0),
    font("popover.ox-menu list > row", 12.0),
    font("popover.ox-menu .shortcut", 10.0),
    font("popover.menu.ox-menu modelbutton", 12.0),
    font("popover.menu.ox-menu accelerator", 10.0),
    font("tooltip", 12.0),
    // The Settings page, at the settings mockup's sizes.
    font(".settings-heading", 28.0),
    font(".settings-subtitle", 13.0),
    font("entry.settings-search", 13.0),
    font(".settings-match-count", 12.0),
    font("list.settings-categories > row", 13.0),
    font(".settings-categories .category-count", 11.0),
    font(".settings .page-title", 24.0),
    font(".settings .page-lead", 13.5),
    font(".settings .group-title", 13.0),
    font(".settings .setting-title", 13.5),
    font(".settings .setting-description", 12.5),
    font(".settings .setting-notice", 12.0),
    font(".settings .setting-value", 12.5),
    font(".settings .status-title", 15.0),
    font(".settings .status-text", 12.5),
    font(".settings .note-text", 12.5),
    font(".settings .settings-paragraph", 13.0),
    font(".settings .settings-no-matches", 13.0),
    font("popover.choice-list list > row", 13.0),
];

/// A bar height that grows with the text, as the
/// `min-height: max(floor, calc(N * var(--text-scale) + M))` rules at the
/// end of `desktop/ui/style.css`. The web heights are border boxes and
/// GTK's `min-height` is the content box; the two agree because these bars
/// have no vertical border or padding.
#[derive(Debug)]
struct ScaledHeight {
    selector: &'static str,
    /// The height at small text sizes.
    floor: i32,
    /// Pixels added per unit of text scale.
    per_scale: f64,
    /// Pixels added regardless of the text scale.
    fixed: f64,
}

impl ScaledHeight {
    /// The height at `scale`.
    fn height(&self, scale: f64) -> i32 {
        let grown = text_size::ceil_pixels(self.per_scale * scale + self.fixed);
        grown.max(self.floor)
    }

    /// The rule for this bar at `scale`.
    fn rule(&self, scale: f64) -> String {
        let height = self.height(scale);
        format!("{} {{ min-height: {height}px; }}", self.selector)
    }
}

/// The title bar (`.titlebar`).
const TITLE_BAR: ScaledHeight = ScaledHeight {
    selector: ".ox-titlebar",
    floor: 42,
    per_scale: 25.0,
    fixed: 12.0,
};

/// A tab (`.tab`).
const TAB: ScaledHeight = ScaledHeight {
    selector: ".tab",
    floor: 35,
    per_scale: 25.0,
    fixed: 7.0,
};

/// Every bar height that follows the text size.
const SCALED_HEIGHTS: &[ScaledHeight] = &[TITLE_BAR, TAB];

/// The padding of the solid window frame GTK draws without a compositor
/// (`window.ox.solid-csd` in `resources/skin/base.css`).
const SOLID_FRAME_PADDING: i32 = 3;

/// The solid window frame's title-colour band, which ends where the title
/// bar does: the frame's padding plus the title bar at `scale`. Dialogs
/// have no title bar, so they have no band.
fn solid_frame_rule(scale: f64) -> String {
    let band = SOLID_FRAME_PADDING + TITLE_BAR.height(scale);
    format!(
        "window.ox.solid-csd:not(.ox-dialog) {{ box-shadow: inset 0 {band}px @ox_title, inset 0 0 0 3px \
         @ox_border; }}"
    )
}

/// The details view's rows: `detail_row` pixels apart, less the 1-pixel
/// margin above and below each row (`.file-row` in style.css).
fn details_row_rule(detail_row: i32) -> String {
    let row = detail_row - 2;
    format!("columnview.files > listview > row {{ min-height: {row}px; }}")
}

/// Menu rows and width (`.menu button{min-height:calc(22px * s + 11px)}`
/// and `.menu.win10{width:max(264px, calc(235px * s))}`). The width rule
/// sets the contents box, inside 3px of padding and a 1px border.
fn menu_rules(scale: f64) -> String {
    let row = 22.0 * scale + 11.0;
    let width = (235.0 * scale).max(264.0) - 8.0;
    format!(
        "popover.ox-menu list > row, popover.menu.ox-menu modelbutton {{ min-height: {row:.0}px; }}\n\
         popover.ox-menu > contents, popover.menu.ox-menu > contents {{ min-width: {width:.0}px; }}"
    )
}

/// Vertical pixels of a tile's cell outside its content box: 12 pixels
/// of padding above and below (`.file-tile`) and the 2-pixel gap to the
/// next row, a 1-pixel margin on each side (style.css).
const TILE_VERTICAL_CHROME: i32 = 12 + 12 + 1 + 1;

/// A tile's size for icons of `icon_size` at `text_size`. Its height fills
/// the cell less the padding and the gap. Its width comes from the column
/// the window sets (`CellSize::columns_in` in `folder_view/grid.rs`), so the
/// minimum is only the icon, which lets GTK use every column the window
/// asks for.
fn tile_rule(icon_size: IconSize, text_size: TextSize) -> String {
    let cell = grid::cell_size(icon_size, text_size);
    let height = cell.height - TILE_VERTICAL_CHROME;
    let width = icon_size.pixels();
    let class = icon_size.css_class();
    format!("gridview.files.{class} > child {{ min-width: {width}px; min-height: {height}px; }}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stylesheet at `percent`, one of the levels.
    fn css_at(percent: u32) -> String {
        css_for_text_size(TextSize::from_percent(percent))
    }

    /// parity: VIEW-044
    #[test]
    fn the_default_text_size_draws_13_pixel_text_and_36_pixel_rows() {
        let css = css_at(100);
        assert!(css.contains("window.ox { font-size: 13.00px; }"));
        assert!(css.contains(".statusbar { font-size: 11.00px; }"));
        assert!(css.contains("columnview.files > listview > row { min-height: 36px; }"));
        assert!(css.contains("gridview.files.icons-large > child { min-width: 56px; min-height: 104px; }"));
        assert!(css
            .contains("popover.ox-menu list > row, popover.menu.ox-menu modelbutton { min-height: 33px; }"));
        assert!(
            css.contains("popover.ox-menu > contents, popover.menu.ox-menu > contents { min-width: 256px; }")
        );
    }

    /// parity: VIEW-044
    #[test]
    fn larger_text_scales_fonts_and_rows() {
        let css = css_at(200);
        assert!(css.contains("window.ox { font-size: 26.00px; }"));
        assert!(css.contains("row { min-height: 60px; }"));
    }

    /// parity: VIEW-044
    #[test]
    fn the_title_bar_and_the_frame_band_grow_together() {
        let css = css_at(200);
        assert!(css.contains(".ox-titlebar { min-height: 62px; }"));
        assert!(css.contains("box-shadow: inset 0 65px @ox_title"));
    }

    #[test]
    fn every_rule_is_well_formed() {
        let css = css_at(125);
        for line in css.lines() {
            assert!(line.ends_with('}'), "{line}");
            assert_eq!(line.matches('{').count(), 1, "{line}");
        }
    }
}
