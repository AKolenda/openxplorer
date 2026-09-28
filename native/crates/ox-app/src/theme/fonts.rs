// SPDX-License-Identifier: AGPL-3.0-only
//! Font sizes and row heights that follow the text size.
//!
//! In the web interface every font size is `calc(Npx * var(--text-scale))`
//! and row metrics come from `metrics()` in text-size.js. GTK 4.14 CSS has
//! no variables, so this module generates those rules for the chosen size
//! from one table of base sizes (the values in `desktop/ui/style.css`).

use std::fmt::Write as _;

use crate::folder_view::grid::{self, IconSize};
use crate::text_size;

/// Selector and font size in pixels at 100%.
const FONT_SIZES: &[(&str, f64)] = &[
    ("window.ox", 13.0),
    (".ox-titlebar .tab label", 12.0),
    (".address", 13.0),
    (".address button.crumb", 12.0),
    (".address entry", 13.0),
    (".search-wrap entry", 12.0),
    (".commandbar .text-command", 12.0),
    (".sidebar list > row", 12.0),
    (".sidebar-bottom button", 12.0),
    ("columnview.files", 12.0),
    ("columnview.files > header > button", 12.0),
    ("gridview.files", 12.0),
    (".details .detail-header", 13.0),
    (".details .detail-name", 16.0),
    (".details .detail-type", 12.0),
    (".details button.detail-button", 12.0),
    (".details .detail-section", 12.0),
    (".details .detail-key", 11.0),
    (".details .detail-value", 11.0),
    (".details .detail-note", 11.0),
    (".statusbar", 11.0),
    (".statusbar .status-mode", 10.0),
    (".toast", 12.0),
    (".landing .page-title", 24.0),
    (".landing .page-subtitle", 12.0),
    (".landing .section-title", 13.0),
    (".landing .section-title button", 11.0),
    (".landing .card-name", 12.0),
    (".landing .drive-card .card-name", 13.0),
    (".landing .card-sub", 11.0),
    (".landing .connected", 10.0),
    (".landing .quiet", 12.0),
    (".landing .notice", 12.0),
    (".landing .banner-hint", 12.0),
    (".landing .primary", 12.0),
    (".landing .network-manual button", 11.0),
    (".landing .network-manual entry", 12.0),
    (".landing .network-count", 11.0),
    (".landing .discovery-note", 11.0),
    (".empty-state .empty-title", 16.0),
    (".empty-state", 12.0),
    ("popover.ox-menu list > row", 12.0),
    ("popover.ox-menu .shortcut", 10.0),
    ("popover.menu.ox-menu modelbutton", 12.0),
    ("popover.menu.ox-menu accelerator", 10.0),
    ("tooltip", 12.0),
];

/// A bar or row height that grows with the text, as the
/// `min-height: max(floor, calc(N * var(--text-scale) + M))` rules at the
/// end of `desktop/ui/style.css`. Heights are border boxes, as in the web
/// stylesheet; `border` is subtracted because GTK's `min-height` is the
/// content box.
struct ScaledHeight {
    selector: &'static str,
    /// The height at small text sizes.
    floor: i32,
    /// Pixels added per unit of text scale.
    per_scale: f64,
    /// Pixels added regardless of the text scale.
    fixed: f64,
    /// Border and padding pixels inside the height.
    border: i32,
}

impl ScaledHeight {
    /// The border-box height at `scale`.
    fn height(&self, scale: f64) -> i32 {
        text_size::ceil_pixels(self.per_scale * scale + self.fixed).max(self.floor)
    }
}

/// The title bar (`.titlebar`).
const TITLE_BAR: ScaledHeight = ScaledHeight {
    selector: ".ox-titlebar",
    floor: 42,
    per_scale: 25.0,
    fixed: 12.0,
    border: 0,
};

/// Every height that follows the text size.
const SCALED_HEIGHTS: &[ScaledHeight] = &[
    TITLE_BAR,
    ScaledHeight {
        selector: ".tab",
        floor: 35,
        per_scale: 25.0,
        fixed: 7.0,
        border: 0,
    },
];

/// The title bar's height at `scale`.
fn title_bar_height(scale: f64) -> i32 {
    TITLE_BAR.height(scale)
}

/// The stylesheet for a text size (percent).
pub fn css_for_text_size(percent: u32) -> String {
    let metrics = text_size::metrics(percent);
    let mut css = String::new();
    for (selector, base) in FONT_SIZES {
        let size = base * metrics.scale;
        let _ = writeln!(css, "{selector} {{ font-size: {size:.2}px; }}");
    }
    for rule in SCALED_HEIGHTS {
        let height = rule.height(metrics.scale) - rule.border;
        let _ = writeln!(css, "{} {{ min-height: {height}px; }}", rule.selector);
    }
    // The solid window frame's title-colour band ends where the title bar
    // does: 3px of frame padding plus the title bar (style.css).
    let band = 3 + title_bar_height(metrics.scale);
    let _ = writeln!(
        css,
        "window.ox.solid-csd {{ box-shadow: inset 0 {band}px @ox_title, inset 0 0 0 3px @ox_border; }}"
    );
    // Rows keep a 1px margin above and below, as .file-row in style.css.
    let row = metrics.detail_row - 2;
    let _ = writeln!(
        css,
        "columnview.files > listview > row {{ min-height: {row}px; }}"
    );
    let _ = writeln!(css, "{}", menu_css(metrics.scale));
    for size in IconSize::ALL {
        let _ = writeln!(css, "{}", tile_css(size, percent));
    }
    css
}

/// Vertical pixels of a tile's cell outside its content box: 12 pixels
/// of padding above and below (`.file-tile`) and the 2-pixel gap to the
/// next row, a 1-pixel margin on each side (style.css).
const TILE_VERTICAL_CHROME: i32 = 12 + 12 + 1 + 1;

/// A tile's size for icons of `size`. Its height fills the cell less the
/// padding and the gap. Its width comes from the column the window sets
/// (`CellSize::columns_in` in `folder_view/grid.rs`), so the minimum is only
/// the icon, which lets GTK use every column the window asks for.
fn tile_css(size: IconSize, percent: u32) -> String {
    let cell = grid::cell_size(size, percent);
    let height = cell.height - TILE_VERTICAL_CHROME;
    let width = size.pixels();
    let class = size.css_class();
    format!("gridview.files.{class} > child {{ min-width: {width}px; min-height: {height}px; }}")
}

/// Menu rows and width (`.menu button{min-height:calc(22px * s + 11px)}`
/// and `.menu.win10{width:max(264px, calc(235px * s))}`). The width rule
/// sets the contents box, inside 3px of padding and a 1px border.
fn menu_css(scale: f64) -> String {
    let row = 22.0 * scale + 11.0;
    let width = (235.0 * scale).max(264.0) - 8.0;
    format!(
        "popover.ox-menu list > row, popover.menu.ox-menu modelbutton {{ min-height: {row:.0}px; }}\n\
         popover.ox-menu > contents, popover.menu.ox-menu > contents {{ min-width: {width:.0}px; }}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_matches_the_web_stylesheet() {
        let css = css_for_text_size(100);
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

    #[test]
    fn larger_text_scales_fonts_and_rows() {
        let css = css_for_text_size(200);
        assert!(css.contains("window.ox { font-size: 26.00px; }"));
        assert!(css.contains("row { min-height: 60px; }"));
    }

    #[test]
    fn every_rule_is_well_formed() {
        let css = css_for_text_size(125);
        for line in css.lines() {
            assert!(line.ends_with('}'), "{line}");
            assert_eq!(line.matches('{').count(), 1, "{line}");
        }
    }
}
