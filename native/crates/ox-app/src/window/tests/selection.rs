// SPDX-License-Identifier: AGPL-3.0-only
//! Selecting with the mouse and the keyboard in a real window, against
//! `selectEntry`, `onKey` and the blank-space click of
//! `v2.0.0:desktop/ui/app.js`, with Dolphin's and Explorer's keys.
//!
//! GTK has no public way to synthesise pointer or key events. A click on
//! an item runs `list.select-item`, where GTK's own click ends; a
//! press on blank space emits `pressed` on the views' capture-phase click
//! gestures; a key runs the shortcut GTK would find for it, bubbling up
//! from the focused widget.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{is_triggered_by, press_shortcut_where_focused, shortcuts_of};
use crate::test_support::harness::{capture, descendants, wait_until, Fixture, TestWindow};
use crate::window::rubber_band::{BandMode, RELEASE_WATCH};

/// A folder long enough to scroll.
const LONG_FOLDER: usize = 300;

/// A folder long enough to scroll whose last grid row is shorter than the
/// others at any column count: a prime number of items.
const PRIME_FOLDER: usize = 293;

/// Clicks the item at `position` as GTK's click on it ends, with Ctrl
/// (`toggle`) and Shift (`range`): in the view's `list.select-item`.
fn click(test: &TestWindow, position: u32, toggle: bool, range: bool) {
    let cell = test
        .window
        .folder_pane()
        .owners()
        .widget_at(position)
        .expect("the item is on screen");
    cell.activate_action("list.select-item", Some(&(position, toggle, range).to_variant()))
        .expect("an item can be selected");
}

/// The positions of the selected items.
fn selected(test: &TestWindow) -> Vec<u32> {
    test.window.folder_model().selected_positions()
}

/// Presses `keyval` with `modifiers` as GTK's key handling routes it: the
/// window's capture-phase shortcuts, then the view's capture-phase key
/// controller while focus is in the view, then the first shortcut for it
/// from the focused widget up.
fn press_key(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) {
    let runs = |widget: &gtk::Widget, phase| {
        shortcuts_of(widget, phase).into_iter().any(|shortcut| {
            let is_the_key = shortcut
                .trigger()
                .is_some_and(|trigger| is_triggered_by(&trigger, keyval, modifiers));
            let action = shortcut.action().filter(|_| is_the_key);
            action.is_some_and(|action| {
                action.activate(
                    gtk::ShortcutActionFlags::empty(),
                    widget,
                    shortcut.arguments().as_ref(),
                )
            })
        })
    };
    if runs(test.window.upcast_ref(), gtk::PropagationPhase::Capture) {
        return;
    }
    if test.window.folder_pane().view_has_focus() && view_handles_key(test, keyval, modifiers) {
        return;
    }
    let mut widget = gtk::prelude::GtkWindowExt::focus(&test.window);
    while let Some(current) = widget {
        if runs(&current, gtk::PropagationPhase::Bubble) {
            return;
        }
        widget = current.parent();
    }
    panic!("nothing handles {keyval:?} with {modifiers:?}");
}

/// Whether the visible view's capture-phase key controller, the window's
/// own key handling, takes `keyval` with `modifiers`.
fn view_handles_key(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let view = test.window.folder_pane().view_widget();
    let controller = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the view has a capture-phase key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>("key-pressed", &[&keyval.into_glib(), &no_keycode, &modifiers])
}

/// Presses the primary button at (`x`, `y`) in the visible view, as far
/// as the window's own capture-phase gestures go.
fn press_at(test: &TestWindow, x: f64, y: f64) {
    let view = test.window.folder_pane().view_widget();
    let controllers = view.observe_controllers();
    let gestures = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .filter(|gesture| {
            gesture.propagation_phase() == gtk::PropagationPhase::Capture
                && gesture.button() == gdk::BUTTON_PRIMARY
        });
    for gesture in gestures {
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
    }
}

/// A point on blank space in the visible view, below its items.
fn blank_point(test: &TestWindow) -> (f64, f64) {
    let view = test.window.folder_pane().view_widget();
    (20.0, f64::from(view.height()) - 10.0)
}

/// parity: SEL-001
#[gtk::test]
fn click_ctrl_click_and_shift_click_select_like_explorer() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    click(&test, 1, false, false);
    assert_eq!(selected(&test), [1]);
    click(&test, 3, false, true);
    assert_eq!(selected(&test), [1, 2, 3], "Shift+click selects from the anchor");
    click(&test, 2, true, false);
    assert_eq!(selected(&test), [1, 3], "Ctrl+click toggles one item");
    click(&test, 0, false, false);
    assert_eq!(selected(&test), [0], "a plain click selects only the item");
}

/// parity: SEL-002
#[gtk::test]
fn a_click_on_blank_space_clears_the_selection_and_focuses_the_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // Large icons' 96-pixel tiles fill the window down to its bottom edge,
    // so the icon view is tried with Medium icons, which leave space.
    for view in ["details", "medium"] {
        show_view(&test, view);
        test.window.folder_model().select_only(1);
        test.window.search_box().focus();
        let (x, y) = blank_point(&test);
        press_at(&test, x, y);
        assert!(selected(&test).is_empty(), "{view}: the selection is cleared");
        assert!(
            test.window.folder_pane().view_has_focus(),
            "{view}: the list keeps keyboard focus"
        );
    }
}

/// A band drawn from blank space selects the rows it crosses while it
/// moves, and Ctrl toggles them in the selection it started from; a press
/// on an item starts none.
///
/// parity: SEL-012
#[gtk::test]
fn a_rubber_band_selects_the_rows_it_crosses_as_it_moves() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    let center_of = |position: u32| {
        let row = pane.owners().widget_at(position).expect("on screen");
        let bounds = row.compute_bounds(&view).expect("a shown item has bounds");
        f64::from(bounds.y() + bounds.height() / 2.0)
    };
    assert!(
        !test.window.is_blank_space(&view, 30.0, center_of(1)),
        "an item drags, not bands"
    );
    let (x, y) = blank_point(&test);
    assert!(test.window.is_blank_space(&view, x, y));
    let last = pane.model().n_items() - 1;

    test.window.begin_band(&view, (x, y), BandMode::Replace);
    test.window.move_band((x + 40.0, center_of(1)));
    capture(&test.window, "native-rubber-band.png");
    let crossed: Vec<u32> = (1..=last).collect();
    assert_eq!(
        selected(&test),
        crossed,
        "the selection follows the band before it ends"
    );
    assert_eq!(
        pane.rubber_band_size().0,
        40,
        "the band is drawn where it is dragged, as in Explorer, not across the view"
    );
    // The pointer moving with no button held ends a band whose release
    // was lost, and the selection stays as it was drawn.
    let controllers = view.observe_controllers();
    let watch = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerMotion>().ok())
        .find(|controller| controller.name().as_deref() == Some(RELEASE_WATCH))
        .expect("the view watches for a lost release");
    watch.emit_by_name::<()>("motion", &[&(x + 50.0), &center_of(1)]);
    assert!(!pane.rubber_band_shown(), "a lost release leaves no band drawn");
    assert_eq!(selected(&test), crossed);
    // So does the pointer leaving the view with no button held.
    test.window.begin_band(&view, (x, y), BandMode::Replace);
    test.window.move_band((x + 40.0, center_of(1)));
    assert!(pane.rubber_band_shown());
    watch.emit_by_name::<()>("leave", &[]);
    assert!(
        !pane.rubber_band_shown(),
        "leaving after a lost release ends the band"
    );
    test.window.end_band();

    test.window.begin_band(&view, (x, y), BandMode::Toggle);
    test.window.move_band((x, center_of(last)));
    test.window.end_band();
    assert_eq!(
        selected(&test),
        (1..last).collect::<Vec<u32>>(),
        "Ctrl toggles the crossed row"
    );
}

/// The middle of the name cell of the row at `position` in `view`, and a
/// point on the same row past the end of its name, in `view`'s
/// coordinates.
fn name_and_beside_it(test: &TestWindow, view: &gtk::Widget, position: u32) -> ((f64, f64), (f64, f64)) {
    let cell = test
        .window
        .folder_pane()
        .owners()
        .file_cell_at(position, view)
        .expect("the row is on screen");
    let bounds = cell.compute_bounds(view).expect("a shown row has bounds");
    let y = f64::from(bounds.y() + bounds.height() / 2.0);
    let on_name = (f64::from(bounds.x()) + 40.0, y);
    let beside = (f64::from(view.width()) - 40.0, y);
    (on_name, beside)
}

/// In a folder whose rows fill the details view, the empty part of an
/// unselected row is blank space for a rubber band, as in Windows
/// Explorer, but a press there keeps the selection: released without a
/// drag it selects the row (full-row select). Its icon and name, a
/// selected row, and the tiles of the icon views stay the item.
///
/// parity: SEL-002, SEL-012
#[gtk::test]
fn the_empty_part_of_an_unselected_details_row_is_blank_space() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    show_view(&test, "details");
    let view = test.window.folder_pane().view_widget();
    let (on_name, beside) = name_and_beside_it(&test, &view, 3);
    test.window.folder_model().select_only(1);

    let name_is_blank = test.window.is_blank_space(&view, on_name.0, on_name.1);
    let beside_is_blank = test.window.is_blank_space(&view, beside.0, beside.1);
    test.window.folder_model().select_only(3);
    let beside_selected_is_blank = test.window.is_blank_space(&view, beside.0, beside.1);
    test.window.folder_model().select_only(1);
    press_at(&test, beside.0, beside.1);
    let after_press = selected(&test);

    assert!(!name_is_blank, "the name is the item");
    assert!(beside_is_blank, "beside the name is blank space");
    assert!(
        !beside_selected_is_blank,
        "a selected row is the item across its width"
    );
    assert_eq!(
        after_press,
        [1],
        "a press beside a name keeps the selection until it is a click or a band"
    );

    show_view(&test, "medium");
    let grid = test.window.folder_pane().view_widget();
    let tile = test
        .window
        .folder_pane()
        .owners()
        .widget_at(3)
        .expect("the tile is on screen");
    let bounds = tile.compute_bounds(&grid).expect("a shown tile has bounds");
    let tile_edge = (
        f64::from(bounds.x() + bounds.width()) - 2.0,
        f64::from(bounds.y() + bounds.height()) - 2.0,
    );
    assert!(
        !test.window.is_blank_space(&grid, tile_edge.0, tile_edge.1),
        "a tile is the item to its edge"
    );
}

/// parity: SEL-004, SEL-005
#[gtk::test]
fn ctrl_a_and_escape_work_outside_the_view_but_not_in_text_fields() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let control = gdk::ModifierType::CONTROL_MASK;
    test.window.search_box().focus();
    press_shortcut_where_focused(&test, gdk::Key::a, control);
    assert!(selected(&test).is_empty(), "the search box keeps its Ctrl+A");
    let place = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("a sidebar place");
    assert!(place.grab_focus(), "a sidebar place takes focus");
    press_key(&test, gdk::Key::a, control);
    assert_eq!(selected(&test), [0, 1, 2, 3], "Ctrl+A from the sidebar");
    let button = descendants::<gtk::Button>(test.window.status_bar())
        .into_iter()
        .find(|button| button.is_visible() && button.grab_focus())
        .expect("a status bar button takes focus");
    press_key(&test, gdk::Key::Escape, gdk::ModifierType::empty());
    assert!(selected(&test).is_empty(), "Escape on a button clears it");
    press_key(&test, gdk::Key::a, control);
    assert_eq!(selected(&test), [0, 1, 2, 3], "Ctrl+A from a button");
    assert!(button.has_focus(), "focus stays where it was");
}

/// parity: SEL-037
#[gtk::test]
fn select_matching_selects_the_shown_items_whose_names_match() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.select_matching("*");
    assert_eq!(
        selected(&test),
        [0, 1, 2, 3],
        "hidden files are not shown, so not matched"
    );
    test.window.select_matching(" *.TXT ");
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt", "Notes 10.txt", "Résumé.txt"]
    );
    test.window.select_matching("");
    assert!(selected(&test).is_empty(), "a blank pattern selects nothing");
}

/// parity: SEL-006
#[gtk::test]
fn invert_selection_swaps_selected_and_unselected_items() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.activate("invert-selection", None);
    assert_eq!(selected(&test), [0, 2, 3]);
}

/// Shows `view` (`details` or an icon size) and waits until its items
/// are laid out.
fn show_view(test: &TestWindow, view: &str) {
    test.activate("view", Some(view));
    let pane = test.window.folder_pane();
    wait_until("the view to lay out its items", || {
        let shown = pane.view_widget();
        shown.height() > 0
            && pane
                .owners()
                .widget_at(0)
                .is_some_and(|cell| cell.is_ancestor(&shown) && cell.height() > 0)
    });
}

/// Selects `position` and makes it the list's current item.
fn start_at(test: &TestWindow, position: u32) {
    test.window.folder_pane().focus_view();
    test.window.folder_pane().reveal(position);
    test.window.folder_model().select_only(position);
}

/// parity: SEL-007, SEL-008, SEL-011
#[gtk::test]
fn arrows_home_end_and_page_keys_move_and_shift_extends_from_the_anchor() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    let none = gdk::ModifierType::empty();
    let shift = gdk::ModifierType::SHIFT_MASK;
    start_at(&test, 0);
    press_key(&test, gdk::Key::Down, none);
    press_key(&test, gdk::Key::Down, none);
    assert_eq!(selected(&test), [2]);
    press_key(&test, gdk::Key::Down, shift);
    press_key(&test, gdk::Key::Down, shift);
    assert_eq!(selected(&test), [2, 3, 4], "repeated Shift+Down keeps extending");
    press_key(&test, gdk::Key::Up, shift);
    assert_eq!(selected(&test), [2, 3]);
    press_key(&test, gdk::Key::End, none);
    let last = u32::try_from(LONG_FOLDER).expect("small") - 1;
    assert_eq!(selected(&test), [last]);
    press_key(&test, gdk::Key::Home, none);
    assert_eq!(selected(&test), [0]);
    press_key(&test, gdk::Key::Page_Down, none);
    let paged = selected(&test);
    assert!(
        paged.len() == 1 && paged[0] > 5,
        "Page Down moves a page: {paged:?}"
    );
}

/// parity: SEL-009
#[gtk::test]
fn ctrl_arrows_move_without_selecting_and_ctrl_space_toggles() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let control = gdk::ModifierType::CONTROL_MASK;
    start_at(&test, 0);
    press_key(&test, gdk::Key::Down, control);
    press_key(&test, gdk::Key::Down, control);
    assert_eq!(selected(&test), [0], "Ctrl+Down keeps the selection");
    press_key(&test, gdk::Key::space, control);
    assert_eq!(selected(&test), [0, 2], "Ctrl+Space toggles the current item");
}

/// parity: SEL-010
#[gtk::test]
fn arrows_move_in_two_dimensions_in_the_icon_grid() {
    let fixture = Fixture::with_files(PRIME_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    show_view(&test, "large");
    let none = gdk::ModifierType::empty();
    start_at(&test, 0);
    press_key(&test, gdk::Key::Right, none);
    assert_eq!(selected(&test), [1]);
    press_key(&test, gdk::Key::Down, none);
    let below = selected(&test);
    assert!(
        below.len() == 1 && below[0] > 2,
        "Down goes to the next row: {below:?}"
    );
    press_key(&test, gdk::Key::Left, none);
    assert_eq!(selected(&test), [below[0] - 1]);
    press_key(&test, gdk::Key::Up, none);
    assert_eq!(selected(&test), [0], "Up returns to the same column");
    // The last column of the row above the shorter last row.
    let columns = test.window.folder_pane().icon_view().grid().max_columns();
    let last = u32::try_from(PRIME_FOLDER).expect("small") - 1;
    let above = last - last % columns - 1;
    start_at(&test, above);
    press_key(&test, gdk::Key::Down, none);
    assert_eq!(selected(&test), [last], "Down into the short row goes to its end");
    press_key(&test, gdk::Key::Up, none);
    assert_eq!(selected(&test), [above], "Up remembers the column");
}

/// In Tiles the arrows move across and down the tiles, Down keeping to
/// the column; in Content, one row per item, Down and Up move a row, and
/// Home, End and Shift work as in the other views.
///
/// parity: VIEW-071, SEL-007, SEL-010
#[gtk::test]
fn arrows_move_through_tiles_and_content() {
    let fixture = Fixture::with_files(PRIME_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    let none = gdk::ModifierType::empty();
    show_view(&test, "tiles");
    start_at(&test, 0);
    press_key(&test, gdk::Key::Right, none);
    assert_eq!(selected(&test), [1]);
    let columns = test.window.folder_pane().icon_view().grid().max_columns();
    assert!(columns >= 2, "tiles fill several columns");
    press_key(&test, gdk::Key::Down, none);
    assert_eq!(selected(&test), [1 + columns], "Down goes to the next row");
    press_key(&test, gdk::Key::Up, none);
    assert_eq!(selected(&test), [1]);

    show_view(&test, "content");
    start_at(&test, 0);
    press_key(&test, gdk::Key::Down, none);
    assert_eq!(selected(&test), [1], "Down goes to the next row");
    press_key(&test, gdk::Key::Down, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(selected(&test), [1, 2], "Shift extends");
    press_key(&test, gdk::Key::End, none);
    let last = u32::try_from(PRIME_FOLDER).expect("small") - 1;
    assert_eq!(selected(&test), [last]);
    press_key(&test, gdk::Key::Home, none);
    assert_eq!(selected(&test), [0]);
    test.window.type_text("file 0004");
    assert_eq!(test.selected_names(), ["file 0004.txt"], "type-ahead selects");
}

/// In grouped Tiles and Content every key moves between items across
/// the headings, as in Details: Down from a group's last row rests on the
/// next group's heading, then goes to its first row, Right from a group's
/// last item goes to the next group's first, Up comes back to the column,
/// and Shift and Ctrl work as elsewhere.
///
/// parity: VIEW-071, VIEW-022, SEL-007, SEL-009, SEL-010
#[gtk::test]
fn arrows_cross_the_group_headings_in_tiles_and_content() {
    let fixture = Fixture::standard();
    fixture.write("Plan.md");
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let none = gdk::ModifierType::empty();
    let shift = gdk::ModifierType::SHIFT_MASK;
    let last = test.window.folder_pane().model().n_items() - 1;
    for view in ["tiles", "content"] {
        show_view(&test, view);
        start_at(&test, 0);
        press_key(&test, gdk::Key::Down, none);
        assert!(
            test.window.focused_heading().is_some(),
            "{view}: Down rests on the heading"
        );
        assert_eq!(
            selected(&test),
            [1],
            "{view}: a heading selects its group's items"
        );
        press_key(&test, gdk::Key::Down, none);
        assert_eq!(selected(&test), [1], "{view}: Down goes on past the heading");
        press_key(&test, gdk::Key::Up, none);
        press_key(&test, gdk::Key::Up, none);
        assert_eq!(selected(&test), [0], "{view}: Up passes it back");
        press_key(&test, gdk::Key::Right, none);
        assert_eq!(selected(&test), [1], "{view}: Right into the next group");
        press_key(&test, gdk::Key::Left, none);
        assert_eq!(selected(&test), [0]);
        press_key(&test, gdk::Key::End, none);
        assert_eq!(selected(&test), [last], "{view}: End");
        press_key(&test, gdk::Key::Home, shift);
        assert_eq!(
            selected(&test),
            (0..=last).collect::<Vec<u32>>(),
            "{view}: Shift+Home"
        );
        // Down from the first item rests on the next heading first.
        press_key(&test, gdk::Key::Down, none);
        press_key(&test, gdk::Key::Down, none);
        assert_eq!(selected(&test), [1], "{view}: a plain key selects one item");
        // Past the next group's heading.
        press_key(&test, gdk::Key::Down, gdk::ModifierType::CONTROL_MASK);
        press_key(&test, gdk::Key::Down, gdk::ModifierType::CONTROL_MASK);
        assert_eq!(selected(&test), [1], "{view}: Ctrl moves the focus alone");
        let focused = test.window.folder_pane().focused_position().expect("focus");
        assert!(focused > 1, "{view}: the focus moved on");
        press_key(&test, gdk::Key::Right, shift);
        let extended: Vec<u32> = (1..=focused + 1).collect();
        assert_eq!(selected(&test), extended, "{view}: Shift extends from the anchor");
    }
}

/// A group collapses under its heading and expands again, in Details,
/// Tiles and Content, as in Windows 11: with the keyboard on the heading,
/// Left collapses it, Right expands it, and Space and Enter select its
/// items. Its items
/// stay selected while hidden; the arrows, a Shift range and
/// type-to-select pass them by, and Select all still selects them.
///
/// parity: VIEW-022, VIEW-071, SEL-007, SEL-009
#[gtk::test]
fn a_group_collapses_under_its_heading_and_its_items_are_passed_by() {
    let fixture = Fixture::standard();
    fixture.write("Plan.md");
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let none = gdk::ModifierType::empty();
    let shift = gdk::ModifierType::SHIFT_MASK;
    let last = test.window.folder_pane().model().n_items() - 1;
    let shows = |position| {
        let grouped = test.window.folder_pane().grouped().expect("a grouped view");
        grouped.layout().expect("laid out").shows(position)
    };
    for view in ["details", "tiles", "content"] {
        show_view(&test, view);
        start_at(&test, 0);
        press_key(&test, gdk::Key::Up, none);
        let heading = test.window.focused_heading().expect("Up rests on the heading");
        assert_eq!(heading.group_index(), Some(0), "{view}");
        press_key(&test, gdk::Key::Left, none);
        assert!(
            heading.collapsed() && !shows(0),
            "{view}: Left collapses the group"
        );
        assert_eq!(selected(&test), [0], "{view}: a hidden item stays selected");
        press_key(&test, gdk::Key::Right, none);
        assert!(!heading.collapsed() && shows(0), "{view}: Right expands it");
        test.window.folder_pane().model().select_none();
        press_key(&test, gdk::Key::space, none);
        assert_eq!(selected(&test), [0], "{view}: Space selects the group's items");
        test.window.folder_pane().model().select_none();
        press_key(&test, gdk::Key::Return, none);
        assert_eq!(selected(&test), [0], "{view}: so does Enter");
        let grouped = test.window.folder_pane().grouped().expect("a grouped view");
        let heading_slot = grouped
            .layout()
            .and_then(|layout| layout.heading_slot(0))
            .expect("a heading");
        assert!(
            !grouped.is_selected(heading_slot),
            "{view}: an expanded heading shows no fill"
        );
        press_key(&test, gdk::Key::Left, none);
        assert!(heading.collapsed(), "{view}: Left collapses it again");
        assert!(
            grouped.is_selected(heading_slot),
            "{view}: a collapsed heading whose items are selected is filled"
        );
        assert!(
            test.window.focused_heading().is_some(),
            "{view}: the heading keeps the keyboard"
        );

        press_key(&test, gdk::Key::Down, none);
        press_key(&test, gdk::Key::Down, none);
        assert_eq!(selected(&test), [1], "{view}: the first item shown");
        press_key(&test, gdk::Key::End, shift);
        assert_eq!(
            selected(&test),
            (1..=last).collect::<Vec<u32>>(),
            "{view}: Shift+End"
        );
        start_at(&test, last);
        test.window.reset_typeahead();
        test.window.type_text("Documents");
        assert_eq!(
            selected(&test),
            [last],
            "{view}: type-to-select passes a hidden item by"
        );
        test.window.reset_typeahead();
        test.activate("select-all", None);
        assert_eq!(
            selected(&test),
            (0..=last).collect::<Vec<u32>>(),
            "{view}: Select all selects the hidden items too"
        );
        test.activate("expand-all-groups", None);
        assert!(shows(0), "{view}: Expand all groups");
        test.activate("collapse-all-groups", None);
        assert!(
            (0..=last).all(|position| !shows(position)),
            "{view}: Collapse all groups"
        );
        test.activate("expand-all-groups", None);
    }
}

/// Presses the primary button `presses` times at the middle of `widget`,
/// as far as the visible view's own capture-phase gestures go.
fn press_on(test: &TestWindow, widget: &gtk::Widget, presses: i32) {
    let view = test.window.folder_pane().view_widget();
    #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
    let middle = gtk::graphene::Point::new(widget.width() as f32 / 2.0, widget.height() as f32 / 2.0);
    let point = widget
        .compute_point(&view, &middle)
        .expect("the widget is in the view");
    let controllers = view.observe_controllers();
    let gestures = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .filter(|gesture| {
            gesture.propagation_phase() == gtk::PropagationPhase::Capture
                && gesture.button() == gdk::BUTTON_PRIMARY
        });
    for gesture in gestures {
        gesture.emit_by_name::<()>(
            "pressed",
            &[&presses, &f64::from(point.x()), &f64::from(point.y())],
        );
    }
}

/// The chevron, the title and the line of the shown heading of the
/// group at `index`.
fn heading_parts(test: &TestWindow, index: usize) -> (gtk::Widget, gtk::Widget, gtk::Widget) {
    use crate::folder_view::cells::FileCell;
    use crate::folder_view::grid::SlotKind;
    let view = test.window.folder_pane().view_widget();
    let cell = descendants::<FileCell>(&view)
        .into_iter()
        .filter(WidgetExt::is_mapped)
        .find(|cell| {
            cell.slot()
                .is_some_and(|slot| slot.kind() == SlotKind::Heading && slot.group_index() == Some(index))
        })
        .expect("the heading is on screen");
    let chevron = descendants::<gtk::Image>(&cell)
        .into_iter()
        .find(|image| image.has_css_class("group-chevron"))
        .expect("a chevron");
    let title = descendants::<gtk::Label>(&cell)
        .into_iter()
        .find(|label| label.has_css_class("group-title"))
        .expect("a title");
    let line = descendants::<gtk::Separator>(&cell)
        .into_iter()
        .next()
        .expect("a line");
    (chevron.upcast(), title.upcast(), line.upcast())
}

/// Clicks on a heading act as Windows 11's: the title or the line selects
/// every item of the group and gives the heading the keyboard, a
/// double-click collapses or expands the group, and the chevron collapses
/// or expands it without selecting.
///
/// parity: VIEW-022, VIEW-071
#[gtk::test]
fn a_click_on_a_heading_selects_its_group_and_the_chevron_folds_it() {
    let fixture = Fixture::standard();
    fixture.write("Plan.md");
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let last = test.window.folder_pane().model().n_items() - 1;
    let group: Vec<u32> = (2..=last).collect();
    for view in ["details", "tiles", "content"] {
        show_view(&test, view);
        start_at(&test, 0);
        let heading = test
            .window
            .folder_pane()
            .grouped()
            .and_then(|grouped| grouped.heading(2))
            .expect("a third group");
        let (_, title, line) = heading_parts(&test, 2);
        press_on(&test, &line, 1);
        assert_eq!(selected(&test), group, "{view}: the line selects the group");
        test.window.folder_pane().model().select_only(0);
        press_on(&test, &title, 1);
        assert_eq!(selected(&test), group, "{view}: the title selects the group");
        assert_eq!(
            test.window
                .focused_heading()
                .and_then(|heading| heading.group_index()),
            Some(2),
            "{view}: and gives the heading the keyboard"
        );
        // GTK may leave the keyboard on the list itself after the click
        // (KDE Plasma's Wayland session): the heading still takes Left.
        let list = test.window.folder_pane().view_widget();
        gtk::prelude::GtkWindowExt::set_focus(&test.window, Some(&list));
        press_key(&test, gdk::Key::Left, gdk::ModifierType::empty());
        assert!(
            heading.collapsed(),
            "{view}: Left collapses the clicked heading's group"
        );
        press_key(&test, gdk::Key::Right, gdk::ModifierType::empty());
        assert!(!heading.collapsed(), "{view}: Right expands it");
        // A double-click folds the group and leaves the selection as it
        // was before it, as in Windows 11.
        test.window.folder_pane().model().select_only(0);
        let (_, title, _) = heading_parts(&test, 2);
        press_on(&test, &title, 1);
        press_on(&test, &title, 2);
        assert!(heading.collapsed(), "{view}: a double-click collapses the group");
        assert_eq!(selected(&test), [0], "{view}: and leaves the selection as it was");
        test.window.folder_pane().model().select_only(0);
        let (chevron, _, _) = heading_parts(&test, 2);
        press_on(&test, &chevron, 1);
        assert!(!heading.collapsed(), "{view}: the chevron expands it");
        assert_eq!(selected(&test), [0], "{view}: without selecting");
    }
}

/// parity: SEL-016
#[gtk::test]
fn a_created_item_is_selected_and_scrolled_into_view() {
    let fixture = Fixture::with_files(LONG_FOLDER);
    let test = TestWindow::open(&fixture.uri());
    let last = u32::try_from(LONG_FOLDER).expect("small") - 1;
    test.window.folder_model().select_only(last);
    test.activate("duplicate", None);
    wait_until("the copy to be selected", || {
        test.selected_names()
            .first()
            .is_some_and(|name| name.contains(" - Copy"))
    });
    wait_until("the copy to be scrolled into view", || {
        test.window.folder_pane().scroll_position() > 0.0
    });
}

/// parity: SEL-036
#[gtk::test]
fn space_selects_the_current_item_and_never_deselects_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    start_at(&test, 0);
    press_key(&test, gdk::Key::Down, gdk::ModifierType::CONTROL_MASK);
    press_key(&test, gdk::Key::space, gdk::ModifierType::empty());
    assert_eq!(
        selected(&test),
        [0, 1],
        "Space adds the current item, as in Dolphin"
    );
    press_key(&test, gdk::Key::space, gdk::ModifierType::empty());
    assert_eq!(selected(&test), [0, 1], "Space on a selected item keeps it");
    test.window.type_text("notes");
    test.window.type_text(" 1");
    assert_eq!(
        test.selected_names(),
        ["Notes 10.txt"],
        "inside a prefix Space is prefix text"
    );
}

/// parity: SEL-034
#[gtk::test]
fn a_type_ahead_match_becomes_the_range_anchor() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    click(&test, 0, false, false);
    test.window.type_text("r");
    assert_eq!(selected(&test), [3]);
    press_key(&test, gdk::Key::Up, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(selected(&test), [2, 3], "Shift+Up extends from the match");
}
