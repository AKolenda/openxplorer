// SPDX-License-Identifier: AGPL-3.0-only
//! Compact view spaces the items as Windows 11's Compact view does, and the
//! usual view keeps its spacing (VIEW-067).
//!
//! The Windows numbers were measured at 100% scaling, in pixels from one
//! line of text to the next: Details rows 21 apart, a group heading 26
//! below the last row before it and 27 above the first row under it, the
//! column header 26 tall with its line, and the navigation pane's rows 24
//! apart, a separator taking one row's room. A heading over one row of
//! Tiles stands 83 from the next heading, which makes rows of tiles 57
//! apart; Content's items are 48 apart, their lines 18 apart, a heading 31
//! to 32 above its first item and 48 to 49 below the last item before it.
//! The distances are logical pixels, so they hold at every display scale
//! (`GDK_SCALE=2` doubles the device pixels alike). Text-to-text distances
//! are measured between the middles of labels, which for labels of one
//! font size are as far apart as their lines of text whatever the labels'
//! line heights. A heading's title is a quarter larger than the items'
//! names (VIEW-022), so its capitals' tops stand a pixel further above its
//! middle than a name's do: a heading 26 below a row and 27 above the
//! next by the capitals, as Windows was measured, is 27 and 26 by the
//! middles.

use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;

/// A folder, a file of a made-up type and twelve text files: grouped by
/// type, the folder alone in File folder, the other file alone in LN1
/// File, then
/// rows of tiles in Text Document.
fn fixture() -> Fixture {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
    std::fs::write(fixture.path("Agenda.ln1"), [0_u8, 1, 2, 3]).expect("fixture file");
    for number in 1..=12 {
        fixture.write(&format!("Notes {number}.txt"));
    }
    fixture
}

/// A window on [`fixture`] grouped by type, with Compact view on or off.
fn grouped_window(fixture: &Fixture, compact: bool) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    if test.window.shows_compact_density() != compact {
        test.activate("compact-density", None);
    }
    test.activate("group-by", Some("type"));
    wait_for_frames(&test.window, 4);
    test
}

/// The middle of `label`, down the visible view.
fn middle(test: &TestWindow, label: &gtk::Label) -> f32 {
    let view = test.window.folder_pane().view_widget();
    let bounds = label.compute_bounds(&view).expect("the label is laid out");
    bounds.y() + bounds.height() / 2.0
}

/// The label of the group heading titled `title` in the visible view.
fn heading_label(test: &TestWindow, title: &str) -> gtk::Label {
    let view = test.window.folder_pane().view_widget();
    descendants::<gtk::Label>(&view)
        .into_iter()
        .find(|label| {
            label.is_mapped()
                && label.text() == title
                && label
                    .parent()
                    .is_some_and(|hit| hit.has_css_class("group-heading-hit"))
        })
        .unwrap_or_else(|| {
            let shown: Vec<String> = descendants::<gtk::Label>(&view)
                .into_iter()
                .filter(|label| label.is_mapped() && label.has_css_class("group-title"))
                .map(|label| label.text().to_string())
                .collect();
            panic!("a heading shows {title}: {shown:?}")
        })
}

/// The name label of the item shown as `name` in the visible view.
fn name_label(test: &TestWindow, name: &str) -> gtk::Label {
    let view = test.window.folder_pane().view_widget();
    descendants::<FileCell>(&view)
        .into_iter()
        .filter(|cell| cell.is_mapped() && cell.group_heading().is_none() && cell.name() == name)
        .find_map(|cell| {
            descendants::<gtk::Label>(&cell)
                .into_iter()
                .find(|label| label.is_mapped() && label.text() == name)
        })
        .unwrap_or_else(|| panic!("an item shows {name}"))
}

/// A line of text the test measures from or to.
#[derive(Clone, Copy)]
enum Line<'a> {
    /// A group heading's title.
    Heading(&'a str),
    /// An item's name.
    Name(&'a str),
}

/// How far below the line `from` the line `to` stands, in whole pixels.
fn text_distance(test: &TestWindow, from: Line<'_>, to: Line<'_>) -> i32 {
    let at = |line: Line<'_>| match line {
        Line::Heading(title) => middle(test, &heading_label(test, title)),
        Line::Name(name) => middle(test, &name_label(test, name)),
    };
    super::geometry::pixels(at(to) - at(from))
}

/// How far apart the rows of tiles are: from the top of the first item's
/// cell (what draws its hover and selection) to the top of the first cell
/// in the next row.
fn tile_row_pitch(test: &TestWindow) -> i32 {
    let view = test.window.folder_pane().view_widget();
    let mut tops: Vec<i32> = descendants::<FileCell>(&view)
        .into_iter()
        .filter(|cell| cell.is_mapped() && cell.group_heading().is_none() && cell.name().starts_with("Notes"))
        .filter_map(|cell| cell.compute_bounds(&view))
        .map(|bounds| super::geometry::pixels(bounds.y()))
        .collect();
    tops.sort_unstable();
    tops.dedup();
    assert!(tops.len() > 1, "two rows of tiles: {tops:?}");
    tops[1] - tops[0]
}

/// The height of the column header, its line under the titles included,
/// as the line was counted in the measurements: how far down the column
/// view its list starts.
fn column_header_height(test: &TestWindow) -> i32 {
    let column_view = test.window.folder_pane().details().column_view().clone();
    let list = descendants::<gtk::Widget>(&column_view)
        .into_iter()
        .find(|widget| widget.css_name() == "listview")
        .expect("the column view's list");
    let bounds = list.compute_bounds(&column_view).expect("the list is laid out");
    super::geometry::pixels(bounds.y())
}

/// How far below the middle of the item `name`'s name the middle of its
/// next line (its type) stands.
fn second_line_distance(test: &TestWindow, name: &str, second: &str) -> i32 {
    let view = test.window.folder_pane().view_widget();
    let cell = descendants::<FileCell>(&view)
        .into_iter()
        .find(|cell| cell.is_mapped() && cell.group_heading().is_none() && cell.name() == name)
        .unwrap_or_else(|| panic!("an item shows {name}"));
    let line = descendants::<gtk::Label>(&cell)
        .into_iter()
        .find(|label| label.is_mapped() && label.text() == second)
        .unwrap_or_else(|| panic!("{name} shows {second}"));
    super::geometry::pixels(middle(test, &line) - middle(test, &name_label(test, name)))
}

/// The places' rows of the navigation pane top to bottom, each with its
/// top and whether a separator stands above it.
fn navigation_rows(test: &TestWindow) -> Vec<(i32, bool)> {
    let list = test.window.sidebar().list().clone();
    let mut rows: Vec<(i32, bool)> = descendants::<gtk::ListBoxRow>(&list)
        .into_iter()
        .filter(|row| row.is_mapped() && row.height() > 0)
        .filter_map(|row| {
            let top = row
                .compute_point(&list, &gtk::graphene::Point::new(0.0, 0.0))?
                .y();
            let separated = row.header().is_some_and(|header| header.is_visible());
            Some((super::geometry::pixels(top), separated))
        })
        .collect();
    rows.sort_unstable();
    rows
}

/// The distances from each navigation row to the next: (within a group,
/// across a separator).
fn navigation_pitches(test: &TestWindow) -> (Vec<i32>, Vec<i32>) {
    let rows = navigation_rows(test);
    let mut within = Vec::new();
    let mut across = Vec::new();
    for pair in rows.windows(2) {
        let pitch = pair[1].0 - pair[0].0;
        if pair[1].1 {
            across.push(pitch);
        } else {
            within.push(pitch);
        }
    }
    within.dedup();
    across.dedup();
    (within, across)
}

/// The Details spacing in pixels: (row to row, last row to the next
/// heading, heading to its first row, column header).
fn details_spacing(test: &TestWindow) -> (i32, i32, i32, i32) {
    wait_until("the headings", || {
        descendants::<gtk::Label>(&test.window.folder_pane().view_widget())
            .iter()
            .any(|label| label.is_mapped() && label.has_css_class("group-title"))
    });
    (
        text_distance(test, Line::Name("Notes 1"), Line::Name("Notes 2")),
        text_distance(test, Line::Name("Agenda.ln1"), Line::Heading("Text Document")),
        text_distance(test, Line::Heading("Text Document"), Line::Name("Notes 1")),
        column_header_height(test),
    )
}

/// The title of the group of the file of a made-up type, named after its
/// extension as no program knows it.
const OTHER_TYPE: &str = "LN1 File";

/// Page Up and Page Down count the rows of the visible grid as tall as
/// they are drawn: a heading's row and an item's row.
fn assert_page_counts_rows(test: &TestWindow, view: FolderView) {
    let pane = test.window.folder_pane();
    let heights = pane.icon_view().row_heights();
    let grid = pane.view_widget();
    let slot_height = |cell: &FileCell| {
        cell.parent()
            .and_then(|slot| slot.compute_bounds(&grid))
            .map(|bounds| super::geometry::pixels(bounds.height()))
            .expect("the slot is laid out")
    };
    let cells = descendants::<FileCell>(&grid);
    let heading = cells
        .iter()
        .find(|cell| cell.is_mapped() && cell.group_heading().is_some())
        .expect("a heading on screen");
    let item = cells
        .iter()
        .find(|cell| cell.is_mapped() && cell.name() == "Notes 1")
        .expect("an item on screen");
    assert_eq!(
        (slot_height(heading), slot_height(item)),
        (heights.heading, heights.item),
        "{view:?}: (heading, item) rows"
    );
}

/// Shows `view` and waits for its items to be laid out.
fn show(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    wait_until("the view to lay out its items", || {
        let pane = test.window.folder_pane();
        let shown = pane.view_widget();
        pane.owners()
            .widget_at(0)
            .is_some_and(|cell| cell.is_ancestor(&shown) && cell.height() > 0)
    });
    wait_for_frames(&test.window, 4);
}

/// Compact view's Details rows are 21 pixels apart, a group heading 26
/// below the row before it and 27 above its first row, the column header
/// 26 tall, and the navigation pane's rows 24 apart with a separator
/// taking one row's room (48 across it), as Windows 11's Compact view.
///
/// parity: VIEW-067
#[gtk::test]
fn compact_view_spaces_details_and_the_navigation_pane_as_windows() {
    let fixture = fixture();
    let test = grouped_window(&fixture, true);
    // By the middles: Windows' 26 and 27 by the capitals' tops.
    assert_eq!(
        details_spacing(&test),
        (21, 27, 26, 26),
        "(row, above heading, below heading, header)"
    );
    assert_eq!(
        navigation_pitches(&test),
        (vec![24], vec![48]),
        "(rows, across a separator)"
    );
}

/// Compact view's Tiles rows are 57 pixels apart (inferred: Windows' tile
/// row with the gap under it, as measured between headings), a heading
/// over one row of tiles 83 from the next heading; Content's items are 48
/// apart, a heading 32 above its first item's name and 49 below the last name before it
/// (Windows: 31 to 32, and 48 to 49), an item's lines 18 apart.
///
/// parity: VIEW-067, VIEW-071
#[gtk::test]
fn compact_view_spaces_tiles_and_content_as_windows() {
    let fixture = fixture();
    let test = grouped_window(&fixture, true);
    show(&test, FolderView::Tiles);
    assert_eq!(tile_row_pitch(&test), 57, "tile rows");
    assert_page_counts_rows(&test, FolderView::Tiles);
    assert_eq!(
        text_distance(&test, Line::Heading("File folder"), Line::Heading(OTHER_TYPE)),
        83,
        "heading over one row"
    );
    show(&test, FolderView::Content);
    assert_eq!(
        text_distance(&test, Line::Name("Notes 1"), Line::Name("Notes 2")),
        48,
        "items"
    );
    // By the middles, a pixel off Windows' 31 to 32 and 48 to 49 by the
    // capitals' tops, and half a pixel either way as the display scale
    // rounds them.
    let below = text_distance(&test, Line::Heading("Text Document"), Line::Name("Notes 1"));
    assert!((30..=31).contains(&below), "heading to first item: {below}");
    let above = text_distance(&test, Line::Name("Agenda.ln1"), Line::Heading("Text Document"));
    assert!((49..=50).contains(&above), "last item to heading: {above}");
    // 17.5 to 18 pixels as the display scale rounds the labels.
    let second = second_line_distance(&test, "Notes 1", "Text Document");
    assert!((17..=18).contains(&second), "a name to its type: {second}");
    assert_page_counts_rows(&test, FolderView::Content);
}

/// The usual view keeps its spacing: Details rows 32 apart, a heading 40
/// below the row before it and 32 above its first row, the column header
/// 32 tall with its line, the navigation pane's rows 32 apart and 56
/// across a separator; Tiles' rows 75 apart and a heading over one row 110 from the
/// next; Content's items 48 apart, their lines 14.5 apart, a heading 29.5
/// above its first item and 53.5 below the last before it.
///
/// parity: VIEW-067
#[gtk::test]
fn the_usual_view_keeps_its_spacing() {
    let fixture = fixture();
    let test = grouped_window(&fixture, false);
    assert!(!test.window.shows_compact_density());
    assert_eq!(
        details_spacing(&test),
        (32, 40, 32, 32),
        "(row, above heading, below heading, header)"
    );
    assert_eq!(
        navigation_pitches(&test),
        (vec![32], vec![56]),
        "(rows, across a separator)"
    );
    show(&test, FolderView::Tiles);
    assert_eq!(tile_row_pitch(&test), 75, "tile rows");
    assert_eq!(
        text_distance(&test, Line::Heading("File folder"), Line::Heading(OTHER_TYPE)),
        110,
        "heading over one row"
    );
    show(&test, FolderView::Content);
    assert_eq!(
        text_distance(&test, Line::Name("Notes 1"), Line::Name("Notes 2")),
        48,
        "items"
    );
    // 29.5 and 53.5 pixels, which the display scale rounds either way.
    let below = text_distance(&test, Line::Heading("Text Document"), Line::Name("Notes 1"));
    assert!((29..=30).contains(&below), "heading to first item: {below}");
    let above = text_distance(&test, Line::Name("Agenda.ln1"), Line::Heading("Text Document"));
    assert!((53..=54).contains(&above), "last item to heading: {above}");
    // 14.5 pixels, which the display scale rounds either way.
    let second = second_line_distance(&test, "Notes 1", "Text Document");
    assert!((14..=15).contains(&second), "a name to its type: {second}");
}

/// In Compact view a removable drive's row, with its eject button, is as
/// tall as every other row of the navigation pane, 23 pixels at 100% and
/// 24 apart, so drives compact too.
///
/// parity: VIEW-067, DEV-007
#[gtk::test]
fn a_drive_with_an_eject_button_compacts_as_the_other_rows() {
    use crate::devices::Removal;
    use crate::icons::{Art, Icon};
    use crate::window::sidebar::entries::{EjectButton, RowLevel, RowTarget, Section, SidebarEntry};

    let fixture = fixture();
    let test = grouped_window(&fixture, true);
    let drive = |label: &str, eject: bool| SidebarEntry {
        section: Section::ThisPc,
        level: RowLevel::Child,
        label: label.into(),
        icon: Art::Glyph(Icon::HardDrive),
        target: RowTarget::MountVolume(label.into()),
        tooltip: label.into(),
        pinned: false,
        menu: None,
        eject: eject.then(|| EjectButton {
            removal: Removal::Eject,
            uri: "file:///media/demo/USB".into(),
            label: "Eject",
        }),
    };
    let sidebar = test.window.sidebar();
    sidebar.set_entries(vec![
        drive("Backup", false),
        drive("USB", true),
        drive("Archive", false),
    ]);
    let row = |index| sidebar.list().row_at_index(index).expect("a drive's row");
    wait_until("the drives' rows", || (0..3).all(|index| row(index).is_mapped()));
    wait_for_frames(&test.window, 3);
    // The least height each row asks for with its 1-pixel margin above:
    // what its parts need, which is what it takes once GTK lays it out
    // again.
    let rows: Vec<i32> = (0..3)
        .map(|index| row(index).measure(gtk::Orientation::Vertical, -1).0)
        .collect();
    assert_eq!(rows, [24, 24, 24], "(plain, with an eject button, plain)");
    assert_eq!(navigation_pitches(&test).0, vec![24], "rows 24 apart");
}
