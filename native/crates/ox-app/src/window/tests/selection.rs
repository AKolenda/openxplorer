// SPDX-License-Identifier: AGPL-3.0-only
//! Selecting with the mouse and the keyboard in a real window, against
//! `selectEntry`, `onKey` and the blank-space click of
//! `desktop/ui/app.js`, with Dolphin's and Explorer's keys.
//!
//! GTK has no public way to synthesise pointer or key events. A click on
//! an item runs `list.select-item`, where GTK's own click ends; a
//! press on blank space emits `pressed` on the views' capture-phase click
//! gestures; a key runs the shortcut GTK would find for it, bubbling up
//! from the focused widget.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{press_shortcut, run_shortcut};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// A folder long enough to scroll.
const LONG_FOLDER: usize = 300;

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

/// Every shortcut `widget`'s own shortcut controllers hold.
fn shortcuts_of(widget: &gtk::Widget) -> Vec<gtk::Shortcut> {
    let controllers: Vec<gtk::ShortcutController> = widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .collect();
    controllers
        .iter()
        .flat_map(|controller| controller.iter::<glib::Object>())
        .filter_map(Result::ok)
        .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
        .collect()
}

/// Presses `keyval` with `modifiers`: the first shortcut for it, from the
/// focused widget up, runs, as GTK's key handling finds it.
fn press_key(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) {
    let key = gtk::KeyvalTrigger::new(keyval, modifiers);
    let mut widget = gtk::prelude::GtkWindowExt::focus(&test.window);
    while let Some(current) = widget {
        for shortcut in shortcuts_of(&current) {
            let is_the_key = shortcut
                .trigger()
                .is_some_and(|trigger| trigger.to_str() == key.to_str());
            let action = shortcut.action().filter(|_| is_the_key);
            let handled = action.is_some_and(|action| {
                action.activate(
                    gtk::ShortcutActionFlags::empty(),
                    &current,
                    shortcut.arguments().as_ref(),
                )
            });
            if handled {
                return;
            }
        }
        widget = current.parent();
    }
    panic!("nothing handles {keyval:?} with {modifiers:?}");
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

/// Whether the visible view starts a rubber band.
fn allows_rubber_band(test: &TestWindow) -> bool {
    let view = test.window.folder_pane().view_widget();
    if let Some(columns) = view.downcast_ref::<gtk::ColumnView>() {
        return columns.enables_rubberband();
    }
    view.downcast_ref::<gtk::GridView>()
        .is_some_and(gtk::GridView::enables_rubberband)
}

/// parity: SEL-001
#[gtk::test]
fn click_ctrl_click_and_shift_click_select_like_explorer() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    click(&test, 1, false, false);
    assert_eq!(selected(&test), [1]);
    assert!(
        test.window.folder_pane().view_has_focus(),
        "a click focuses the list"
    );
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

/// parity: SEL-012
#[gtk::test]
fn a_rubber_band_starts_on_blank_space_only() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for view in ["details", "large"] {
        show_view(&test, view);
        let item = test
            .window
            .folder_pane()
            .owners()
            .widget_at(1)
            .expect("on screen");
        let view_widget = test.window.folder_pane().view_widget();
        let bounds = item
            .compute_bounds(&view_widget)
            .expect("a shown item has bounds");
        press_at(
            &test,
            f64::from(bounds.x() + 4.0),
            f64::from(bounds.y() + bounds.height() / 2.0),
        );
        assert!(!allows_rubber_band(&test), "{view}: dragging an item drags it");
        let (x, y) = blank_point(&test);
        press_at(&test, x, y);
        assert!(allows_rubber_band(&test), "{view}: blank space starts a band");
    }
}

/// parity: SEL-004
#[gtk::test]
fn ctrl_a_selects_every_shown_item_outside_text_fields() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.search_box().focus();
    run_shortcut(&test, gdk::Key::a, gdk::ModifierType::CONTROL_MASK);
    assert!(selected(&test).is_empty(), "the search box keeps its Ctrl+A");
    press_shortcut(&test, gdk::Key::a, gdk::ModifierType::CONTROL_MASK);
    assert_eq!(selected(&test), [0, 1, 2, 3]);
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
    let fixture = Fixture::with_files(LONG_FOLDER);
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
fn space_selects_the_current_item() {
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
    test.window.type_text("notes");
    test.window.type_text(" 1");
    assert_eq!(
        test.selected_names(),
        ["Notes 10.txt"],
        "inside a prefix Space is prefix text"
    );
}
