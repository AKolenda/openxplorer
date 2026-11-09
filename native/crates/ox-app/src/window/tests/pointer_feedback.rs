// SPDX-License-Identifier: AGPL-3.0-only
//! What a real pointer on check.py's private display sees: the label on
//! the drag icon that says what a release would do (DND-028), a click
//! anywhere on a details row selecting it, and the rubber band drawn from
//! the empty part of a details row (SEL-002, SEL-012). GTK's own
//! drag source, drop targets and gestures run, so these catch what calling
//! the handlers directly cannot, such as which gesture takes a press on a
//! row.

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::right_drag::{blank_on_screen, item_on_screen, HOLD, PROBE};
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::file_drag::{shown_drag_glyph, shown_drag_label};

/// A folder whose rows fill the details view.
const FULL_FOLDER: usize = 120;

/// A real drag of a file over a folder of its drive says "Move to" that
/// folder, "Copy to" it while Ctrl is held, and nothing over blank space of
/// its own folder, where a release does nothing, as in Windows 11.
///
/// parity: DND-028
#[gtk::test]
fn a_real_drag_says_what_a_release_would_do_and_follows_the_keys() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let onto = item_on_screen(&mut pointer, &test, "Documents");
    let blank = blank_on_screen(&mut pointer, &test);

    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    wait_until("the move label", || {
        shown_drag_label().as_deref() == Some("Move to Documents")
    });
    let move_glyph = shown_drag_glyph();
    pointer.hold_key(gdk::Key::Control_L);
    pointer.glide(onto, (onto.0 + 4, onto.1), 2);
    wait_until("the copy label", || {
        shown_drag_label().as_deref() == Some("Copy to Documents")
    });
    let copy_glyph = shown_drag_glyph();
    pointer.let_go_key(gdk::Key::Control_L);
    pointer.glide((onto.0 + 4, onto.1), blank, 12);
    wait_for(HOLD);
    let over_own_folder = shown_drag_label();
    pointer.release(Button::Primary);
    wait_until("the drag to end", || {
        test.window.imp().outgoing_drag.borrow().is_none()
    });

    assert_eq!(move_glyph.as_deref(), Some("ox-arrow-right-20-symbolic"));
    assert_eq!(copy_glyph.as_deref(), Some("ox-add-20-symbolic"));
    assert_eq!(over_own_folder, None, "a release in its own folder does nothing");
    assert!(fixture.path("Notes 2.txt").is_file(), "nothing moved");
}

/// Points on the details row at `position`, in `view`'s coordinates, each
/// with what it is on: the name, the empty end of the Name column past
/// the name, and the middle of each other shown column, by its title.
pub(super) fn points_on_row(
    test: &TestWindow,
    view: &gtk::Widget,
    position: u32,
) -> Vec<(String, (f64, f64))> {
    let cell = test
        .window
        .folder_pane()
        .owners()
        .file_cell_at(position, view)
        .expect("the row is shown");
    let bounds = cell.compute_bounds(view).expect("the row is laid out");
    let y = f64::from(bounds.y() + bounds.height() / 2.0);
    let mut points = vec![("the name".to_owned(), (f64::from(bounds.x()) + 30.0, y))];
    let row = cell
        .ancestor(gtk::ListView::static_type())
        .and_then(|list| {
            std::iter::successors(Some(cell.clone().upcast::<gtk::Widget>()), WidgetExt::parent)
                .find(|widget| widget.parent().as_ref() == Some(&list))
        })
        .expect("the cell is in a row");
    let column_view = view
        .downcast_ref::<gtk::ColumnView>()
        .expect("the details view is a column view");
    // A column view row holds one cell for each shown column, in order.
    let columns = column_view.columns();
    let shown = (0..columns.n_items())
        .filter_map(|index| columns.item(index).and_downcast::<gtk::ColumnViewColumn>())
        .filter(gtk::ColumnViewColumn::is_visible);
    let cells = std::iter::successors(row.first_child(), WidgetExt::next_sibling);
    for (column, row_cell) in shown.zip(cells) {
        let title = column.title().map(|title| title.to_string()).unwrap_or_default();
        let cell_bounds = row_cell.compute_bounds(view).expect("the cell is laid out");
        if title == "Name" {
            let x = f64::from(cell_bounds.x() + cell_bounds.width()) - 8.0;
            points.push(("the empty end of the Name cell".to_owned(), (x, y)));
        } else {
            let x = f64::from(cell_bounds.x() + cell_bounds.width() / 2.0);
            points.push((title, (x, y)));
        }
    }
    points
}

/// In a folder whose rows fill the details view, a real click anywhere on
/// an unselected row (its name, the empty end of its Name cell, its Date
/// modified, Type and Size columns) selects that row alone and moves the
/// keyboard into the list, and draws no rubber band, as Windows 11
/// Explorer's full-row select does in Details.
///
/// parity: SEL-002, SEL-012
#[gtk::test]
fn a_real_click_anywhere_on_a_details_row_selects_it() {
    let fixture = Fixture::with_files(FULL_FOLDER);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let model = test.window.folder_model();
    let view = test.window.folder_pane().input_widget();
    let points = points_on_row(&test, &view, 3);
    let titles: Vec<&str> = points.iter().map(|(title, _)| title.as_str()).collect();
    for column in ["Date modified", "Type", "Size"] {
        assert!(titles.contains(&column), "{column} is shown: {titles:?}");
    }

    for (place, point) in &points {
        model.select_only(0);
        test.activate("location", None);
        wait_until("the keyboard to leave the list", || {
            !test.window.folder_pane().view_has_focus()
        });
        let (x, y) = screen_point(&mut pointer, &view, *point, PROBE);
        pointer.move_to(x, y);
        pointer.press(Button::Primary);
        pointer.release(Button::Primary);
        wait_until(&format!("a click on {place} to select its row"), || {
            model.selected_positions() == [3]
        });
        wait_for(HOLD);
        assert_eq!(
            model.selected_positions(),
            [3],
            "a click on {place} selects its row alone"
        );
        assert!(
            test.window.folder_pane().view_has_focus(),
            "a click on {place} moves the keyboard into the list"
        );
        assert!(
            !test.window.folder_pane().rubber_band_shown(),
            "a click on {place} draws no band"
        );
    }
}

/// The screen point of the Size column of the row at `position`, past the
/// end of its name.
fn beside_name_on_screen(pointer: &mut Pointer, test: &TestWindow, position: u32) -> (i32, i32) {
    let view = test.window.folder_pane().input_widget();
    let cell = test
        .window
        .folder_pane()
        .owners()
        .file_cell_at(position, &view)
        .expect("the row is shown");
    let bounds = cell.compute_bounds(&view).expect("the row is laid out");
    let point = (
        f64::from(view.width()) - 40.0,
        f64::from(bounds.y() + bounds.height() / 2.0),
    );
    screen_point(pointer, &view, point, PROBE)
}

/// In a folder whose rows fill the details view, a real drag from the
/// empty part of an unselected row draws a rubber band over the rows it
/// crosses instead of dragging the row, as in Windows Explorer, where only
/// the icon and name drag the item.
///
/// parity: SEL-012
#[gtk::test]
fn a_real_drag_from_beside_a_details_name_draws_a_band() {
    let fixture = Fixture::with_files(FULL_FOLDER);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let model = test.window.folder_model();
    model.select_only(0);

    let from = beside_name_on_screen(&mut pointer, &test, 2);
    let to = beside_name_on_screen(&mut pointer, &test, 5);
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 - 30, to.1), 12);
    wait_until("the band to select the rows", || {
        model.selected_positions() == [2, 3, 4, 5]
    });
    let band_shown = test.window.folder_pane().rubber_band_shown();
    let dragged = test.window.imp().outgoing_drag.borrow().is_some();
    pointer.release(Button::Primary);
    wait_until("the band to end", || {
        !test.window.folder_pane().rubber_band_shown()
    });
    wait_for(HOLD);

    assert!(band_shown, "the band is drawn");
    assert!(!dragged, "no row is dragged");
    assert_eq!(
        model.selected_positions(),
        [2, 3, 4, 5],
        "the release leaves the band's rows selected"
    );
}
