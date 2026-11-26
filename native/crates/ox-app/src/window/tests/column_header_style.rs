// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > Appearance > Layout > "Column header style" in a real
//! window (VIEW-014). The app's own style stays the default; Windows'
//! draws the sorted column as Windows 11 File Explorer does: only a small
//! gray chevron centered over the column, above its title, which keeps
//! its color and weight, with no line under it and no change of
//! background, while every other title draws nothing new.

use std::time::Duration;

use gtk::prelude::*;
use ox_core::settings::{ColumnHeaderStyle, PreferencesUpdate, Settings};

use super::geometry::laid_out;
use super::icons::{pixel_rows, Pixel};
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow};

/// Saves `style` as Settings or another window would, and lets the
/// window draw it.
fn save_style(test: &TestWindow, style: ColumnHeaderStyle) {
    let update = PreferencesUpdate {
        column_header_style: Some(style),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
    wait_until("the window to follow the style", || {
        test.window.column_header_style() == style
    });
    settle_colors(test);
}

/// Lets the titles' 83ms color fades (base.css, M01) end and the window
/// redraw.
fn settle_colors(test: &TestWindow) {
    wait_for(Duration::from_millis(250));
    wait_for_frames(&test.window, 2);
}

/// Sorts the window by `column` in `direction` (`ascending` or
/// `descending`), as the Sort menu does, and lets it redraw.
fn sort_by(test: &TestWindow, column: &str, direction: &str) {
    test.activate("sort", Some(column));
    test.activate("direction", Some(direction));
    settle_colors(test);
}

/// The title of `column` in the window's details view.
fn title_of(test: &TestWindow, column: SortColumn) -> gtk::Widget {
    let pane = test.window.folder_pane();
    let titles = pane.details().titles_by_column();
    titles.title(column).expect("a title for every column").clone()
}

/// The pixels `title` draws, row by row, as the header draws it in
/// place, but for its last column: the faint divider after every title,
/// drawn the titles' full height in either style, as Windows 11 draws it.
fn drawn(title: &gtk::Widget) -> Vec<Vec<Pixel>> {
    let renderer = title
        .native()
        .and_then(|native| native.renderer())
        .expect("the title is on a drawn window");
    let header = title.parent().expect("a title is in the header");
    let place = title.compute_bounds(&header).expect("the title is laid out");
    let snapshot = gtk::Snapshot::new();
    header.snapshot_child(title, &snapshot);
    let node = snapshot.to_node().expect("the title draws something");
    let mut rows = pixel_rows(&renderer.render_texture(&node, Some(&place)));
    for row in &mut rows {
        row.pop();
    }
    rows
}

/// Whether two pixels differ visibly.
fn differ(one: Pixel, other: Pixel) -> bool {
    one.iter()
        .zip(other)
        .any(|(channel, background)| channel.abs_diff(background) > 40)
}

/// The inked pixels of `rows`, those that differ from the top-left
/// corner's background, as (column, row).
fn ink(rows: &[Vec<Pixel>], limit: usize) -> Vec<(usize, usize)> {
    let background = rows[0][0];
    let mut inked = Vec::new();
    for (y, row) in rows.iter().take(limit).enumerate() {
        for (x, pixel) in row.iter().enumerate() {
            if differ(*pixel, background) {
                inked.push((x, y));
            }
        }
    }
    inked
}

/// The visible label in `title`.
fn label_of(title: &gtk::Widget) -> gtk::Label {
    descendants::<gtk::Label>(title)
        .into_iter()
        .find(WidgetExt::is_visible)
        .expect("a title has a label")
}

/// The rows of `title` above its label's text: the chevron's rows.
fn rows_above_text(title: &gtk::Widget) -> usize {
    let label = label_of(title);
    let place = label.compute_bounds(title).expect("the label is laid out");
    // A 12-pixel label is centered in its box; its text starts below it.
    let text_top = place.y() + (place.height() - 16.0) / 2.0;
    (0_u16..=u16::MAX)
        .take_while(|row| f32::from(*row) + 1.0 <= text_top)
        .count()
}

/// Each column's title, with the window sorted by Date modified, which is
/// not the default sort.
const COLUMNS: [SortColumn; 4] = [
    SortColumn::Name,
    SortColumn::Modified,
    SortColumn::Type,
    SortColumn::Size,
];

/// A new window keeps the app's own column titles, and the window follows
/// the style Settings saves at once.
///
/// parity: VIEW-014
#[gtk::test]
fn the_apps_own_column_titles_stay_the_default() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert_eq!(test.window.column_header_style(), ColumnHeaderStyle::OpenXplorer);
    save_style(&test, ColumnHeaderStyle::Windows);
    save_style(&test, ColumnHeaderStyle::OpenXplorer);
}

/// With Windows' style, a sort other than the default draws no accent and
/// no line: the sorted title draws exactly what it drew unsorted, but for
/// its chevron above the text, and its label keeps its color.
///
/// parity: VIEW-014
#[gtk::test]
fn windows_style_draws_no_accent_or_line_on_the_sorted_title() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    save_style(&test, ColumnHeaderStyle::Windows);
    let modified = title_of(&test, SortColumn::Modified);
    let unsorted = drawn(&modified);
    let unsorted_color = label_of(&modified).color();
    let chevron_rows = rows_above_text(&modified);
    assert!(chevron_rows >= 5, "room above the text: {chevron_rows}");

    for direction in ["ascending", "descending"] {
        sort_by(&test, "modified", direction);
        let sorted = drawn(&modified);
        assert_eq!(sorted.len(), unsorted.len());
        for (y, (row, before)) in sorted.iter().zip(&unsorted).enumerate().skip(chevron_rows) {
            for (x, (pixel, was)) in row.iter().zip(before).enumerate() {
                assert!(
                    !differ(*pixel, *was),
                    "{direction}: the sorted title changed at ({x}, {y}): {pixel:?} for {was:?}"
                );
            }
        }
        let label = label_of(&modified);
        assert_eq!(label.color(), unsorted_color, "the title keeps its color");
        for other in COLUMNS
            .into_iter()
            .filter(|column| *column != SortColumn::Modified)
        {
            assert_eq!(label_of(&title_of(&test, other)).color(), unsorted_color);
        }
    }
}

/// With Windows' style, only the sorted title draws a chevron: a small
/// gray one centered over its column, at the top of the title, up while
/// ascending and down while descending, as Windows 11's 8 by 5 pixels at
/// 100% scale.
///
/// parity: VIEW-014
#[gtk::test]
fn windows_style_shows_one_small_chevron_over_the_sorted_column() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    save_style(&test, ColumnHeaderStyle::Windows);
    for (direction, upward) in [("ascending", true), ("descending", false)] {
        sort_by(&test, "modified", direction);
        for column in COLUMNS {
            let title = title_of(&test, column);
            let indicator = descendants::<gtk::Widget>(&title)
                .into_iter()
                .find(|child| child.css_name() == "sort-indicator")
                .expect("a title has a sort indicator");
            assert_eq!(indicator.width(), 0, "GTK's own arrow takes no room");
            let inked = ink(&drawn(&title), rows_above_text(&title));
            if column != SortColumn::Modified {
                assert!(inked.is_empty(), "{column:?} has no arrow: {inked:?}");
                continue;
            }
            let left = inked.iter().map(|ink| ink.0).min().expect("a chevron");
            let right = inked.iter().map(|ink| ink.0).max().expect("a chevron");
            let top = inked.iter().map(|ink| ink.1).min().expect("a chevron");
            let bottom = inked.iter().map(|ink| ink.1).max().expect("a chevron");
            let (width, height) = (right - left + 1, bottom - top + 1);
            assert!((8..=10).contains(&width), "{direction}: {width} wide");
            assert!((4..=6).contains(&height), "{direction}: {height} high");
            assert!(top <= 2, "{direction}: at the title's top edge, from row {top}");
            let middle = f64::from(u32::try_from(left + right).expect("a narrow title")) / 2.0;
            // The title's whole width, its padding too; the divider's 1px
            // is at its right.
            let place = title.compute_bounds(&title).expect("the title is laid out");
            let column_middle = (f64::from(place.width()) - 2.0) / 2.0;
            assert!(
                (middle - column_middle).abs() <= 1.0,
                "{direction}: centered at {middle}, the column's middle {column_middle}"
            );
            let span = |row: usize| {
                let row: Vec<usize> = inked.iter().filter(|ink| ink.1 == row).map(|ink| ink.0).collect();
                row.iter()
                    .max()
                    .zip(row.iter().min())
                    .map_or(0, |(max, min)| max - min)
            };
            assert_eq!(
                span(top) < span(bottom),
                upward,
                "{direction}: rows {} and {}",
                span(top),
                span(bottom)
            );
        }
    }
}

/// Back on the app's own style, a sort other than the default draws the
/// owner's look again: the sorted title changes below its top rows.
///
/// parity: VIEW-014
#[gtk::test]
fn the_apps_own_style_comes_back_with_its_accent() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    save_style(&test, ColumnHeaderStyle::Windows);
    let modified = title_of(&test, SortColumn::Modified);
    let unsorted = drawn(&modified);
    let chevron_rows = rows_above_text(&modified);
    sort_by(&test, "modified", "ascending");
    save_style(&test, ColumnHeaderStyle::OpenXplorer);
    let sorted = drawn(&modified);
    let changed = sorted
        .iter()
        .zip(&unsorted)
        .skip(chevron_rows)
        .any(|(row, before)| row.iter().zip(before).any(|(pixel, was)| differ(*pixel, *was)));
    assert!(changed, "the app's own style marks a sort other than the default");
}

/// With Windows' style the chevron stands at the title's top edge, well
/// clear of the text under it (3 pixels above its capitals, as in Windows
/// 11), in a column as wide as a new window's, in one narrower than its
/// title, where the text runs under the middle, and in a wide one, in
/// normal and in Compact view, whose titles are six pixels shorter.
///
/// parity: VIEW-014, VIEW-067
#[gtk::test]
fn windows_style_keeps_the_chevron_clear_of_the_text_in_any_column_width() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    save_style(&test, ColumnHeaderStyle::Windows);
    let view = test.window.folder_pane().details();
    let modified = view.column(SortColumn::Modified).expect("a Date modified column");
    let normal_height = title_of(&test, SortColumn::Modified).height();
    // A new window's width, then one narrower than the title and one wider
    // than a new window's, in each density; each case sets its own.
    let new_window_width = modified.fixed_width();
    let cases = [
        (false, new_window_width),
        (false, 70),
        (false, 320),
        (true, new_window_width),
        (true, 70),
        (true, 320),
    ];
    let mut is_compact = false;
    for (compact, width) in cases {
        if compact != is_compact {
            test.activate("compact-density", None);
            wait_until("the compact titles", || {
                title_of(&test, SortColumn::Modified).height() < normal_height
            });
            settle_colors(&test);
            is_compact = compact;
        }
        let before = title_of(&test, SortColumn::Modified).width();
        let changes = modified.fixed_width() != width;
        modified.set_fixed_width(width);
        if changes {
            wait_until("the column laid out at its new width", || {
                title_of(&test, SortColumn::Modified).width() != before
            });
        }
        wait_for_frames(&test.window, 3);
        for direction in ["ascending", "descending"] {
            sort_by(&test, "modified", direction);
            let title = title_of(&test, SortColumn::Modified);
            let rows = drawn(&title);
            // Every inked row, the line under the header left out.
            let inked_rows: Vec<usize> = (0..rows.len() - 1)
                .filter(|row| !ink(&rows[*row..=*row], 1).is_empty())
                .collect();
            let top = *inked_rows.first().expect("a chevron");
            assert!(
                top <= 2,
                "compact {compact}, {width:?} {direction}: from row {top}"
            );
            let chevron_end = inked_rows
                .iter()
                .zip(inked_rows.iter().skip(1))
                .find(|(row, next)| **next > **row + 1)
                .map(|(row, _)| *row);
            let Some(chevron_end) = chevron_end else {
                panic!("compact {compact}, {width:?} {direction}: text clear of the chevron");
            };
            // The capitals' top, where the title's first letter, the "D"
            // of "Date modified", starts at the text's left edge: a font's
            // taller letters, such as "d" and "f", may stand a pixel above
            // its capitals.
            let text = (chevron_end + 1..rows.len() - 1).flat_map(|row| ink(&rows[row..=row], 1));
            let left = text.map(|(x, _)| x).min().expect("text under the chevron");
            let capitals_top = (chevron_end + 1..rows.len() - 1)
                .find(|row| {
                    ink(&rows[*row..=*row], 1)
                        .iter()
                        .any(|(x, _)| (left..left + 3).contains(x))
                })
                .expect("text under the chevron");
            assert!(
                capitals_top - chevron_end > 3,
                "compact {compact}, {width:?} {direction}: the chevron ends at row {chevron_end}, the capitals start at {capitals_top}"
            );
        }
    }
    test.activate("compact-density", None);
}
