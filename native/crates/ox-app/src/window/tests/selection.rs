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
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::window::rubber_band::BandMode;

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
    for view in ["details", "large"] {
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
    let crossed: Vec<u32> = (1..=last).collect();
    assert_eq!(
        selected(&test),
        crossed,
        "the selection follows the band before it ends"
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

/// The marker on an item's icon toggles that item and keeps the rest of
/// the selection, and shows the minus once the item is selected.
///
/// parity: SEL-014
#[gtk::test]
fn the_selection_marker_toggles_its_item_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    pane.model().select_only(1);
    let cell = pane
        .owners()
        .file_cell_at(2, &pane.view_widget())
        .expect("on screen");
    let marker = cell.selection_marker();
    marker.emit_clicked();
    assert_eq!(selected(&test), [1, 2]);
    assert_eq!(marker.tooltip_text().as_deref(), Some("Deselect"));
    marker.emit_clicked();
    assert_eq!(selected(&test), [1]);
    marker.set_visible(true);
    pane.owners().set_selection_markers(false);
    assert!(!marker.is_visible(), "Settings hides an already hovered marker");
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
            .is_some_and(|name| name.contains("(copy"))
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
