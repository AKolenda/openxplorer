// SPDX-License-Identifier: AGPL-3.0-only
//! A group's heading looks as Windows 11 draws it, in every view and with
//! Compact view on or off (VIEW-022): its title in regular weight about a
//! quarter larger than the items' names (15 pixels to their 12 at 100%),
//! and no line after it.

use gtk::pango;
use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;

/// A folder and two text files, grouped by type in the window.
fn grouped_window(fixture: &Fixture, compact: bool) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    if test.window.shows_compact_density() != compact {
        test.activate("compact-density", None);
    }
    test.activate("group-by", Some("type"));
    wait_for_frames(&test.window, 4);
    test
}

/// Shows `view` and waits for its headings.
fn show(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    wait_until("the view's headings", || {
        descendants::<gtk::Label>(&test.window.folder_pane().view_widget())
            .iter()
            .any(|label| label.is_mapped() && label.has_css_class("group-title"))
    });
    wait_for_frames(&test.window, 3);
}

/// A label's font size in whole pixels and its weight.
fn font_of(label: &gtk::Label) -> (i32, pango::Weight) {
    let font = label
        .pango_context()
        .font_description()
        .expect("the label has a font");
    assert!(font.is_size_absolute(), "a size in pixels: {font}");
    (font.size() / pango::SCALE, font.weight())
}

/// In Details, Tiles and Content, normal and compact, a heading's title is
/// 15 pixels in regular weight, where the items' names are 12, and nothing
/// draws a line after it.
///
/// parity: VIEW-022, VIEW-067, VIEW-071
#[gtk::test]
fn a_heading_has_windows_11_s_larger_regular_title_and_no_line() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
    fixture.write("Notes 1.txt");
    fixture.write("Notes 2.txt");
    for compact in [false, true] {
        let test = grouped_window(&fixture, compact);
        for view in [FolderView::Details, FolderView::Tiles, FolderView::Content] {
            show(&test, view);
            let shown = test.window.folder_pane().view_widget();
            let titles: Vec<gtk::Label> = descendants::<gtk::Label>(&shown)
                .into_iter()
                .filter(|label| label.is_mapped() && label.has_css_class("group-title"))
                .collect();
            assert!(!titles.is_empty(), "{view:?}, compact {compact}: headings");
            for title in &titles {
                assert_eq!(
                    font_of(title),
                    (15, pango::Weight::Normal),
                    "{view:?}, compact {compact}: {:?}",
                    title.text()
                );
            }
            let name = descendants::<FileCell>(&shown)
                .into_iter()
                .find(|cell| cell.is_mapped() && cell.name() == "Notes 1")
                .and_then(|cell| {
                    descendants::<gtk::Label>(&cell)
                        .into_iter()
                        .find(|label| label.is_mapped() && label.text() == "Notes 1")
                })
                .expect("an item's name");
            assert_eq!(font_of(&name).0, 12, "{view:?}: the items' names");
            let lines: Vec<gtk::Separator> = descendants::<gtk::Separator>(&shown)
                .into_iter()
                .filter(WidgetExt::is_mapped)
                .collect();
            assert!(
                lines.is_empty(),
                "{view:?}, compact {compact}: no line after a heading"
            );
        }
    }
}

/// The left and right insets of `cell` within the grid's slot holding it.
fn insets_in_slot(cell: &FileCell, grid: &gtk::Widget) -> (f32, f32) {
    let slot = cell.parent().expect("the cell is in a slot");
    let inner = cell.compute_bounds(grid).expect("the cell is laid out");
    let outer = slot.compute_bounds(grid).expect("the slot is laid out");
    (
        inner.x() - outer.x(),
        outer.x() + outer.width() - (inner.x() + inner.width()),
    )
}

/// In Tiles and Content, normal and compact, a heading's hover and
/// selection box starts and ends as far in from the row's edges as an
/// item's box does, as Windows 11 draws them: its first slot is inset as
/// far as an item's on the left, its last as far on the right.
///
/// parity: VIEW-022, VIEW-071
#[gtk::test]
fn a_heading_s_box_is_inset_as_an_item_s() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
    fixture.write("Notes 1.txt");
    fixture.write("Notes 2.txt");
    for compact in [false, true] {
        let test = grouped_window(&fixture, compact);
        for view in [FolderView::Tiles, FolderView::Content] {
            show(&test, view);
            let grid = test.window.folder_pane().view_widget();
            let cells = descendants::<FileCell>(&grid);
            let item = cells
                .iter()
                .find(|cell| cell.is_mapped() && cell.group_heading().is_none() && cell.name() == "Notes 1")
                .expect("an item");
            let in_row = |class: &str| {
                cells
                    .iter()
                    .find(|cell| {
                        cell.is_mapped() && cell.has_css_class("heading-row") && cell.has_css_class(class)
                    })
                    .unwrap_or_else(|| panic!("a heading's {class} slot"))
                    .clone()
            };
            let (item_left, item_right) = insets_in_slot(item, &grid);
            let (start_left, _) = insets_in_slot(&in_row("row-start"), &grid);
            let (_, end_right) = insets_in_slot(&in_row("row-end"), &grid);
            assert!(
                (start_left - item_left).abs() < 0.5 && (end_right - item_right).abs() < 0.5,
                "{view:?}, compact {compact}: heading inset {start_left}, {end_right}; item {item_left}, {item_right}"
            );
        }
    }
}

/// The box a heading's hover, selection and focus are drawn on: the
/// heading's own box in Details, its first slot's cell in a grid.
fn heading_box(test: &TestWindow, view: FolderView) -> (gtk::Widget, gtk::Label) {
    let shown = test.window.folder_pane().view_widget();
    let title = descendants::<gtk::Label>(&shown)
        .into_iter()
        .find(|label| label.is_mapped() && label.has_css_class("group-title"))
        .expect("a heading's title");
    let drawn_on = |widget: &gtk::Widget| {
        if view == FolderView::Details {
            widget.has_css_class("group-header")
        } else {
            widget.has_css_class("heading-row") && widget.has_css_class("row-start")
        }
    };
    let heading = std::iter::successors(title.parent(), WidgetExt::parent)
        .find(drawn_on)
        .expect("the heading's box");
    (heading, title)
}

/// In every view, normal and compact, a heading's box leaves room round
/// its title as an item's box does round its name: at least 2.5 pixels of
/// the box above and below the title's line (Windows 11's 25-pixel box in
/// compact Tiles round a 20-pixel line of 15-pixel text), the title in its
/// middle, and the chevron at least 8 pixels in from the box's left edge.
///
/// parity: VIEW-022, VIEW-067
#[gtk::test]
fn a_heading_s_box_leaves_room_round_its_title() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
    fixture.write("Notes 1.txt");
    for compact in [false, true] {
        let test = grouped_window(&fixture, compact);
        for view in [FolderView::Details, FolderView::Tiles, FolderView::Content] {
            show(&test, view);
            let (heading, title) = heading_box(&test, view);
            let shown = test.window.folder_pane().view_widget();
            let bounds = |widget: &gtk::Widget| widget.compute_bounds(&shown).expect("laid out");
            let outer = bounds(&heading);
            // The title's line of text, which overhangs its allocation.
            let line = title.layout().pixel_size().1;
            let inner = bounds(title.upcast_ref());
            let middle = inner.y() + inner.height() / 2.0 - outer.y();
            #[expect(clippy::cast_precision_loss, reason = "heights are small")]
            let room = (outer.height() - line as f32) / 2.0;
            assert!(
                room >= 2.5 && (middle - outer.height() / 2.0).abs() <= 1.0,
                "{view:?}, compact {compact}: a {}-pixel box round a {line}-pixel title, its middle at {middle}",
                outer.height()
            );
            let chevron = descendants::<gtk::Image>(&heading)
                .into_iter()
                .find(|image| image.has_css_class("group-chevron"))
                .expect("a chevron");
            let from_edge = bounds(chevron.upcast_ref()).x() - outer.x();
            assert!(
                from_edge >= 8.0,
                "{view:?}, compact {compact}: chevron {from_edge} in"
            );
        }
    }
}
