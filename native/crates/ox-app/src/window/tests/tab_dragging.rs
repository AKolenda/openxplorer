// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging tabs with a real pointer: which of the tab drag and the title
//! bar's window drag gets a press on a tab, the tab following the pointer
//! along its row, and drag and drop once it leaves the row (TAB-032 to
//! TAB-034, TAB-048).
//!
//! The title bar is a `GtkWindowHandle` and the tab strip is inside it, so
//! a press on a tab reaches both the tabs' drag source and the handle's
//! own drag gesture, and GTK alone decides which one takes it. These tests
//! therefore press and move a real pointer ([`Pointer`]) on the private
//! display instead of calling the drag's methods: only then do GTK's
//! gestures compete as they do on a desktop. With no window manager on
//! that display, GDK moves a window itself when the handle asks to, so a
//! window drag that wins shows as a window that moved.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{
    wait_for, wait_until, windows_besides, Fixture, OpenedWindows, TestWindow,
};
use crate::test_support::pointer::{screen_point, window_origin, Button, Pointer};
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// Where the pointer goes to find a window on the screen: any point works,
/// as GDK gives the pointer's position relative to the window.
const PROBE: (i32, i32) = (8, 8);

/// How long a held press waits before it moves: longer than the 100
/// milliseconds `GtkDragSource` waits before it starts a drag.
const HOLD: Duration = Duration::from_millis(200);

/// How a hand starts a drag.
#[derive(Debug, Clone, Copy)]
enum Pace {
    /// Moves at once: past the drag threshold before `GtkDragSource` may
    /// start a drag.
    Brisk,
    /// Holds the press a moment, then moves.
    Held,
}

fn tab_ids(window: &BrowserWindow) -> Vec<TabId> {
    let session = window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

fn active_tab(window: &BrowserWindow) -> Option<TabId> {
    window.imp().session.borrow().active_id()
}

/// The tab widget at `index` of `window`'s strip.
fn tab_widget(window: &BrowserWindow, index: usize) -> gtk::Widget {
    crate::window::widget_tree::children(&window.tab_strip().tab_list())
        .nth(index)
        .expect("the window shows the tab")
}

/// A window on `fixture` with a second tab on its Documents folder, in
/// front.
fn two_tabs(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    test
}

/// The screen point of `point` in `widget`.
fn on_screen(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>, point: (f64, f64)) -> (i32, i32) {
    screen_point(pointer, widget, point, PROBE)
}

/// The middle of `widget` on the screen.
fn middle_on_screen(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let (width, height) = (f64::from(widget.width()), f64::from(widget.height()));
    on_screen(pointer, widget, (width / 2.0, height / 2.0))
}

/// Where `window` is on the screen.
fn origin(pointer: &mut Pointer, window: &BrowserWindow) -> (i32, i32) {
    window_origin(pointer, window, PROBE)
}

/// Presses at `from`, moves to `to` at `pace` and releases there.
fn drag(pointer: &mut Pointer, from: (i32, i32), to: (i32, i32), pace: Pace) {
    press_and_glide(pointer, from, to, pace);
    pointer.release(Button::Primary);
}

/// Presses at `from` and moves to `to` at `pace`, still pressed, until
/// the window caught up.
fn press_and_glide(pointer: &mut Pointer, from: (i32, i32), to: (i32, i32), pace: Pace) {
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    if matches!(pace, Pace::Held) {
        wait_for(HOLD);
    }
    pointer.glide(from, to, 12);
    // The drop lands where the pointer rests, after GTK's drag caught up.
    wait_for(HOLD);
}

/// The left edge of `tab` in `window`'s row of tabs, where it is drawn now.
fn left_in_row(window: &BrowserWindow, tab: &gtk::Widget) -> f64 {
    let bounds = tab
        .compute_bounds(&window.tab_strip().tab_list())
        .expect("a shown tab");
    f64::from(bounds.x())
}

/// Whether `window` runs GTK's drag and drop for one of its tabs.
fn drags_and_drops(window: &BrowserWindow) -> bool {
    window.imp().outgoing_tab.borrow().is_some()
}

/// Turns the desktop's animations off until dropped, as reduced motion
/// does.
struct AnimationsOff(bool);

impl AnimationsOff {
    fn new() -> Self {
        let settings = gtk::Settings::default().expect("GTK tests run on a private display");
        let before = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        Self(before)
    }
}

impl Drop for AnimationsOff {
    fn drop(&mut self) {
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_enable_animations(self.0);
        }
    }
}

/// A pressed tab dragged along its row is the tab itself following the
/// pointer, held where it was grabbed, with no drag and drop, drag icon or
/// drop mark; the other tab slides aside once the dragged tab's edge
/// passes its middle, so the order shown is live, and a release keeps it.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_dragged_along_its_row_follows_the_pointer_and_the_other_slides_aside() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let (first_tab, second_tab) = (tab_widget(&test.window, 0), tab_widget(&test.window, 1));
    let (first_left, second_left) = (
        left_in_row(&test.window, &first_tab),
        left_in_row(&test.window, &second_tab),
    );
    let width = f64::from(first_tab.width());
    let from = middle_on_screen(&mut pointer, &first_tab);

    press_and_glide(&mut pointer, from, (from.0 + 60, from.1), Pace::Held);
    let followed = left_in_row(&test.window, &first_tab) - first_left;
    let other_before_its_middle = left_in_row(&test.window, &second_tab);
    let drag_and_drop_in_row = drags_and_drops(&test.window);
    #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
    let past = (from.0 + 60 + (width * 0.8) as i32, from.1);
    pointer.glide((from.0 + 60, from.1), past, 4);
    wait_for(HOLD);
    let other_after_its_middle = left_in_row(&test.window, &second_tab);
    let order_while_pressed = tab_ids(&test.window);
    let drag_and_drop_after = drags_and_drops(&test.window);
    pointer.release(Button::Primary);
    wait_until("the order to be kept", || {
        tab_ids(&test.window) == [second, first]
    });

    assert!(
        (followed - 60.0).abs() <= 1.5,
        "the tab followed the pointer: {followed}"
    );
    assert!(
        !drag_and_drop_in_row && !drag_and_drop_after,
        "no drag and drop along the row"
    );
    assert!(
        (other_before_its_middle - second_left).abs() <= 0.5,
        "the other tab stays until passed: {other_before_its_middle}"
    );
    assert!(
        (other_after_its_middle - first_left).abs() <= 0.5,
        "the other tab slid into the first place: {other_after_its_middle}"
    );
    assert_eq!(
        order_while_pressed,
        [first, second],
        "the session changes on release"
    );
    assert!(windows_besides(&[&test.window]).is_empty());
}

/// Escape while a tab is dragged along its row puts every tab back where
/// it was; the rest of the press drags nothing.
///
/// parity: TAB-032
#[gtk::test]
fn escape_during_a_row_drag_puts_the_tabs_back() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let order = tab_ids(&test.window);
    let [first, second] = order[..] else {
        panic!("two tabs are open");
    };
    let (first_tab, second_tab) = (tab_widget(&test.window, 0), tab_widget(&test.window, 1));
    let (first_left, second_left) = (
        left_in_row(&test.window, &first_tab),
        left_in_row(&test.window, &second_tab),
    );
    let from = middle_on_screen(&mut pointer, &first_tab);
    let past = middle_on_screen(&mut pointer, &second_tab);
    let past = (past.0 + 40, past.1);

    press_and_glide(&mut pointer, from, past, Pace::Held);
    let other_while_dragged = left_in_row(&test.window, &second_tab);
    pointer.tap_key(gtk::gdk::Key::Escape);
    wait_for(HOLD);
    // The dragged tab came to the front, so the strip is drawn anew.
    let tab_of = |id| {
        test.window
            .tab_strip()
            .tab_widget_of(id)
            .expect("the tab is shown")
    };
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    let after_escape = (
        left_in_row(&test.window, &tab_of(first)),
        left_in_row(&test.window, &tab_of(second)),
    );
    pointer.glide(past, (past.0 - 120, past.1), 4);
    let after_moving_on = left_in_row(&test.window, &tab_of(first));
    pointer.release(Button::Primary);
    wait_for(HOLD);

    assert!(
        (other_while_dragged - first_left).abs() <= 0.5,
        "the order was live"
    );
    assert!(
        (after_escape.0 - first_left).abs() <= 0.5 && (after_escape.1 - second_left).abs() <= 0.5,
        "every tab went back: {after_escape:?}"
    );
    assert!(
        (after_moving_on - first_left).abs() <= 0.5,
        "the rest of the press drags nothing: {after_moving_on}"
    );
    assert_eq!(tab_ids(&test.window), order);
    assert!(!drags_and_drops(&test.window));
    assert!(windows_besides(&[&test.window]).is_empty());
}

/// With the desktop's animations off, as reduced motion turns them, the
/// tabs move into place at once while the dragged tab still follows the
/// pointer, and a release keeps the order at once.
///
/// parity: TAB-032, ACC-010
#[gtk::test]
fn without_animations_tabs_jump_into_place_while_the_dragged_tab_follows() {
    let _off = AnimationsOff::new();
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let (first_tab, second_tab) = (tab_widget(&test.window, 0), tab_widget(&test.window, 1));
    let first_left = left_in_row(&test.window, &first_tab);
    let from = middle_on_screen(&mut pointer, &first_tab);
    // Past the other tab's middle, short of the row's end.
    let past = (from.0 + 150, from.1);

    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    wait_for(HOLD);
    pointer.glide(from, past, 3);
    // Two frames: far less than a slide would take.
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    let other_at_once = left_in_row(&test.window, &second_tab);
    let followed = left_in_row(&test.window, &first_tab) - first_left;
    pointer.release(Button::Primary);
    wait_for(std::time::Duration::from_millis(40));
    let order_at_once = tab_ids(&test.window);

    assert!(
        (other_at_once - first_left).abs() <= 0.5,
        "no slide: {other_at_once}"
    );
    assert!(
        (followed - f64::from(past.0 - from.0)).abs() <= 1.5,
        "the dragged tab follows the pointer: {followed}"
    );
    assert_eq!(order_at_once, [second, first]);
}

/// A tab dragged out of its row is carried by GTK's drag and drop; brought
/// back into the row of its own window it follows the pointer again, the
/// other tab sliding aside, and lands where it was shown.
///
/// parity: TAB-032, TAB-034
#[gtk::test]
fn a_tab_dragged_out_and_back_into_its_row_lands_where_it_was_shown() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let (first_tab, second_tab) = (tab_widget(&test.window, 0), tab_widget(&test.window, 1));
    let first_left = left_in_row(&test.window, &first_tab);
    let from = middle_on_screen(&mut pointer, &first_tab);
    let away = middle_on_screen(&mut pointer, test.window.folder_pane());
    let past = middle_on_screen(&mut pointer, &second_tab);
    let past = (past.0 + 40, past.1);

    press_and_glide(&mut pointer, from, away, Pace::Held);
    let carried = drags_and_drops(&test.window);
    let hint = test.window.folder_pane().drag_hint();
    pointer.glide(away, past, 12);
    wait_for(HOLD);
    // The strip drew the tabs anew when the dragged one came to the front.
    let second_tab = test
        .window
        .tab_strip()
        .tab_widget_of(second)
        .expect("the other tab is shown");
    let other_back_in_row = left_in_row(&test.window, &second_tab);
    pointer.release(Button::Primary);
    wait_until("the order to be kept", || {
        tab_ids(&test.window) == [second, first]
    });

    assert!(carried, "away from the row the tab is carried by drag and drop");
    assert_eq!(hint, None, "nothing is written over the file list");
    assert!(
        (other_back_in_row - first_left).abs() <= 0.5,
        "back in the row the other tab made room: {other_back_in_row}"
    );
    assert!(windows_besides(&[&test.window]).is_empty(), "no window opens");
}

/// The desktop's drag threshold, in pixels.
fn drag_threshold() -> f64 {
    let settings = gtk::Settings::default().expect("GTK tests run on a private display");
    f64::from(settings.gtk_dnd_drag_threshold())
}

/// The farthest the title bar's own window drag sees the pointer go from
/// its press, while `during` runs.
fn farthest_window_drag(window: &BrowserWindow, during: impl FnOnce()) -> f64 {
    let handle = window
        .titlebar()
        .and_downcast::<gtk::WindowHandle>()
        .expect("the title bar is the window handle");
    let window_drag = handle
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::GestureDrag>().ok())
        .expect("the window handle drags the window with its own gesture");
    let farthest = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));
    let seen = std::rc::Rc::clone(&farthest);
    let watch = window_drag.connect_drag_update(move |_, x, y| {
        seen.set(seen.get().max(x.abs()).max(y.abs()));
    });
    during();
    window_drag.disconnect(watch);
    farthest.get()
}

/// Pressing a tab and dragging it along the row reorders the tabs, whether
/// the hand moves at once or holds first; the window never moves, and the
/// title bar's window drag never sees the pointer pass the drag threshold:
/// the tab drag has taken the press by then.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_dragged_along_the_row_reorders_and_never_moves_the_window() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    for pace in [Pace::Brisk, Pace::Held] {
        let test = two_tabs(&fixture);
        let [first, second] = tab_ids(&test.window)[..] else {
            panic!("two tabs are open");
        };
        let before = origin(&mut pointer, &test.window);
        let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 0));
        let last = tab_widget(&test.window, 1);
        let to = on_screen(&mut pointer, &last, (f64::from(last.width()) - 12.0, 12.0));

        let farthest = farthest_window_drag(&test.window, || drag(&mut pointer, from, to, pace));
        wait_for(HOLD);
        let after = origin(&mut pointer, &test.window);

        assert!(
            farthest <= drag_threshold(),
            "{pace:?}: the window drag saw the pointer {farthest} pixels away"
        );
        assert_eq!(after, before, "{pace:?}: the window stays where it was");
        assert_eq!(
            tab_ids(&test.window),
            [second, first],
            "{pace:?}: the tab moved along the row"
        );
        assert!(
            windows_besides(&[&test.window]).is_empty(),
            "{pace:?}: no window opens"
        );
        assert!(
            test.window.imp().outgoing_tab.borrow().is_none(),
            "{pace:?}: the drag ended"
        );
    }
}

/// Dragging a tab down out of the row and releasing it over the file list
/// of its own window opens it in a new window, as tearing a tab out of
/// Explorer's or Chrome's strip does; the window it left stays put.
///
/// parity: TAB-034
#[gtk::test]
fn a_tab_dragged_down_into_the_file_list_opens_in_a_new_window() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    for pace in [Pace::Brisk, Pace::Held] {
        let test = two_tabs(&fixture);
        let before = origin(&mut pointer, &test.window);
        let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 1));
        let to = middle_on_screen(&mut pointer, test.window.folder_pane());

        press_and_glide(&mut pointer, from, to, pace);
        let hint = test.window.folder_pane().drag_hint();
        pointer.release(Button::Primary);
        let opened = OpenedWindows::only(&[&test.window]);
        let after = origin(&mut pointer, &test.window);

        assert_eq!(
            opened.window().current_uri(),
            Some(fixture.uri_of("Documents")),
            "{pace:?}: the new window shows the dragged tab"
        );
        assert_eq!(test.window.tab_count(), 1, "{pace:?}: the tab left this window");
        assert_eq!(hint, None, "{pace:?}: nothing is written over the file list");
        assert_eq!(after, before, "{pace:?}: the window stays where it was");
    }
}

/// The blank part of the title bar still drags the window, and a tab
/// dropped on another window's row moves into that window before the tab
/// it was dropped on.
///
/// parity: TAB-033, TAB-048
#[gtk::test]
fn a_tab_dropped_on_another_windows_row_moves_there_where_it_was_dropped() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let other = test.open_beside(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&other.window, 3);
    let other_first = tab_ids(&other.window)[0];
    // The new window covers the first: drag it down by its blank title
    // bar, so both rows show.
    let blank =
        crate::test_support::harness::descendants::<gtk::Box>(&other.window.titlebar().expect("a title bar"))
            .into_iter()
            .find(|child| child.has_css_class("title-drag"))
            .expect("the bar has its blank drag area");
    let other_before = origin(&mut pointer, &other.window);
    let grab = middle_on_screen(&mut pointer, &blank);
    drag(&mut pointer, grab, (grab.0, grab.1 + 150), Pace::Held);
    wait_for(HOLD);
    let other_after = origin(&mut pointer, &other.window);
    assert_eq!(
        other_after,
        (other_before.0, other_before.1 + 150),
        "the blank title bar drags the window"
    );

    let before = origin(&mut pointer, &test.window);
    let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 1));
    let target = tab_widget(&other.window, 0);
    let target_left = left_in_row(&other.window, &target);
    let width = tab_widget(&test.window, 1)
        .compute_bounds(&test.window)
        .map_or(0.0, |bounds| f64::from(bounds.width()));
    let to = on_screen(&mut pointer, &target, (12.0, f64::from(target.height()) / 2.0));
    press_and_glide(&mut pointer, from, to, Pace::Brisk);
    let made_room = left_in_row(&other.window, &target) - target_left;
    pointer.release(Button::Primary);
    wait_until("the tab to leave", || test.window.tab_count() == 1);

    assert!(
        (made_room - (width + 2.0)).abs() <= 0.5,
        "the other window's tabs made room for the tab: {made_room}"
    );
    assert_eq!(other.window.tab_count(), 2);
    assert_eq!(other.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(
        tab_ids(&other.window)[1],
        other_first,
        "the tab went before the first"
    );
    assert_eq!(
        origin(&mut pointer, &test.window),
        before,
        "the window stays where it was"
    );
    assert_eq!(
        origin(&mut pointer, &other.window),
        other_after,
        "the window it went to stays too"
    );
}

/// A plain click on a tab shows it, also when the hand slips by less than
/// the drag threshold; a middle-click closes a tab and its close button
/// closes it too. None of them drags a tab or moves the window.
///
/// parity: TAB-002, TAB-004
#[gtk::test]
fn clicks_on_a_tab_show_or_close_it_and_drag_nothing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let before = origin(&mut pointer, &test.window);
    let (x, y) = middle_on_screen(&mut pointer, &tab_widget(&test.window, 0));
    #[expect(clippy::cast_possible_truncation, reason = "the threshold is a few pixels")]
    let slip = (drag_threshold() / 2.0) as i32;

    pointer.move_to(x, y);
    pointer.press(Button::Primary);
    pointer.move_to(x + slip, y + 1);
    pointer.release(Button::Primary);
    wait_until("the clicked tab to show", || {
        active_tab(&test.window) == Some(first)
    });
    let order = tab_ids(&test.window);
    let (x, y) = middle_on_screen(&mut pointer, &tab_widget(&test.window, 1));
    pointer.move_to(x, y);
    pointer.press(Button::Middle);
    pointer.release(Button::Middle);
    wait_until("the middle-clicked tab to close", || test.window.tab_count() == 1);
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the tab again");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let close = tab_widget(&test.window, 1)
        .last_child()
        .expect("the tab has a close button");
    let (x, y) = middle_on_screen(&mut pointer, &close);
    pointer.move_to(x, y);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    wait_until("the closed tab to go", || test.window.tab_count() == 1);

    assert_eq!(order, [first, second], "a click reorders nothing");
    assert_eq!(
        origin(&mut pointer, &test.window),
        before,
        "the window stays where it was"
    );
    assert!(windows_besides(&[&test.window]).is_empty());
}
