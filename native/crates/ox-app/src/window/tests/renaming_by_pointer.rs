// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming in place with check.py's display's real pointer and keyboard,
//! as in Windows 11 File Explorer: a click away from the name field, on
//! blank space or on another item, keeps the typed name (OPS-010), and a
//! slow second click on a tile's name starts a rename in the icon views
//! as it does in Details (OPS-011).

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{is_renaming_in_place, name_editor, select_names};
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};

/// The views a rename in place is tried in: a list and a grid of tiles.
const VIEWS: [&str; 3] = ["details", "large", "medium"];

/// Whether the keyboard is in `field`.
fn has_keyboard(test: &TestWindow, field: &gtk::Entry) -> bool {
    GtkWindowExt::focus(&test.window).is_some_and(|focus| focus.is_ancestor(field) || focus == *field)
}

/// Clicks `point`, a point on the screen, with the real pointer, first
/// as a hand presses and lets go of a mouse button, then, with `quick`,
/// as a touchpad's tap does, the release right after the press.
fn click_at(pointer: &mut Pointer, (x, y): (i32, i32), quick: bool) {
    pointer.move_to(x, y);
    if quick {
        pointer.tap(Button::Primary);
    } else {
        pointer.press(Button::Primary);
        pointer.release(Button::Primary);
    }
}

/// The screen point `(x, y)` of `widget`, in its own coordinates.
fn point_in(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>, x: f64, y: f64) -> (i32, i32) {
    screen_point(pointer, widget, (x, y), (200, 200))
}

/// The drawn cell of the item called `name`.
fn cell_of(test: &TestWindow, name: &str) -> FileCell {
    let find = || {
        descendants::<FileCell>(&test.window.folder_pane().view_widget())
            .into_iter()
            .find(|cell| cell.is_drawable() && cell.width() > 0 && cell.name() == name)
    };
    wait_until(&format!("the view to show {name}"), || find().is_some());
    find().expect("wait_until returned only once it was drawn")
}

/// The label that shows the name in `cell`.
fn name_label(cell: &FileCell, name: &str) -> gtk::Label {
    descendants::<gtk::Label>(cell)
        .into_iter()
        .find(|label| label.is_drawable() && label.text() == name)
        .expect("the cell shows its name")
}

/// The middle of the text `label` draws, in its own coordinates: a tile
/// centers its name, a row's name starts at the left of a wider label.
fn text_middle(label: &gtk::Label) -> (f64, f64) {
    let (left, top) = label.layout_offsets();
    let (_, lines) = label.layout().pixel_extents();
    (
        f64::from(left + lines.x()) + f64::from(lines.width()) / 2.0,
        f64::from(top + lines.y()) + f64::from(lines.height()) / 2.0,
    )
}

/// Starts renaming "Notes 2.txt" with F2 on the real keyboard in `view`
/// and types "plan" over the selected name, with extensions shown.
fn type_new_name(test: &TestWindow, pointer: &mut Pointer, view: &str) -> gtk::Entry {
    let shown = view.split(',').next().unwrap_or(view);
    test.activate("view", Some(shown));
    select_names(test, &["Notes 2.txt"]);
    test.window.folder_pane().focus_view();
    wait_for_frames(&test.window, 3);
    let inside = point_in(pointer, &test.window, 40.0, 40.0);
    pointer.move_to(inside.0, inside.1);
    pointer.tap_key(gdk::Key::F2);
    let field = name_editor(test);
    wait_until(&format!("{view}: the field takes the keyboard"), || {
        has_keyboard(test, &field) && field.selection_bounds() == Some((0, 7))
    });
    for key in [gdk::Key::p, gdk::Key::l, gdk::Key::a, gdk::Key::n] {
        pointer.tap_key(key);
    }
    wait_until(&format!("{view}: the typed name"), || field.text() == "plan.txt");
    field
}

/// A click on blank space in the view while a name is edited keeps the
/// typed name, as in Windows 11, where clicking away from the name box
/// renames the item: the list rebinding its rows for the cleared
/// selection used to close the field and lose the name. Nothing is
/// selected afterwards, as the click on blank space asked.
///
/// parity: OPS-010
#[gtk::test]
fn a_click_on_blank_space_keeps_the_typed_name() {
    let mut pointer = Pointer::connect();
    for (view, quick) in VIEWS.into_iter().flat_map(|view| [(view, false), (view, true)]) {
        let view = &format!("{view}, quick: {quick}");
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        super::support::show_file_name_extensions(&test);
        type_new_name(&test, &mut pointer, view);
        let list = test.window.folder_pane().view_widget();
        let blank = point_in(
            &mut pointer,
            &list,
            f64::from(list.width()) - 20.0,
            f64::from(list.height()) - 20.0,
        );

        click_at(&mut pointer, blank, quick);

        wait_until(&format!("{view}: the rename"), || {
            fixture.path("plan.txt").is_file()
        });
        wait_until(&format!("{view}: the field to close"), || {
            !is_renaming_in_place(&test)
        });
        assert!(
            !fixture.path("Notes 2.txt").exists(),
            "{view}: the old name is gone"
        );
        wait_for(Duration::from_millis(300));
        assert!(
            test.selected_names().is_empty(),
            "{view}: the click on blank space selected nothing"
        );
    }
}

/// A click on another item while a name is edited keeps the typed name
/// and leaves the clicked item selected, as in Windows 11, so the next
/// command acts on the item the user clicked, not on the renamed one.
///
/// parity: OPS-010
#[gtk::test]
fn a_click_on_another_item_keeps_the_typed_name_and_selects_that_item() {
    let mut pointer = Pointer::connect();
    for (view, quick) in VIEWS.into_iter().flat_map(|view| [(view, false), (view, true)]) {
        let view = &format!("{view}, quick: {quick}");
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        super::support::show_file_name_extensions(&test);
        type_new_name(&test, &mut pointer, view);
        let other = cell_of(&test, "Résumé.txt");
        let label = name_label(&other, "Résumé.txt");
        let (x, y) = text_middle(&label);
        let on_other = point_in(&mut pointer, &label, x, y);

        click_at(&mut pointer, on_other, quick);

        wait_until(&format!("{view}: the rename"), || {
            fixture.path("plan.txt").is_file()
        });
        wait_until(&format!("{view}: the field to close"), || {
            !is_renaming_in_place(&test)
        });
        test.wait_for_listing("the listing again");
        wait_for(Duration::from_millis(300));
        assert_eq!(
            test.selected_names(),
            ["Résumé.txt"],
            "{view}: the clicked item stays selected"
        );
    }
}

/// A slow second click on the name of the only selected item renames it
/// in place in the icon views too, whose names are centered under the
/// icon: the click used to be measured against a name starting at the
/// tile's left edge, so a click on the middle of the text missed it.
///
/// parity: OPS-011
#[gtk::test]
fn a_slow_second_click_on_a_tiles_name_renames_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    let mut pointer = Pointer::connect();
    let interval = gtk::Settings::default()
        .map_or(400, |settings| settings.gtk_double_click_time())
        .unsigned_abs();
    for view in ["details", "large", "medium"] {
        test.activate("view", Some(view));
        select_names(&test, &["Notes 2.txt"]);
        let cell = cell_of(&test, "Notes 2.txt");
        let label = name_label(&cell, "Notes 2.txt");
        wait_for_frames(&test.window, 3);
        let (x, y) = text_middle(&label);
        let middle = point_in(&mut pointer, &label, x, y);

        click_at(&mut pointer, middle, false);
        wait_until(&format!("{view}: the rename in place"), || {
            is_renaming_in_place(&test)
        });

        let field = name_editor(&test);
        assert_eq!(field.text(), "Notes 2.txt", "{view}: the name to edit");
        pointer.tap_key(gdk::Key::Escape);
        wait_until(&format!("{view}: Escape ends it"), || {
            !is_renaming_in_place(&test)
        });
        // The next view's first click is not the second of a double-click.
        wait_for(Duration::from_millis(u64::from(interval) * 2));
    }
}

/// F2 on the selected item scrolled out of the view's sight scrolls it
/// back and renames it in place where the user sees it. GTK keeps rows
/// just outside the visible area alive, and the name field used to open
/// in such a row, out of sight, taking what the user typed. F2 is the
/// real key.
///
/// parity: OPS-010
#[gtk::test]
fn f2_on_an_item_scrolled_out_of_sight_scrolls_it_back_to_rename() {
    let fixture = Fixture::with_files(80);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["file 0000.txt"]);
    let view = test.window.folder_pane().view_widget();
    let scrolled = view
        .clone()
        .dynamic_cast::<gtk::Scrollable>()
        .expect("the view scrolls")
        .vadjustment()
        .expect("the view has a vertical adjustment");
    // Just far enough that the first row is out of sight while GTK keeps it.
    scrolled.set_value(200.0);
    wait_for_frames(&test.window, 5);
    test.window.folder_pane().focus_view();
    let inside = point_in(&mut pointer, &test.window, 40.0, 40.0);
    pointer.move_to(inside.0, inside.1);

    pointer.tap_key(gdk::Key::F2);
    let field = name_editor(&test);
    wait_for_frames(&test.window, 5);

    assert_eq!(field.text(), "file 0000.txt");
    let bounds = field.compute_bounds(&view).expect("the field is in the view");
    assert!(
        f64::from(bounds.y()) >= 0.0 && f64::from(bounds.y() + bounds.height()) <= f64::from(view.height()),
        "the field is in sight: {bounds:?} in a view {} high",
        view.height()
    );
    assert!(super::file_ops_support::dialog_over(&test).is_none(), "no dialog");
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(&test));
}
