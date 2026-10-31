// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging tabs with a real pointer: which of the tab drag and the title
//! bar's window drag gets a press on a tab (TAB-032 to TAB-034, TAB-048).
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
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    if matches!(pace, Pace::Held) {
        wait_for(HOLD);
    }
    pointer.glide(from, to, 12);
    // The drop lands where the pointer rests, after GTK's drag caught up.
    wait_for(HOLD);
    pointer.release(Button::Primary);
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

        drag(&mut pointer, from, to, pace);
        let opened = OpenedWindows::only(&[&test.window]);
        let after = origin(&mut pointer, &test.window);

        assert_eq!(
            opened.window().current_uri(),
            Some(fixture.uri_of("Documents")),
            "{pace:?}: the new window shows the dragged tab"
        );
        assert_eq!(test.window.tab_count(), 1, "{pace:?}: the tab left this window");
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
    let to = on_screen(&mut pointer, &target, (12.0, f64::from(target.height()) / 2.0));
    drag(&mut pointer, from, to, Pace::Brisk);
    wait_until("the tab to leave", || test.window.tab_count() == 1);

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
