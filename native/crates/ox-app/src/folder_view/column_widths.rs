// SPDX-License-Identifier: AGPL-3.0-only
//! The details columns' widths: where they start, what GTK is told and
//! what settings save.
//!
//! Ports `columnDefaults` and `applyColumnLayout` in `desktop/ui/app.js`:
//! Name takes the remaining width until the user resizes it, the other
//! columns default to 152, 135 and 90 pixels, and saved widths are clamped
//! to the limits the Python app uses (ox-core's [`Column::width_range`]).
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

/// The settings column of a details column.
pub(crate) const fn settings_column(column: SortColumn) -> Column {
    match column {
        SortColumn::Name => Column::Name,
        SortColumn::Modified => Column::Modified,
        SortColumn::Type => Column::Type,
        SortColumn::Size => Column::Size,
    }
}

/// The part of `column`'s width that is the list's end padding.
const fn edge_gutter(column: SortColumn) -> u32 {
    match column {
        SortColumn::Name | SortColumn::Size => EDGE_GUTTER,
        SortColumn::Modified | SortColumn::Type => 0,
    }
}

/// Width of a column nobody resized (`columnDefaults` in app.js). Name has
/// none: it takes the remaining space.
const fn default_width(column: SortColumn) -> Option<u32> {
    match column {
        SortColumn::Name => None,
        SortColumn::Modified => Some(152),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-028
    #[test]
    fn unsaved_columns_start_at_the_python_defaults() {
        assert_eq!(start_width(SortColumn::Name, None), None);
        assert_eq!(start_width(SortColumn::Modified, None), Some(152));
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
