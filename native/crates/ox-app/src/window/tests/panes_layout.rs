// SPDX-License-Identifier: AGPL-3.0-only
//! The file list, details pane and status bar geometry, measured against
//! the current app (see [`super::geometry`]).

use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::geometry::{bounds, laid_out, laid_out_with_details_pane, Bounds};
use super::icons::{pixel_rows, Pixel};
use crate::folder_view::column_widths;
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{
    descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::widget_tree::children;

/// The column titles of the details view, left to right.
fn column_titles(test: &TestWindow) -> Vec<gtk::Widget> {
    let details = test.window.folder_pane().details().column_view();
    let header = children(details)
        .find(|child| child.css_name() == "header")
        .expect("the details view has a header");
    // The Folder path title of a search is there but hidden.
    children(&header).filter(WidgetExt::is_visible).collect()
}

/// The first row of the details view.
fn first_row(test: &TestWindow) -> gtk::Widget {
    let details = test.window.folder_pane().details().column_view();
    let list = children(details)
        .find(|child| child.css_name() == "listview")
        .expect("the details view has a list");
    children(&list)
        .find(|child| child.css_name() == "row")
        .expect("the fixture has rows")
}

/// parity: VIEW-001
#[gtk::test]
fn columns_run_from_14_pixels_in_with_the_web_widths() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let titles: Vec<Bounds> = column_titles(&test)
        .iter()
        .map(|title| bounds(&test, title))
        .collect();
    let widths: Vec<i32> = titles.iter().map(|title| title.width).collect();
    // Size is as wide as its text needs in the list's font.
    let size = i32::try_from(test.window.folder_pane().details().size_minimum()).expect("a width");
    assert_eq!(
        &widths[1..],
        [176, 135, size + 14],
        "Date, Type, and Size with the end padding"
    );
    assert_eq!(titles[0].x, list.x, "Name holds the 14 pixels before the columns");
    assert_eq!(
        titles[3].right(),
        list.right(),
        "Size holds the 14 pixels after them"
    );
    assert!(
        titles.iter().all(|title| title.height == 31),
        "31-pixel titles above a 1-pixel line, as Windows 11's"
    );
}

/// What a title paints in part of it: how wide its ink is in its first
/// and last inked rows.
#[derive(Debug)]
struct Ink {
    top_width: i32,
    bottom_width: i32,
}

/// What `title` paints in its rows above `limit` and its columns from
/// `columns.0` up to `columns.1`, measured against its top-left pixel;
/// `None` where it paints nothing there.
fn ink_in(title: &gtk::Widget, columns: (i32, i32), limit: i32) -> Option<Ink> {
    let renderer = title
        .native()
        .and_then(|native| native.renderer())
        .expect("the title is on a drawn window");
    // Drawn in place by the header, so its pixels are the title's own,
    // unscaled.
    let header = title.parent().expect("a title is in the header");
    let place = title.compute_bounds(&header).expect("the title is laid out");
    let snapshot = gtk::Snapshot::new();
    header.snapshot_child(title, &snapshot);
    let node = snapshot.to_node()?;
    let rows = pixel_rows(&renderer.render_texture(&node, Some(&place)));
    let corner = rows[1][1];
    let differs = |pixel: &Pixel| {
        pixel
            .iter()
            .zip(corner)
            .any(|(channel, background)| channel.abs_diff(background) > 40)
    };
    let limit = usize::try_from(limit).expect("the rows are inside the title");
    let start = usize::try_from(columns.0).expect("the columns are inside the title");
    let end = usize::try_from(columns.1).expect("the columns are inside the title");
    let inked: Vec<(i32, i32)> = rows[..limit]
        .iter()
        .filter_map(|row| {
            let part = &row[start..end];
            let left = start + part.iter().position(differs)?;
            let right = start + part.iter().rposition(differs)?;
            let column = |index: usize| i32::try_from(index).expect("a narrow title");
            Some((column(left), column(right)))
        })
        .collect();
    let (first, last) = (inked.first()?, inked.last()?);
    Some(Ink {
        top_width: first.1 - first.0,
        bottom_width: last.1 - last.0,
    })
}

/// Every title is left-aligned, Size's too, as in Windows 11 File
/// Explorer, while the sizes stay right-aligned under it. Only the
/// sorted title shows an arrow: a 12-pixel chevron 6 pixels after its
/// label, up while ascending.
///
/// parity: VIEW-014
#[gtk::test]
fn titles_are_left_aligned_and_the_sorted_one_has_a_chevron_after_its_label() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let titles = column_titles(&test);
    let label_of = |title: &gtk::Widget| {
        descendants::<gtk::Label>(title)
            .into_iter()
            .find(WidgetExt::is_visible)
            .expect("a title has a label")
    };
    let chevron_of = |title: &gtk::Widget| {
        descendants::<gtk::Widget>(title)
            .into_iter()
            .find(|child| child.css_name() == "sort-indicator")
            .expect("a title has a sort indicator")
    };
    for title in &titles[1..] {
        let label = label_of(title);
        assert_eq!(
            bounds(&test, &label).x,
            bounds(&test, title).x + 12,
            "{} starts after the title's 12-pixel padding",
            label.text()
        );
        assert_eq!(
            bounds(&test, &chevron_of(title)).width,
            0,
            "only the sorted title has an arrow"
        );
    }
    let name = &titles[0];
    let name_place = bounds(&test, name);
    let chevron = bounds(&test, &chevron_of(name));
    assert_eq!(chevron.width, 12, "a 12-pixel chevron: {chevron:?}");
    assert_eq!(
        chevron.x,
        bounds(&test, &label_of(name)).right() + 6,
        "6 pixels after the label"
    );
    let columns = (chevron.x - name_place.x, chevron.right() - name_place.x);
    // Above the line a sort other than the default draws under the title.
    let rows = name_place.height - 3;
    let ink = ink_in(name, columns, rows).expect("Name sorts, so its chevron is drawn");
    assert!(ink.top_width < ink.bottom_width, "up while ascending: {ink:?}");
    test.activate("direction", Some("descending"));
    wait_for_frames(&test.window, 2);
    let ink = ink_in(name, columns, rows).expect("a chevron while descending");
    assert!(ink.top_width > ink.bottom_width, "down while descending: {ink:?}");
}

/// Saves Expandable folders as `on`, as Settings would, and lets the
/// window follow it.
pub(super) fn save_expandable_folders(test: &TestWindow, on: bool) {
    let update = PreferencesUpdate {
        expandable_folders: Some(on),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
    wait_for_frames(&test.window, 3);
}

/// The x of each cell of `row` and of each column title.
fn cell_and_title_x(test: &TestWindow, row: &gtk::Widget) -> (Vec<i32>, Vec<i32>) {
    let cell_x = children(row).map(|cell| bounds(test, &cell).x).collect();
    let title_x = column_titles(test)
        .iter()
        .map(|title| bounds(test, title).x)
        .collect();
    (cell_x, title_x)
}

/// Rows are inset 12 pixels on each side. While folders show arrows
/// (VIEW-035) a row reaches 10 pixels further left, so the arrows sit
/// inside its highlight; its cells stay under the titles either way.
///
/// parity: VIEW-001, VIEW-035, LOOK-014
#[gtk::test]
fn rows_are_inset_12_pixels_and_their_cells_sit_under_the_titles() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let row = first_row(&test);
    let row_place = bounds(&test, &row);
    assert_eq!(
        (row_place.x, row_place.width, row_place.height),
        (list.x + 12, list.width - 24, 30),
        "folders show no arrows by default, as in Explorer"
    );
    let (cell_x, title_x) = cell_and_title_x(&test, &row);
    assert_eq!(cell_x, title_x);

    save_expandable_folders(&test, true);
    let row = first_row(&test);
    let row_place = bounds(&test, &row);
    assert_eq!(
        (row_place.x, row_place.width, row_place.height),
        (list.x + 2, list.width - 14, 30),
        "the fixture has folders, so rows make room for their arrows"
    );
    let (cell_x, title_x) = cell_and_title_x(&test, &row);
    assert_eq!(cell_x, title_x);
}

/// A folder's expand arrow sits inside its row, so a selected row's
/// highlight goes around it rather than through it.
///
/// parity: VIEW-035
#[gtk::test]
fn a_folders_arrow_sits_inside_its_rows_highlight() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    save_expandable_folders(&test, true);
    let view = test.window.folder_pane().details().column_view();
    let arrows: Vec<gtk::Widget> = descendants::<gtk::Widget>(view)
        .into_iter()
        .filter(|widget| widget.has_css_class("folder-expander") && widget.is_mapped())
        .collect();
    assert!(!arrows.is_empty(), "the fixture's folders show arrows");
    for arrow in arrows {
        let row = std::iter::successors(arrow.parent(), gtk::Widget::parent)
            .find(|widget| widget.css_name() == "row")
            .expect("an arrow is in a row");
        let (arrow_place, row_place) = (bounds(&test, &arrow), bounds(&test, &row));
        assert!(
            arrow_place.x >= row_place.x + 4,
            "the arrow at {} starts 4 pixels inside its row at {}",
            arrow_place.x,
            row_place.x
        );
    }
}

/// The pane stacks Windows 11's parts: the preview at the top, with no
/// heading or close button above it, then the small icon and name, the Details rows and the
/// Properties button, each in the pane's width.
///
/// parity: PROP-009
#[gtk::test]
fn the_details_pane_stacks_its_parts_as_windows_does() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let pane = test.window.details_pane();
    test.select_named("Notes 2.txt");
    wait_for_frames(&test.window, 2);
    let pane_place = bounds(&test, pane);
    assert_eq!(pane_place.width, 262);
    let frames = descendants::<gtk::CenterBox>(pane);
    let preview = frames.first().expect("the pane has a preview");
    let preview_place = bounds(&test, preview);
    assert_eq!(
        (preview_place.x, preview_place.width, preview_place.height),
        (pane_place.x + 23, 217, 148)
    );
    assert_eq!(
        preview_place.y,
        pane_place.y + 22,
        "the pane's padding, with no close button above"
    );
    let title = descendants::<gtk::Box>(pane)
        .into_iter()
        .find(|row| row.has_css_class("detail-title"))
        .expect("the icon and name");
    let title_place = bounds(&test, &title);
    assert_eq!(title_place.y, preview_place.bottom() + 14);
    assert_eq!(title_place.x, preview_place.x);
    let properties = descendants::<gtk::Button>(pane)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.properties"))
        .expect("the Properties button");
    let grid = descendants::<gtk::Grid>(pane).remove(0);
    assert!(bounds(&test, &properties).y > bounds(&test, &grid).bottom());
}

/// Several selected items show their pictures stacked, as Windows 11's
/// Details pane does, and one item its own picture again.
///
/// parity: PROP-009
#[gtk::test]
fn several_selected_items_are_pictured_stacked() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let pane = test.window.details_pane();
    let stack = || {
        descendants::<gtk::Overlay>(pane)
            .into_iter()
            .find(|overlay| overlay.has_css_class("detail-stack"))
    };
    test.window.folder_model().select_positions(&[0, 1, 2]);
    wait_until("the stacked pictures", || stack().is_some());
    let pictures = descendants::<crate::icons::ArtImage>(&stack().expect("the stack"));
    assert_eq!(pictures.len(), 3);
    assert_eq!(pane.shown_name(), "3 items selected");
    let heading = descendants::<gtk::Label>(pane)
        .into_iter()
        .find(|label| label.has_css_class("detail-section"))
        .expect("the Details heading");
    let has_rows = descendants::<gtk::Grid>(pane).remove(0).first_child().is_some();
    assert_eq!(heading.is_visible(), has_rows, "no heading over no rows");
    test.select_named("Notes 2.txt");
    wait_until("one picture again", || stack().is_none());
    assert!(heading.is_visible(), "the heading over the file's rows");
}

/// Every value of the Details rows starts at one line, right of the
/// widest name, and the names start at the pane's edge.
///
/// parity: PROP-009
#[gtk::test]
fn the_details_values_line_up() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    test.select_named("Notes 2.txt");
    wait_for_frames(&test.window, 2);
    let pane = test.window.details_pane();
    let grid = descendants::<gtk::Grid>(pane).remove(0);
    let labels = descendants::<gtk::Label>(&grid);
    let (names, values): (Vec<_>, Vec<_>) =
        labels.iter().partition(|label| label.has_css_class("detail-key"));
    assert!(names.len() >= 4, "Type, Size, File location, Date modified");
    let name_x: Vec<i32> = names.iter().map(|label| bounds(&test, *label).x).collect();
    let value_x: Vec<i32> = values.iter().map(|label| bounds(&test, *label).x).collect();
    assert!(name_x.iter().all(|x| *x == name_x[0]), "names at {name_x:?}");
    assert!(value_x.iter().all(|x| *x == value_x[0]), "values at {value_x:?}");
    let widest_name = names
        .iter()
        .map(|label| bounds(&test, *label).right())
        .max()
        .unwrap_or_default();
    assert!(value_x[0] > widest_name, "values right of the names");
    let preview = bounds(
        &test,
        descendants::<gtk::CenterBox>(pane).first().expect("the preview"),
    );
    assert_eq!(name_x[0], preview.x);
    for value in &values {
        assert!(bounds(&test, *value).right() <= preview.right() + 1);
    }
}

/// At 200% text the details pane is twice as wide and its names column
/// as wide as its widest name, which never wraps, so a value such as
/// "Text Document" reads on one line instead of a few letters per line.
///
/// parity: ACC-012, PROP-009
#[gtk::test]
fn large_text_widens_the_details_pane_and_keeps_names_and_values_whole() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let skin = crate::test_support::harness::skin();
    let before = skin.text_size();
    test.window.set_default_size(1366, 768);
    wait_for_frames(&test.window, 3);
    let pane = test.window.details_pane();
    let normal = pane.width_request();

    skin.set_text_size(crate::text_size::TextSize::from_percent(200));
    wait_for_frames(&test.window, 3);
    test.select_named("Notes 2.txt");
    wait_for_frames(&test.window, 3);
    let large = pane.width_request();
    let grid = descendants::<gtk::Grid>(pane).remove(0);
    let labels = descendants::<gtk::Label>(&grid);
    let lines = |label: &gtk::Label| label.layout().line_count();
    let names: Vec<(String, i32)> = labels
        .iter()
        .filter(|label| label.has_css_class("detail-key"))
        .map(|label| (label.label().to_string(), lines(label)))
        .collect();
    let type_value = labels
        .iter()
        .find(|label| label.has_css_class("detail-value") && label.label().contains("Document"))
        .map(|label| (label.label().to_string(), lines(label)));
    skin.set_text_size(before);
    wait_for_frames(&test.window, 3);
    let back = pane.width_request();

    assert_eq!((normal, large, back), (262, 524, 262), "the pane's widths");
    assert!(
        names.iter().all(|(_, count)| *count == 1),
        "names on one line: {names:?}"
    );
    let (text, count) = type_value.expect("the Type value");
    assert_eq!(count, 1, "{text:?} on one line");
}

/// The view buttons at the status bar's right end, as Windows 11 draws
/// them (reference ref-01, dark, measured): 24 pixels square, side by
/// side with no gap, 4 pixels from the window's right edge, flat and
/// borderless but for the current view's, which sits on a square gray
/// box (#4d4d4d inside a 1-pixel #636363 edge) with its glyph in the
/// text color, not the accent.
///
/// parity: VIEW-006
#[gtk::test]
fn the_status_bar_view_buttons_are_flat_side_by_side_with_the_view_on_a_gray_square() {
    let _theme = crate::test_support::harness::ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("theme", Some("dark"));
    wait_for(super::icons::TRANSITION_TIME);
    wait_for_frames(&test.window, 2);
    // "Check for updates" shows only while an update needs the user.
    let status_buttons: Vec<gtk::Button> = descendants::<gtk::Button>(test.window.status_bar())
        .into_iter()
        .filter(gtk::Button::is_visible)
        .collect();
    let placed: Vec<Bounds> = status_buttons
        .iter()
        .map(|button| bounds(&test, button))
        .collect();
    assert!(
        placed
            .iter()
            .all(|button| (button.width, button.height) == (24, 24)),
        "{placed:?}"
    );
    for pair in placed.windows(2) {
        assert_eq!(pair[1].x - pair[0].right(), 0, "side by side: {placed:?}");
    }
    let last = placed.last().expect("the view buttons");
    assert_eq!(
        bounds(&test, test.window.status_bar()).right() - last.right(),
        4,
        "4 pixels from the edge"
    );
    assert_eq!(test.window.status_bar().active_view_buttons(), ["Details view"]);
    let colour = super::icons::painted_colour;
    let same = super::icons::assert_same_colour;
    let css = super::icons::css_colour;
    let (details, icons) = (&status_buttons[0], &status_buttons[1]);
    same(colour(details, 0, 0), css("#636363"), "the gray square's edge");
    same(colour(details, 2, 2), css("#4d4d4d"), "the gray square");
    // GTK crops a drawn widget to what it paints: the flat button paints
    // only its 16-pixel glyph.
    let flat = super::icons::painted(icons);
    assert!(
        flat.width() <= 16 && flat.height() <= 16,
        "the other button is flat"
    );
    let glyph = descendants::<gtk::Image>(details)
        .into_iter()
        .next()
        .expect("a glyph");
    same(
        super::icons::painted_ink(&glyph),
        css("#f1f1f1"),
        "the glyph in the text color",
    );
    test.activate("view", Some("large"));
    wait_for_frames(&test.window, 2);
    assert_eq!(test.window.status_bar().active_view_buttons(), ["Large icons"]);
}

#[gtk::test]
fn a_selected_tile_keeps_the_text_colour() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    test.activate("theme", Some("light"));
    test.activate("view", Some("large"));
    test.window.folder_model().select_only(1);
    wait_for_frames(&test.window, 3);
    let grid = test.window.folder_pane().icon_view().grid();
    let labels = descendants::<gtk::Label>(grid);
    let selected = labels.iter().find(|label| label.text() == "Notes 2.txt");
    let other = labels.iter().find(|label| label.text() == "Notes 10.txt");
    let (Some(selected), Some(other)) = (selected, other) else {
        panic!("the grid shows both names");
    };
    assert_eq!(
        selected.color().to_str(),
        other.color().to_str(),
        "not GTK's white selected text on the pale selection"
    );
}

/// A larger text size widens Size so a 9.5 GB size still shows whole,
/// at 1x and at 2x alike, since the list measures in its own pixels.
///
/// parity: VIEW-028
#[gtk::test]
fn a_larger_text_size_widens_size_to_its_text() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let details = test.window.folder_pane().details();
    let size = details.column(SortColumn::Size).expect("a Size column");
    let before = details.size_minimum();
    for _ in 0..3 {
        test.activate("text-larger", None);
    }
    crate::test_support::harness::wait_until("Size to follow the larger text", || {
        details.size_minimum() > before
            && size.fixed_width()
                >= column_widths::fixed_width(SortColumn::Size, Some(details.size_minimum()))
    });
    // The text size belongs to the whole app: the next test starts at 100%.
    test.activate("text-reset", None);
}
