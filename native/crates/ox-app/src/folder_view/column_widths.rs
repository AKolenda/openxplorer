// SPDX-License-Identifier: AGPL-3.0-only
//! The details columns' widths: where they start, what GTK is told and
//! what settings save.
//!
//! Ports `columnDefaults` and `applyColumnLayout` in `v2.0.0:desktop/ui/app.js`:
//! Name takes the remaining width until the user resizes it, the other
//! columns default to 176 (Date modified), 330 (Folder path), 135 (Type),
//! 130 (Size) and 90 (Extension) pixels, and saved widths are clamped to the limits the Python app uses (ox-core's [`Column::width_range`]).
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
        SortColumn::Created => Column::Created,
        SortColumn::Extension => Column::Extension,
        SortColumn::Owner => Column::Owner,
        SortColumn::Permissions => Column::Permissions,
    }
}

/// The part of `column`'s width that is the list's end padding.
const fn edge_gutter(column: SortColumn) -> u32 {
    match column {
        SortColumn::Name | SortColumn::Size => EDGE_GUTTER,
        _ => 0,
    }
}

/// Width of a column nobody resized (`columnDefaults` in app.js). Name has
/// none: it takes the remaining space. Date modified shows the time too,
/// so it starts wider than the web app's 152 pixels, enough for
/// `12/31/2026 11:59 PM` at 125% text size. Size keeps the web app's 90
/// pixels here, but the view widens it to [`size_minimum`], measured in the
/// list's font, which is wider for a size such as `6,291,456 KB`.
const fn default_width(column: SortColumn) -> Option<u32> {
    match column {
        SortColumn::Name => None,
        SortColumn::Modified | SortColumn::Created | SortColumn::Deleted => Some(176),
        SortColumn::FolderPath | SortColumn::OriginalLocation => Some(330),
        SortColumn::Type => Some(135),
        SortColumn::Size | SortColumn::Extension => Some(90),
        SortColumn::Owner | SortColumn::Permissions => Some(110),
    }
}

/// The width Windows 11 gives the Name column at 100% text, when Settings
/// asks for it ("Name column starts at Windows' width").
pub(crate) const WINDOWS_NAME_WIDTH: u32 = 256;

/// The text size, in percent, at which `column`'s default width was
/// chosen: the dates' at 125%, which their default already fits, the
/// others' at 100%.
const fn default_fits_percent(column: SortColumn) -> f64 {
    match column {
        SortColumn::Modified | SortColumn::Created | SortColumn::Deleted => 125.0,
        _ => 100.0,
    }
}

/// The default width of `column` at a text drawn `scale` times its 100%
/// size: as much wider as the text is larger than the size the default
/// was chosen for, never narrower than the default, within the column's
/// limits. Size has its own measured minimum ([`size_minimum`]) and is
/// not scaled. `None` for Name, which fills the space.
pub(crate) fn scaled_default_width(column: SortColumn, scale: f64) -> Option<u32> {
    let width = default_width(column)?;
    if column == SortColumn::Size {
        return Some(width);
    }
    let growth = (scale * 100.0 / default_fits_percent(column)).max(1.0);
    let limits = width_limits(column);
    let scaled = (f64::from(width) * growth).round();
    Some(fitted_pixels(
        scaled.clamp(f64::from(*limits.start()), f64::from(*limits.end())),
    ))
}

/// The width `column` starts with: the saved width within the Python
/// app's limits, else its default. `None` lets Name fill the space.
pub(crate) fn start_width(column: SortColumn, saved: Option<&ColumnWidths>) -> Option<u32> {
    start_width_at(column, saved, 1.0)
}

/// [`start_width`] with text drawn `scale` times its 100% size: a column
/// without a saved width starts at [`scaled_default_width`]. A saved width
/// is a width at 100% and grows with the text, as Windows' scaling grows
/// a column with the rest of the layout.
pub(crate) fn start_width_at(column: SortColumn, saved: Option<&ColumnWidths>, scale: f64) -> Option<u32> {
    let limits = settings_column(column).width_range();
    let saved = saved.and_then(|widths| widths.get(settings_column(column)));
    let Some(saved) = saved else {
        return scaled_default_width(column, scale).map(|width| width.clamp(*limits.start(), *limits.end()));
    };
    Some(scaled_width(saved.clamp(*limits.start(), *limits.end()), scale))
}

/// `width`, a width at 100%, with text drawn `scale` times that size.
pub(crate) fn scaled_width(width: u32, scale: f64) -> u32 {
    fitted_pixels((f64::from(width) * scale).round())
}

/// The size the Size column fits whole at its narrowest and by default:
/// `9,999,999 KB`, 9.5 GB, as Windows' Size column writes it.
pub(crate) const SIZE_THAT_FITS: u64 = 9_999_999 * 1024;

/// A Size cell's padding beside its text, without the list's end gutter:
/// 12 pixels at the start and 26 at the end, 14 of them the gutter
/// (folder-views.css), and 2 more so rounding never cuts the last digit.
const SIZE_TEXT_PADDING: i32 = 12 + 26 - 14 + 2;

/// The narrowest width Size may have, in the pixels settings save, when
/// [`SIZE_THAT_FITS`] is `text_width` pixels wide in the list's font:
/// the text and its padding, within the column's limits. A larger desktop
/// font or text size makes it wider, so the size is never cut short.
pub(crate) fn size_minimum(text_width: i32) -> u32 {
    let limits = width_limits(SortColumn::Size);
    let wanted = u32::try_from(text_width + SIZE_TEXT_PADDING).unwrap_or(0);
    wanted.clamp(*limits.start(), *limits.end())
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
        _ => 30.0,
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

    /// Larger text widens the default widths by as much as the text grew
    /// past the size each default was chosen for; smaller text keeps them.
    ///
    /// parity: ACC-012, VIEW-028
    #[test]
    fn default_widths_grow_with_the_text() {
        assert_eq!(scaled_default_width(SortColumn::Type, 1.0), Some(135));
        assert_eq!(scaled_default_width(SortColumn::Type, 2.0), Some(270));
        assert_eq!(scaled_default_width(SortColumn::Type, 0.8), Some(135));
        assert_eq!(scaled_default_width(SortColumn::Modified, 1.25), Some(176));
        assert_eq!(scaled_default_width(SortColumn::Modified, 2.0), Some(282));
        assert_eq!(
            scaled_default_width(SortColumn::Size, 2.0),
            Some(90),
            "Size is measured"
        );
        assert_eq!(scaled_default_width(SortColumn::Name, 2.0), None);
        assert_eq!(start_width_at(SortColumn::Type, None, 2.0), Some(270));
        let saved = ColumnWidths {
            file_type: Some(150),
            ..ColumnWidths::default()
        };
        assert_eq!(
            start_width_at(SortColumn::Type, Some(&saved), 2.0),
            Some(300),
            "a saved width is one at 100% and grows with the text"
        );
    }

    /// Size's narrowest width is its measured text and padding, never
    /// below the Python app's 70 pixels nor above its 600.
    ///
    /// parity: VIEW-028
    #[test]
    fn size_holds_its_measured_text() {
        assert_eq!(size_minimum(80), 106);
        assert_eq!(size_minimum(10), 70);
        assert_eq!(size_minimum(900), 600);
    }
}
