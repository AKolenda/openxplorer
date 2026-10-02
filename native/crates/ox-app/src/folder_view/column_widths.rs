// SPDX-License-Identifier: AGPL-3.0-only
//! The details columns' widths: where they start, what GTK is told and
//! what settings save.
//!
//! Ports `columnDefaults` and `applyColumnLayout` in `v2.0.0:desktop/ui/app.js`:
//! Name takes the remaining width until the user resizes it, the other
//! columns default to 176 (Date modified), 330 (Folder path), 135 and 90
//! pixels, and saved widths are clamped to the limits the Python app uses (ox-core's [`Column::width_range`]).
//! The details view applies these widths and saves them after a resize.

use ox_core::settings::{Column, ColumnWidths};

use crate::folder_view::sorting::SortColumn;

/// The web list pads its column header and rows 14 pixels at both ends
/// (`.column-head{padding:0 14px}`), so its columns stop short of the
/// list's edges. GTK lays columns out across the whole column view, so
/// the first and last columns, Name and Size, hold those pixels: they are
/// this much wider than the widths saved in settings, and their titles and
/// cells pad for it (resources/style.css).
const EDGE_GUTTER: u32 = 14;

/// GTK's fixed width for a column without one, which shares the space.
const NO_FIXED_WIDTH: i32 = -1;

/// The settings column of a details column. The Recycle Bin's columns
/// share the width of the column they stand in for: Original location
/// that of Folder path, Date deleted that of Date modified.
pub(crate) const fn settings_column(column: SortColumn) -> Column {
    match column {
        SortColumn::Name => Column::Name,
        SortColumn::Modified | SortColumn::Deleted => Column::Modified,
        SortColumn::FolderPath | SortColumn::OriginalLocation => Column::ParentUri,
        SortColumn::Type => Column::Type,
        SortColumn::Size => Column::Size,
    }
}

/// The part of `column`'s width that is the list's end padding.
const fn edge_gutter(column: SortColumn) -> u32 {
    match column {
        SortColumn::Name | SortColumn::Size => EDGE_GUTTER,
        SortColumn::Modified
        | SortColumn::FolderPath
        | SortColumn::OriginalLocation
        | SortColumn::Deleted
        | SortColumn::Type => 0,
    }
}

/// Width of a column nobody resized (`columnDefaults` in app.js). Name has
/// none: it takes the remaining space. Date modified shows the time too,
/// so it starts wider than the web app's 152 pixels, enough for
/// `12/31/2026 11:59 PM` at 125% text size.
const fn default_width(column: SortColumn) -> Option<u32> {
    match column {
        SortColumn::Name => None,
        SortColumn::Modified | SortColumn::Deleted => Some(176),
        SortColumn::FolderPath | SortColumn::OriginalLocation => Some(330),
        SortColumn::Type => Some(135),
        SortColumn::Size => Some(90),
    }
}

/// The width `column` starts with: the saved width within the Python
/// app's limits, else its default. `None` lets Name fill the space.
pub(crate) fn start_width(column: SortColumn, saved: Option<&ColumnWidths>) -> Option<u32> {
    let limits = settings_column(column).width_range();
    let saved = saved.and_then(|widths| widths.get(settings_column(column)));
    let width = saved.or(default_width(column))?;
    Some(width.clamp(*limits.start(), *limits.end()))
}

/// GTK's fixed width for `column` starting `width` pixels wide: the width
/// and the column's end gutter, or [`NO_FIXED_WIDTH`] while Name fills the
/// space.
pub(crate) fn fixed_width(column: SortColumn, width: Option<u32>) -> i32 {
    let Some(width) = width else {
        return NO_FIXED_WIDTH;
    };
    i32::try_from(width + edge_gutter(column)).unwrap_or(NO_FIXED_WIDTH)
}

/// The width settings save for a column `fixed_width` pixels wide, or
/// `None` while it has no width of its own.
pub(crate) fn saved_width(column: SortColumn, fixed_width: i32) -> Option<f64> {
    let width = u32::try_from(fixed_width).ok().filter(|width| *width > 0)?;
    let without_gutter = width.saturating_sub(edge_gutter(column));
    Some(f64::from(without_gutter))
}

/// Room a fitted column keeps beside its widest text (`fitColumn` in
/// app.js): the icon, its gap and the padding in Name, the padding
/// elsewhere.
const fn fit_padding(column: SortColumn) -> f64 {
    match column {
        SortColumn::Name => 64.0,
        SortColumn::Modified
        | SortColumn::FolderPath
        | SortColumn::OriginalLocation
        | SortColumn::Deleted
        | SortColumn::Type
        | SortColumn::Size => 30.0,
    }
}

/// The width `column` may have, without its end gutter.
pub(crate) fn width_limits(column: SortColumn) -> std::ops::RangeInclusive<u32> {
    settings_column(column).width_range()
}

/// `fixed_width`, a GTK fixed width the user dragged `column` to, kept
/// within the column's limits (`setColumnWidth` in app.js).
pub(crate) fn clamped_fixed_width(column: SortColumn, fixed_width: i32) -> i32 {
    let limits = width_limits(column);
    let gutter = i32::try_from(edge_gutter(column)).unwrap_or_default();
    let low = i32::try_from(*limits.start()).unwrap_or(i32::MAX);
    let high = i32::try_from(*limits.end()).unwrap_or(i32::MAX);
    (fixed_width - gutter).clamp(low, high) + gutter
}

/// The width that fits `column` to its widest text, `widest_text` pixels,
/// within the column's limits (`fitColumn`).
pub(crate) fn fitted_width(column: SortColumn, widest_text: f64) -> u32 {
    let limits = width_limits(column);
    let wanted = (widest_text + fit_padding(column)).ceil();
    let low = f64::from(*limits.start());
    let high = f64::from(*limits.end());
    // Clamped to the limits, so the value fits a `u32`.
    let fitted = wanted.clamp(low, high);
    fitted_pixels(fitted)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the width is clamped to the column limits first"
)]
fn fitted_pixels(width: f64) -> u32 {
    width as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-028
    #[test]
    fn unsaved_columns_start_at_the_python_defaults() {
        assert_eq!(start_width(SortColumn::Name, None), None);
        assert_eq!(start_width(SortColumn::Modified, None), Some(176));
        assert_eq!(start_width(SortColumn::FolderPath, None), Some(330));
        assert_eq!(start_width(SortColumn::Type, None), Some(135));
        assert_eq!(start_width(SortColumn::Size, None), Some(90));
    }

    /// parity: VIEW-028
    #[test]
    fn saved_widths_are_used_within_their_limits() {
        let saved = ColumnWidths {
            name: Some(300),
            modified: Some(40),
            size: Some(5000),
            ..ColumnWidths::default()
        };
        assert_eq!(start_width(SortColumn::Name, Some(&saved)), Some(300));
        assert_eq!(start_width(SortColumn::Modified, Some(&saved)), Some(100));
        assert_eq!(start_width(SortColumn::Size, Some(&saved)), Some(600));
        assert_eq!(start_width(SortColumn::Type, Some(&saved)), Some(135));
    }

    /// parity: VIEW-028
    #[test]
    fn a_dragged_column_stops_at_its_limits() {
        assert_eq!(clamped_fixed_width(SortColumn::Name, 40), 140 + 14);
        assert_eq!(clamped_fixed_width(SortColumn::Name, 400), 400);
        assert_eq!(clamped_fixed_width(SortColumn::Modified, 2000), 1000);
        assert_eq!(clamped_fixed_width(SortColumn::Type, 10), 80);
        assert_eq!(clamped_fixed_width(SortColumn::Size, 900), 600 + 14);
    }

    /// Ported from `fitColumn` in `v2.0.0:desktop/ui/app.js`: the widest text and
    /// its padding, never below or above the column's limits.
    ///
    /// parity: VIEW-029
    #[test]
    fn a_fitted_column_holds_its_widest_text_within_its_limits() {
        assert_eq!(fitted_width(SortColumn::Name, 200.4), 265);
        assert_eq!(fitted_width(SortColumn::Type, 90.0), 120);
        assert_eq!(
            fitted_width(SortColumn::Size, 10.0),
            70,
            "never below the minimum"
        );
        assert_eq!(
            fitted_width(SortColumn::Modified, 5000.0),
            1000,
            "never above the maximum"
        );
    }

    #[test]
    fn the_end_columns_hold_the_lists_padding() {
        assert_eq!(fixed_width(SortColumn::Name, None), NO_FIXED_WIDTH);
        assert_eq!(fixed_width(SortColumn::Name, Some(300)), 314);
        assert_eq!(fixed_width(SortColumn::Modified, Some(152)), 152);
        assert_eq!(fixed_width(SortColumn::Size, Some(90)), 104);
    }

    /// parity: VIEW-028
    #[test]
    fn saved_widths_leave_out_the_end_columns_padding() {
        assert_eq!(saved_width(SortColumn::Size, 104), Some(90.0));
        assert_eq!(saved_width(SortColumn::Name, 314), Some(300.0));
        assert_eq!(saved_width(SortColumn::Type, 135), Some(135.0));
        assert_eq!(
            saved_width(SortColumn::Name, -1),
            None,
            "Name still fills the space"
        );
    }
}
