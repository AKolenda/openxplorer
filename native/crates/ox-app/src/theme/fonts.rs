// SPDX-License-Identifier: AGPL-3.0-only
//! Font sizes and row heights that follow the text size.
//!
//! In the web interface every font size is `calc(Npx * var(--text-scale))`
//! and row metrics come from `metrics()` in text-size.js. GTK 4.14 CSS has
//! no variables, so this module generates those rules for the chosen size
//! from one table of base sizes (the values in `desktop/ui/style.css`).

use std::fmt::Write as _;

use crate::folder_view::grid::IconSize;
use crate::text_size;

/// Selector and font size in pixels at 100%.
const FONT_SIZES: &[(&str, f64)] = &[
    ("window.ox", 13.0),
    (".ox-titlebar .tab label", 12.0),
    (".address", 13.0),
    (".address entry", 13.0),
    ("entry.search", 12.0),
    (".commandbar button.text-command", 12.0),
    (".sidebar list > row", 12.0),
    (".sidebar-bottom button", 12.0),
    ("columnview.files", 12.0),
    ("columnview.files > header > button", 12.0),
    ("gridview.files", 12.0),
    (".details .detail-header", 13.0),
    (".details .dname", 16.0),
    (".details .dtype", 12.0),
    (".details button.dbutton", 12.0),
    (".details .dsection", 12.0),
    (".details .dkey", 11.0),
    (".details .dval", 11.0),
    (".details .note", 11.0),
    (".statusbar", 11.0),
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
    (".landing .recent-row", 12.0),
    (".empty-state .empty-title", 16.0),
    (".empty-state", 12.0),
    ("popover.menu.ox-menu modelbutton", 12.0),
    ("popover.menu.ox-menu accelerator", 10.0),
    ("tooltip", 12.0),
];

/// The stylesheet for a text size (percent).
pub fn css_for_text_size(percent: u32) -> String {
    let metrics = text_size::metrics(percent);
    let mut css = String::new();
    for (selector, base) in FONT_SIZES {
        let size = base * metrics.scale;
        let _ = writeln!(css, "{selector} {{ font-size: {size:.2}px; }}");
    }
    // Rows keep a 1px margin above and below, as .file-row in style.css.
    let row = metrics.detail_row - 2;
    let _ = writeln!(
        css,
        "columnview.files > listview > row {{ min-height: {row}px; }}"
    );
    for size in IconSize::ALL {
        let (width, height) = tile_size(metrics, size);
        let class = size.css_class();
        let _ = writeln!(
            css,
            "gridview.files.{class} > child {{ min-width: {width}px; min-height: {height}px; }}"
        );
    }
    css
}

/// Icon-view tile size for an icon size: the web interface's 135 × 130
/// cell for large icons, grown or shrunk with the icon, never narrower than
/// the text needs. The 6 and 8 pixels are the tile margins.
fn tile_size(metrics: text_size::Metrics, size: IconSize) -> (i32, i32) {
    let growth = size.pixels() - IconSize::Large.pixels();
    let width = metrics.grid_width.max(size.pixels() + 79) - 6;
    let height = metrics.grid_row + growth - 8;
    (width, height)
}

/// Padding and margins style.css adds around a tile's minimum width.
const TILE_CHROME: i32 = 24;

/// The narrowest an icon-view tile of `size` is at any text size, border
/// box included. The icon view uses it to bound how many columns it needs.
pub fn narrowest_tile_width(size: IconSize) -> i32 {
    let smallest_text = text_size::metrics(text_size::LEVELS[0]);
    let (width, _) = tile_size(smallest_text, size);
    width + TILE_CHROME
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
        assert!(css.contains("gridview.files.icons-large > child { min-width: 129px; min-height: 122px; }"));
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
