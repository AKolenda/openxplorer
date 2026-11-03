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

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{
    wait_for, wait_until, windows_besides, Fixture, OpenedWindows, TestWindow,
};
use crate::test_support::pointer::{screen_point, window_origin, Button, Pointer};
use crate::window::session::{Tab, TabId};
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

/// A second window beside `test`, dragged 150 pixels down by its blank
/// title bar, so both tab rows show; with where it was and is.
fn open_another_window_lower(
    test: &TestWindow,
    fixture: &Fixture,
    pointer: &mut Pointer,
) -> (TestWindow, (i32, i32), (i32, i32)) {
    let other = test.open_beside(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&other.window, 3);
    // The new window covers the first: drag it down by its blank title
    // bar, so both rows show.
    let blank =
        crate::test_support::harness::descendants::<gtk::Box>(&other.window.titlebar().expect("a title bar"))
            .into_iter()
            .find(|child| child.has_css_class("title-drag"))
            .expect("the bar has its blank drag area");
    let before = origin(pointer, &other.window);
    let grab = middle_on_screen(pointer, &blank);
    drag(pointer, grab, (grab.0, grab.1 + 150), Pace::Held);
    wait_for(HOLD);
    let after = origin(pointer, &other.window);
    (other, before, after)
}

/// Where `window` is on the screen, while the pointer, at `at` on the
/// screen, is over it, without moving the pointer, which would move a
/// window the desktop is moving with it.
fn origin_under(window: &BrowserWindow, at: (i32, i32)) -> Option<(i32, i32)> {
    let surface = window.surface()?;
    let device = gtk::gdk::Display::default()?.default_seat()?.pointer()?;
    let (x, y, _) = surface.device_position(&device)?;
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    Some((at.0 - x.round() as i32, at.1 - y.round() as i32))
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

/// A tab let go into its place without a slide is grabbed again before
/// the main loop turns, as presses queued behind the release are: it goes
/// on following the pointer, and the release's settling, overtaken, does
/// not end the new drag. The strip is called directly, since a real
/// pointer lets the main loop turn between its steps.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_grabbed_again_before_its_release_settled_goes_on_following() {
    let _off = AnimationsOff::new();
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    let [first, _] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, first);
    let settled = Rc::new(Cell::new(false));

    strip.grab_tab(first, (x, 10.0));
    strip.follow_pointer(first, x + 40.0);
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            settled,
            move |_| settled.set(true)
        ),
    );
    strip.follow_pointer(first, x + 60.0);
    wait_for(Duration::from_millis(40));

    assert!(!settled.get(), "the overtaken release did not settle");
    assert_eq!(strip.live_tab(), Some(first));
    assert!(strip.is_following(), "the tab follows the pointer");
}

/// Another tab grabbed before a release without a slide settled: the
/// released tab takes its place first, at once, and the new drag goes on.
///
/// parity: TAB-032
#[gtk::test]
fn another_tab_grabbed_before_a_release_settled_follows_once_it_landed() {
    let _off = AnimationsOff::new();
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let strip = test.window.tab_strip();
    let (x, other_x) = (grab_point(&test.window, first), grab_point(&test.window, second));
    let settled = Rc::new(Cell::new(false));

    strip.grab_tab(first, (x, 10.0));
    strip.follow_pointer(first, x + 40.0);
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            settled,
            move |_| settled.set(true)
        ),
    );
    strip.grab_tab(second, (other_x, 10.0));
    strip.follow_pointer(second, other_x + 20.0);
    let settled_first = settled.get();
    wait_for(Duration::from_millis(40));

    assert!(settled_first, "the released tab took its place first");
    assert_eq!(strip.live_tab(), Some(second));
    assert!(strip.is_following(), "the other tab follows the pointer");
}

/// Another tab grabbed before a release settled, after it changed folder
/// meanwhile: the release's landing draws that tab anew, and the new drag
/// moves the tab as drawn now, not the widget it had when grabbed.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_that_changed_folder_while_a_release_settled_follows_once_grabbed() {
    let _off = AnimationsOff::new();
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    assert_eq!(active_tab(&test.window), Some(second));
    let strip = test.window.tab_strip();
    let (x, other_x) = (grab_point(&test.window, first), grab_point(&test.window, second));

    strip.grab_tab(first, (x, 10.0));
    strip.follow_pointer(first, x + 40.0);
    strip.let_go(true, |_| ());
    // The tab in front goes elsewhere: the strip waits to draw it anew.
    test.window.navigate(&fixture.uri_of("Music")).expect("a folder");
    let grabbed_widget = strip.tab_widget_of(second).expect("a shown tab");
    strip.grab_tab(second, (other_x, 10.0));
    strip.follow_pointer(second, other_x + 20.0);

    let dragged = strip.tab_widget_of(second).expect("a shown tab");
    assert_ne!(dragged, grabbed_widget, "the tab was drawn anew");
    assert_eq!(dragged.parent(), Some(strip.tab_list().upcast::<gtk::Widget>()));
    assert_eq!(strip.live_tab(), Some(second));
    assert!(strip.is_following(), "the tab follows the pointer");
}

/// A tab let go before a tab that closes while it slides into place lands
/// before the next tab still open, where the strip showed it, not at the
/// end of the row.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_whose_landing_neighbour_closes_while_it_settles_lands_before_the_next() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    for folder in ["Music", "Pictures"] {
        test.window.add_tab(&fixture.uri_of(folder)).expect("a folder");
        test.wait_for_listing("another tab");
    }
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [first, second, third, fourth] = tab_ids(&test.window)[..] else {
        panic!("four tabs are open");
    };
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, first);
    let second_width = strip.tab_width(second).expect("a shown tab");
    let moved = Rc::new(Cell::new(None));

    strip.grab_tab(first, (x, 10.0));
    // Past the second tab's middle, short of the third's.
    strip.follow_pointer(first, grab_point(&test.window, second) + second_width / 2.0);
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            moved,
            move |went| moved.set(Some(went))
        ),
    );
    test.window.close_tab(third);
    crate::test_support::harness::wait_until("the tab settled", || moved.get().is_some());

    assert_eq!(moved.get(), Some(Some((first, Some(fourth)))));
}

/// F3 while a tab is dragged along its row splits no tab, not even the
/// one in front that is not dragged: splitting gives that tab another id,
/// by which the dragged tab keeps where it lands, so it would land at the
/// end of the row instead.
///
/// parity: TAB-032, VIEW-059
#[gtk::test]
fn no_tab_splits_while_another_is_dragged_along_its_row() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    test.window.add_tab(&fixture.uri_of("Music")).expect("a folder");
    test.wait_for_listing("the third tab");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [_, second, third] = tab_ids(&test.window)[..] else {
        panic!("three tabs are open");
    };
    let strip = test.window.tab_strip();
    let moved = Rc::new(Cell::new(None));

    strip.grab_tab(third, (grab_point(&test.window, third), 10.0));
    // Before the second tab's middle: the order shown changes.
    strip.follow_pointer(third, grab_point(&test.window, second) - 20.0);
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    test.window.switch_tab(second);
    test.activate("split-view", None);
    let split = test
        .window
        .imp()
        .session
        .borrow()
        .active()
        .is_some_and(Tab::is_split);
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            moved,
            move |went| moved.set(Some(went))
        ),
    );
    wait_until("the tab settled", || moved.get().is_some());

    assert!(!split, "the tab in front stays whole");
    assert_eq!(
        moved.get(),
        Some(Some((third, Some(second)))),
        "the dragged tab lands before it"
    );
}

/// With "Tab key switches between split panes" on, Tab while a split tab
/// is dragged along its row keeps its active pane active: the drag holds
/// the tab by that pane, so leaving the row after a switch would carry the
/// other pane out and leave the tab behind as well.
///
/// parity: TAB-032, VIEW-059
#[gtk::test]
fn tab_keeps_the_active_pane_of_a_split_tab_dragged_along_its_row() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    ox_core::settings::Settings::open(test.settings_directory())
        .update_preferences(&ox_core::settings::PreferencesUpdate {
            tab_switches_split_panes: Some(true),
            ..ox_core::settings::PreferencesUpdate::default()
        })
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the choice", || {
        test.context.settings_data().preferences.tab_switches_split_panes
    });
    test.activate("split-view", None);
    test.wait_for_listing("the new pane");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let dragged = active_tab(&test.window).expect("a tab in front");
    let side = test.window.active_side();
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, dragged);

    strip.grab_tab(dragged, (x, 10.0));
    strip.follow_pointer(dragged, x - 40.0);
    test.window.tab_to_other_pane();

    assert_eq!(
        active_tab(&test.window),
        Some(dragged),
        "the dragged pane stays active"
    );
    assert_eq!(test.window.active_side(), side);
    assert_eq!(strip.live_tab(), Some(dragged));
}

/// Keyboard focus moved into the inactive pane of a split tab dragged
/// along its row goes back to its active pane, by which the drag holds the
/// tab, so keys act on the pane the window shows active.
///
/// parity: TAB-032, VIEW-059
#[gtk::test]
fn focus_moved_into_the_other_pane_of_a_dragged_split_tab_goes_back() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    test.activate("split-view", None);
    test.wait_for_listing("the new pane");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let dragged = active_tab(&test.window).expect("a tab in front");
    let side = test.window.active_side();
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, dragged);

    strip.grab_tab(dragged, (x, 10.0));
    strip.follow_pointer(dragged, x - 40.0);
    test.window.pane_on(side.other()).focus_view();

    wait_until("focus back in the active pane", || {
        gtk::prelude::RootExt::focus(&test.window)
            .is_some_and(|focus| focus.is_ancestor(test.window.pane_on(side)))
    });
    assert_eq!(test.window.active_side(), side);
    assert_eq!(active_tab(&test.window), Some(dragged));
    assert_eq!(strip.live_tab(), Some(dragged));
}

/// A pane of a split tab clicked while the tab, let go past another a
/// moment ago, still slides into its new place becomes active: the tab
/// lands at once, by the pane it was dragged with, then the pane switches,
/// so the window's active pane is the one the click focused.
///
/// parity: TAB-032, VIEW-059
#[gtk::test]
fn a_pane_clicked_while_its_split_tab_settles_becomes_active_once_it_landed() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    test.activate("split-view", None);
    test.wait_for_listing("the new pane");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [first, dragged] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    assert_eq!(
        active_tab(&test.window),
        Some(dragged),
        "the split tab is in front"
    );
    let side = test.window.active_side();
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, dragged);
    let moved = Rc::new(Cell::new(None));

    strip.grab_tab(dragged, (x, 10.0));
    // Past the first tab's start: the order shown changes.
    strip.follow_pointer(dragged, grab_point(&test.window, first) - 20.0);
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            moved,
            move |went| moved.set(Some(went))
        ),
    );
    assert!(moved.get().is_none(), "the tab still slides into place");
    test.window.activate_pane(side.other());

    assert_eq!(moved.get(), Some(Some((dragged, Some(first)))), "it landed first");
    assert_eq!(test.window.active_side(), side.other());
    assert_ne!(
        active_tab(&test.window),
        Some(dragged),
        "the other pane is in front"
    );
    assert_eq!(strip.live_tab(), None);
}

/// A tab of another window dropped before a tab of this row that closed
/// while it was dragged over the row lands before the next tab still
/// open, where its stand-in showed it, not at the end of the row.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_dragged_in_before_a_tab_closed_meanwhile_lands_before_the_next() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    for folder in ["Music", "Pictures"] {
        test.window.add_tab(&fixture.uri_of(folder)).expect("a folder");
        test.wait_for_listing("another tab");
    }
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [first, _, third, fourth] = tab_ids(&test.window)[..] else {
        panic!("four tabs are open");
    };
    let strip = test.window.tab_strip();
    let view = strip.tab_view(first).expect("a shown tab");

    // Over the third tab's first half: the stand-in goes before it.
    strip.host_guest(&view, grab_point(&test.window, third), 0.0);
    test.window.close_tab(third);
    let landing = strip.land_guest();

    assert_eq!(
        landing,
        Some(crate::window::tab_strip::Landing { before: Some(fourth) })
    );
}

/// A tab of another window over the blank title bar beside a window's
/// only tab: its stand-in goes after that tab, where the pointer is, and a
/// drop there without another motion lands it at the end, though the row
/// was only as wide as one tab when the stand-in came.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_dragged_in_beside_a_windows_only_tab_lands_after_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [only] = tab_ids(&test.window)[..] else {
        panic!("one tab is open");
    };
    let strip = test.window.tab_strip();
    let view = strip.tab_view(only).expect("a shown tab");
    let bounds = strip.tab_bounds_in(only, strip).expect("a shown tab");

    strip.host_guest(&view, f64::from(bounds.x() + bounds.width()) + 40.0, 0.0);
    let landing = strip.land_guest();

    assert_eq!(landing, Some(crate::window::tab_strip::Landing { before: None }));
}

/// Three tabs and the share of its width a tab dragged in from another
/// window was grabbed at: 160 pixels into a 215 pixel tab.
fn three_tabs_and_a_wide_tabs_grab(fixture: &Fixture) -> (TestWindow, [TabId; 3], f64) {
    let test = two_tabs(fixture);
    test.window.add_tab(&fixture.uri_of("Music")).expect("a folder");
    test.wait_for_listing("a third tab");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let [first, second, third] = tab_ids(&test.window)[..] else {
        panic!("three tabs are open");
    };
    let share = crate::window::tab_layout::grab_share(160.0, Some(215.0));
    (test, [first, second, third], share)
}

/// A tab grabbed 160 pixels into a 215 pixel tab, dragged over a row
/// whose tabs are 100 pixels wide, near the right edge of its third tab:
/// its stand-in is held at the same share of its width, under the pointer,
/// so it takes the third tab's place, not the second's.
///
/// parity: TAB-032
#[gtk::test]
fn a_wide_tab_dragged_into_a_row_of_narrow_tabs_stays_under_the_pointer() {
    let fixture = Fixture::standard();
    let (test, [first, _, third], share) = three_tabs_and_a_wide_tabs_grab(&fixture);
    let strip = test.window.tab_strip();
    strip.set_tab_width(100);
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let width = strip.tab_width(third).expect("a shown tab");
    assert!(width < 110.0, "narrow tabs: {width}");
    let bounds = strip.tab_bounds_in(third, strip).expect("a shown tab");
    let view = strip.tab_view(first).expect("a shown tab");

    strip.host_guest(&view, f64::from(bounds.x()) + 0.9 * width, share);
    let landing = strip.land_guest();

    assert_eq!(
        landing,
        Some(crate::window::tab_strip::Landing { before: Some(third) })
    );
}

/// A tab dragged in from another window, held over the right part of the
/// row's third tab, when the row's tabs then narrow without the pointer
/// moving, as the window does: the stand-in is held at the same share of
/// its new width, under the pointer, now past the narrowed tabs, so it
/// lands at the end.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_dragged_in_stays_under_the_pointer_when_the_rows_tabs_narrow() {
    let fixture = Fixture::standard();
    let (test, [first, _, third], share) = three_tabs_and_a_wide_tabs_grab(&fixture);
    let strip = test.window.tab_strip();
    let width = strip.tab_width(third).expect("a shown tab");
    assert!(width >= 150.0, "wide tabs: {width}");
    let bounds = strip.tab_bounds_in(third, strip).expect("a shown tab");
    let view = strip.tab_view(first).expect("a shown tab");

    strip.host_guest(&view, f64::from(bounds.x()) + 0.9 * width, share);
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    strip.set_tab_width(100);
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let landing = strip.land_guest();

    assert_eq!(landing, Some(crate::window::tab_strip::Landing { before: None }));
}

/// A tab let go past another and pressed again while it still slides into
/// its new place, before the strip took the new order: it is held where
/// the press met it as drawn, so the next motion moves it with the
/// pointer, not a tab's width away.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_pressed_again_while_it_slides_into_place_is_held_where_it_is_drawn() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    let [first, _] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let strip = test.window.tab_strip();
    let x = grab_point(&test.window, first);
    let frames = |count| crate::test_support::harness::wait_for_frames(&test.window, count);

    strip.grab_tab(first, (x, 10.0));
    // Past the other tab: the order shown changes.
    strip.follow_pointer(first, x + 150.0);
    frames(2);
    strip.let_go(true, |_| ());
    // One frame into the 200 ms slide into its new place.
    frames(1);
    let drawn = grab_point(&test.window, first);
    strip.grab_tab(first, (drawn, 10.0));
    let grabbed_at = strip.grab_offset(first).map(|(along, _)| along);
    strip.follow_pointer(first, drawn);
    frames(1);
    let after = grab_point(&test.window, first);

    assert!(
        grabbed_at.is_some_and(|along| (along - 20.0).abs() <= 1.0),
        "held 20 pixels into the tab as drawn: {grabbed_at:?}"
    );
    assert!(
        (after - drawn).abs() <= 1.0,
        "the tab stays under the pointer: {drawn} then {after}"
    );
}

/// A tab let go past another and pressed again while it still slides
/// into its new place, the press held until it is there and the other tab
/// slid under the point pressed: the press drags the tab it met, not the
/// one now under that point.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_pressed_while_it_slides_into_place_is_the_one_dragged_once_it_landed() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let strip = test.window.tab_strip();
    let first_tab = tab_widget(&test.window, 0);
    let width = f64::from(first_tab.width());
    let pressed_in_row = left_in_row(&test.window, &first_tab) + 20.0 + (width * 0.7).trunc();
    let from = on_screen(&mut pointer, &first_tab, (20.0, 10.0));
    // Past the other tab's middle, still over its place.
    #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
    let released = (from.0 + (width * 0.7) as i32, from.1);

    press_and_glide(&mut pointer, from, released, Pace::Held);
    pointer.release(Button::Primary);
    // Pressed again on the tab as drawn, at the start of its slide.
    pointer.press(Button::Primary);
    wait_until("the tab to land", || {
        tab_ids(&test.window) == [second, first] && strip.live_tab().is_none()
    });
    let under_the_press = strip
        .tab_bounds_in(second, &strip.tab_list())
        .is_some_and(|bounds| pressed_in_row < f64::from(bounds.x() + bounds.width()));
    pointer.glide(released, (released.0 + 40, released.1), 4);
    wait_for(HOLD);
    let dragged = strip.live_tab();
    pointer.release(Button::Primary);
    wait_until("the release to settle", || strip.live_tab().is_none());

    assert!(under_the_press, "the other tab slid under the point pressed");
    assert_eq!(dragged, Some(first), "the press drags the tab it met");
    assert_eq!(tab_ids(&test.window), [second, first]);
}

/// The tab in front, with keyboard focus, leaves the front while the tabs
/// are laid out live, as when it is dragged out of its row and the tab
/// beside it comes to the front: that tab takes the focus, as a strip
/// drawn anew gives it, so the arrow keys, Home and End still move along
/// the tabs.
///
/// parity: ACC-008, TAB-034
#[gtk::test]
fn the_tab_that_comes_to_the_front_during_a_drag_takes_the_focus() {
    let fixture = Fixture::standard();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let strip = test.window.tab_strip();
    let widget_of = |id| strip.tab_widget_of(id).expect("a shown tab");
    widget_of(second).grab_focus();
    assert!(widget_of(second).has_focus(), "the tab in front has focus");
    let x = grab_point(&test.window, second);

    strip.grab_tab(second, (x, 10.0));
    strip.follow_pointer(second, x + 4.0);
    strip.detach_tab(second);
    test.window.switch_tab(first);

    assert_eq!(strip.live_tab(), Some(second), "the tabs are still laid out live");
    assert!(widget_of(first).has_focus(), "the tab now in front has focus");
}

/// A point 20 pixels into tab `id`, along the strip.
fn grab_point(window: &BrowserWindow, id: TabId) -> f64 {
    let strip = window.tab_strip();
    let bounds = strip.tab_bounds_in(id, strip).expect("a shown tab");
    f64::from(bounds.x()) + 20.0
}

/// With another window open, a tab dragged out of its row is carried by
/// GTK's drag and drop, so it could land on that window's strip: it is gone
/// from its own strip, which closes up, the tab beside it comes to the
/// front, as Chrome shows the next tab, and nothing is written over the
/// file list. Brought back into its row it follows the pointer again, in
/// front again, the other tab sliding aside, and lands where it was shown.
///
/// parity: TAB-032, TAB-034
#[gtk::test]
fn a_tab_dragged_out_and_back_into_its_row_lands_where_it_was_shown() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let (other, _, _) = open_another_window_lower(&test, &fixture, &mut pointer);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let (first_tab, second_tab) = (tab_widget(&test.window, 0), tab_widget(&test.window, 1));
    let first_left = left_in_row(&test.window, &first_tab);
    let from = middle_on_screen(&mut pointer, &first_tab);
    // Below the strip, above the other window.
    let away = (from.0 + 40, from.1 + 90);
    let past = middle_on_screen(&mut pointer, &second_tab);
    let past = (past.0 + 40, past.1);

    press_and_glide(&mut pointer, from, away, Pace::Held);
    let carried = drags_and_drops(&test.window);
    let hint = test.window.folder_pane().drag_hint();
    let closed_up = left_in_row(&test.window, &second_tab);
    let gone = first_tab.has_css_class("tab-drag-source");
    let front_while_away = active_tab(&test.window);
    let shown_in_front_while_away = second_tab.has_css_class("active");
    pointer.glide(away, past, 12);
    wait_for(HOLD);
    let front_back_in_row = active_tab(&test.window);
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
    assert!(gone, "out of its strip the tab is gone from it");
    assert_eq!(
        front_while_away,
        Some(second),
        "the tab beside it comes to the front"
    );
    assert!(shown_in_front_while_away, "and the strip shows it in front");
    assert_eq!(
        front_back_in_row,
        Some(first),
        "back in its row the tab is in front again"
    );
    assert_eq!(active_tab(&test.window), Some(first));
    assert!(
        (closed_up - first_left).abs() <= 0.5,
        "the strip closed up where it was: {closed_up}"
    );
    assert_eq!(hint, None, "nothing is written over the file list");
    assert!(
        (other_back_in_row - first_left).abs() <= 0.5,
        "back in the row the other tab made room: {other_back_in_row}"
    );
    assert!(
        windows_besides(&[&test.window, &other.window]).is_empty(),
        "no window opens"
    );
}

/// A carried tab whose drag is called off, as Escape does, stays where it
/// was, in front again.
///
/// parity: TAB-034, ACC-010
#[gtk::test]
fn a_carried_tab_whose_drag_is_called_off_is_in_front_again() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 0));
    let away = (from.0 + 40, from.1 + 90);

    press_and_glide(&mut pointer, from, away, Pace::Held);
    let front_while_away = active_tab(&test.window);
    tab_drag_source(&test.window).drag_cancel();
    wait_until("the drag to end", || !drags_and_drops(&test.window));
    pointer.release(Button::Primary);
    wait_for(HOLD);

    assert_eq!(front_while_away, Some(second));
    assert_eq!(
        tab_ids(&test.window),
        [first, second],
        "the tab stays where it was"
    );
    assert_eq!(active_tab(&test.window), Some(first), "in front again");
    assert!(
        tab_widget(&test.window, 0).has_css_class("active"),
        "and shown in front"
    );
    assert!(windows_besides(&[&test.window]).is_empty(), "no window opens");
}

/// A carried tab whose drag is called off comes back with the search its
/// window showed: the tab beside it, in front meanwhile, emptied the search
/// box, which the tabs share.
///
/// parity: TAB-034, SRCH-001
#[gtk::test]
fn a_carried_tab_whose_drag_is_called_off_keeps_its_search() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    test.window.switch_tab(first);
    test.start_search_cache();
    test.search_for("notes");
    let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 0));
    let away = (from.0 + 40, from.1 + 90);

    press_and_glide(&mut pointer, from, away, Pace::Held);
    let front_while_away = active_tab(&test.window);
    let box_while_away = test.window.search_box().entry().text();
    tab_drag_source(&test.window).drag_cancel();
    wait_until("the drag to end", || !drags_and_drops(&test.window));
    pointer.release(Button::Primary);
    wait_until("the search to run again", || {
        !test.status_count().starts_with("Searching") && test.names() == ["Notes 2.txt", "Notes 10.txt"]
    });

    assert_eq!(front_while_away, Some(second));
    assert_eq!(box_while_away, "", "the tab beside it emptied the search box");
    assert_eq!(active_tab(&test.window), Some(first), "in front again");
    assert_eq!(
        test.window.search_box().entry().text(),
        "notes",
        "with its search"
    );
}

/// The drag source of `window`'s tabs, on its title bar.
fn tab_drag_source(window: &BrowserWindow) -> gtk::DragSource {
    let bar = window.titlebar().expect("a title bar");
    std::iter::once(bar.clone())
        .chain(crate::test_support::harness::descendants::<gtk::Widget>(&bar))
        .flat_map(|widget| {
            widget
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|controller| controller.downcast::<gtk::DragSource>().ok())
                .collect::<Vec<_>>()
        })
        .next()
        .expect("the title bar has the tabs' drag source")
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
    let (other, other_before, other_after) = open_another_window_lower(&test, &fixture, &mut pointer);
    let other_first = tab_ids(&other.window)[0];
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
    // Past the middle of the other window's tab, so the tab's left edge,
    // held where it was grabbed, comes before that middle.
    let to = on_screen(
        &mut pointer,
        &target,
        (f64::from(target.width()) * 0.7, f64::from(target.height()) / 2.0),
    );
    let tab_corner = border_corner_on_screen(&mut pointer, &tab_widget(&test.window, 1));
    press_and_glide(&mut pointer, from, to, Pace::Brisk);
    let made_room = left_in_row(&other.window, &target) - target_left;
    let held_at = test
        .window
        .carried_look_for_test()
        .map(|(_, _, _, held_at)| held_at);
    let stand_in = other
        .window
        .tab_strip()
        .tab_list()
        .last_child()
        .expect("the stand-in, drawn last");
    let stand_in_left = border_left_in(other_after, &stand_in);
    pointer.release(Button::Primary);
    wait_until("the tab to leave", || test.window.tab_count() == 1);

    assert!(
        (made_room - (width + 2.0)).abs() <= 0.5,
        "the other window's tabs made room for the tab: {made_room}"
    );
    assert!(
        held_at.is_some_and(|(_, y)| y > 10_000),
        "over the other strip the carried tab hides: {held_at:?}"
    );
    assert_eq!(
        stand_in_left,
        to.0 - (from.0 - tab_corner.0),
        "a stand-in for the tab follows the pointer in that strip, held where it was grabbed"
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

/// A press on a tab shows it at once, before any drag, as Chrome does, and
/// the strip keeps the pressed tab's widget, so one tab looks in front and
/// the press goes on.
///
/// parity: TAB-004, TAB-032
#[gtk::test]
fn a_press_shows_the_tab_at_once_and_keeps_its_widget() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let [first, second] = tab_ids(&test.window)[..] else {
        panic!("two tabs are open");
    };
    let pressed = tab_widget(&test.window, 0);
    let at = middle_on_screen(&mut pointer, &pressed);

    pointer.move_to(at.0, at.1);
    pointer.press(Button::Primary);
    wait_for(HOLD);
    let in_front_while_pressed = active_tab(&test.window);
    let same_widget = test.window.tab_strip().tab_widget_of(first);
    let fronts: Vec<bool> = [first, second]
        .iter()
        .map(|id| {
            test.window
                .tab_strip()
                .tab_widget_of(*id)
                .is_some_and(|tab| tab.has_css_class("active"))
        })
        .collect();
    pointer.release(Button::Primary);

    assert_eq!(in_front_while_pressed, Some(first), "shown on the press");
    assert_eq!(same_widget.as_ref(), Some(&pressed), "the pressed widget stays");
    assert_eq!(fronts, [true, false], "only the pressed tab looks in front");
}

/// A tab dragged out of its row is carried as itself, as Chromium carries
/// it where the desktop cannot move a real window with it: the tab, as it
/// looks in front, follows the pointer held where it was grabbed, and no
/// window opens until the release, which opens one with the tab.
///
/// parity: TAB-034
#[gtk::test]
fn a_tab_dragged_out_of_its_row_is_carried_as_itself_until_released() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let tab = tab_widget(&test.window, 1);
    let tab_corner = border_corner_on_screen(&mut pointer, &tab);
    let tab_width = border_width(&tab);
    let from = middle_on_screen(&mut pointer, &tab);
    let out = (from.0 + 20, from.1 + 80);

    press_and_glide(&mut pointer, from, out, Pace::Held);
    let carried = drags_and_drops(&test.window);
    let opened_while_pressed = windows_besides(&[&test.window]).len();
    let look = test.window.carried_look_for_test();
    let carried_tab = test.window.carried_tab_for_test().expect("a carried tab");
    let (carried_close, close) = (close_button_of(&carried_tab), close_button_of(&tab));
    let close_drawn_alike = drawn(&test.window, &carried_close) == drawn(&test.window, &close);
    pointer.release(Button::Primary);
    let opened = OpenedWindows::only(&[&test.window]);

    assert!(carried, "the tab is carried by drag and drop");
    assert_eq!(opened_while_pressed, 0, "no window opens before the release");
    let (title, classes, width, held_at) = look.expect("the tab itself follows the pointer");
    assert_eq!(title, "Documents");
    assert!(
        classes.contains(&"tab".to_owned()) && classes.contains(&"active".to_owned()),
        "{classes:?}"
    );
    assert_eq!(width, tab_width, "as wide as the tab");
    assert_eq!(
        held_at,
        (from.0 - tab_corner.0, from.1 - tab_corner.1),
        "held where it was grabbed"
    );
    assert!(
        close_drawn_alike,
        "its close button is drawn as the tab's, though GTK's drag icon is no window"
    );
    assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 1, "the tab left this window");
}

/// The close button of `tab`.
fn close_button_of(tab: &gtk::Widget) -> gtk::Button {
    crate::test_support::harness::descendants::<gtk::Button>(tab)
        .into_iter()
        .find(|button| button.has_css_class("tab-close"))
        .expect("a tab has a close button")
}

/// The size and pixels of `widget` as it draws itself, rendered by
/// `window`'s renderer.
fn drawn(window: &BrowserWindow, widget: &impl IsA<gtk::Widget>) -> (i32, i32, Vec<u8>) {
    let (width, height) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let Some(node) = snapshot.to_node() else {
        return (width, height, Vec::new());
    };
    let renderer = window.renderer().expect("a drawn window");
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    let stride = usize::try_from(texture.width()).unwrap_or(0) * 4;
    let mut pixels = vec![0; stride * usize::try_from(texture.height()).unwrap_or(0)];
    texture.download(&mut pixels, stride);
    (width, height, pixels)
}

/// The top left corner of `widget`'s border box on the screen: widget
/// coordinates start inside its padding.
fn border_corner_on_screen(pointer: &mut Pointer, widget: &gtk::Widget) -> (i32, i32) {
    let bounds = widget.compute_bounds(widget).expect("a shown widget");
    on_screen(pointer, widget, (f64::from(bounds.x()), f64::from(bounds.y())))
}

/// The left edge of `widget`'s border box on the screen, its window being
/// at `origin`, found without moving the pointer, which moves a drag.
fn border_left_in(origin: (i32, i32), widget: &gtk::Widget) -> i32 {
    let native = widget.native().expect("a shown widget is in a window");
    let bounds = widget.compute_bounds(&native).expect("a laid out widget");
    let (shadow_x, _) = native.surface_transform();
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let left = (f64::from(bounds.x()) + shadow_x).round() as i32;
    origin.0 + left
}

/// The width of `widget`'s border box, as drawn.
fn border_width(widget: &gtk::Widget) -> i32 {
    let bounds = widget.compute_bounds(widget).expect("a shown widget");
    #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
    let width = bounds.width().round() as i32;
    width
}

/// A tab flicked out of its row and released before GTK may start a drag
/// and drop (100 ms after the press) tears out as a release out there does
/// once it has started, and the strip does not stay live. The press and
/// both moves reach the window together, as from a hand, so a busy main
/// loop cannot let GTK's 100 ms pass between them.
///
/// parity: TAB-032, TAB-034
#[gtk::test]
fn a_tab_released_out_of_its_row_before_drag_and_drop_starts_tears_out() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let tab = tab_widget(&test.window, 1);
    let from = middle_on_screen(&mut pointer, &tab);
    let strip_height = test.window.tab_strip().height();

    pointer.move_to(from.0, from.1);
    // Past the drag threshold, then well below the strip, at once.
    pointer.press_and_flick(
        Button::Primary,
        &[(from.0 + 12, from.1), (from.0 + 14, from.1 + strip_height + 40)],
    );
    let started = drags_and_drops(&test.window);
    pointer.release(Button::Primary);
    let opened = OpenedWindows::only(&[&test.window]);

    assert!(!started, "released before drag and drop could start");
    assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 1, "the tab left this window");
    assert_eq!(test.window.tab_strip().live_tab(), None, "the strip is still");
}

/// A tab dragged along its row while its window becomes busy, as a paste
/// may make it, and then out of the row, starts no drag and drop: it goes
/// back where it was, and the strip does not stay live, so the tab may
/// close and the strip shows changes again.
///
/// parity: TAB-031, TAB-032
#[gtk::test]
fn a_tab_whose_window_became_busy_while_it_followed_the_pointer_goes_back_on_leaving_the_row() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    let order = tab_ids(&test.window);
    let tab = tab_widget(&test.window, 1);
    let from = middle_on_screen(&mut pointer, &tab);
    let along = (from.0 - 30, from.1);
    let out = (along.0, along.1 + 80);

    press_and_glide(&mut pointer, from, along, Pace::Held);
    let was_live = test.window.tab_strip().live_tab();
    assert!(test.window.begin_test_write(), "the window is busy");
    pointer.glide(along, out, 6);
    wait_for(HOLD);
    let carried = drags_and_drops(&test.window);
    pointer.release(Button::Primary);
    wait_until("the strip is still", || {
        test.window.tab_strip().live_tab().is_none()
    });
    test.window.end_test_write();

    assert_eq!(was_live, order.get(1).copied(), "the tab followed the pointer");
    assert!(!carried, "a busy window starts no drag and drop");
    assert_eq!(tab_ids(&test.window), order, "every tab is where it was");
    assert!(windows_besides(&[&test.window]).is_empty());
}

/// The only tab of a window, dragged out of its row, takes the window
/// itself along, as Chrome moves a window by its only tab.
///
/// parity: TAB-034, TAB-048
#[gtk::test]
fn the_only_tab_dragged_out_of_its_row_moves_its_window() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 0));
    let out = (from.0 + 20, from.1 + 80);

    press_and_glide(&mut pointer, from, out, Pace::Held);
    let at_first = origin_under(&test.window, out);
    let further = (out.0 + 90, out.1 + 60);
    pointer.glide(out, further, 6);
    wait_for(HOLD);
    let at_last = origin_under(&test.window, further);
    pointer.release(Button::Primary);
    wait_for(HOLD);

    let (Some(first), Some(last)) = (at_first, at_last) else {
        panic!("the window is under the pointer: {at_first:?} {at_last:?}");
    };
    assert_eq!(
        (last.0 - first.0, last.1 - first.1),
        (90, 60),
        "it moved with the pointer"
    );
    assert!(
        windows_besides(&[&test.window]).is_empty(),
        "no other window opens"
    );
    assert_eq!(test.window.tab_count(), 1);
}

/// `window` narrowed to `width` pixels, laid out.
fn narrowed(window: &BrowserWindow, width: i32) {
    window.set_default_size(width, 600);
    wait_until("the window to narrow", || window.width() <= width);
    crate::test_support::harness::wait_for_frames(window, 3);
}

/// A tab dragged straight sideways out of its window onto a window beside
/// it, whose row lines up with its own, leaves its row and moves into that
/// window, as one dragged down and over does.
///
/// parity: TAB-033, TAB-034
#[gtk::test]
fn a_tab_dragged_sideways_onto_a_window_beside_moves_there() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = two_tabs(&fixture);
    narrowed(&test.window, 680);
    let other = test.open_beside(&fixture.uri());
    narrowed(&other.window, 680);
    // The new window covers the first: drag it right of it by its blank
    // title bar, so the two stand side by side, their rows lined up.
    let blank =
        crate::test_support::harness::descendants::<gtk::Box>(&other.window.titlebar().expect("a title bar"))
            .into_iter()
            .find(|child| child.has_css_class("title-drag"))
            .expect("the bar has its blank drag area");
    let grab = middle_on_screen(&mut pointer, &blank);
    let (right, _) = on_screen(&mut pointer, &test.window, (f64::from(test.window.width()), 0.0));
    let (other_left, _) = on_screen(&mut pointer, &other.window, (0.0, 0.0));
    drag(
        &mut pointer,
        grab,
        (grab.0 + right - other_left + 20, grab.1),
        Pace::Held,
    );
    wait_for(HOLD);
    let other_first = tab_ids(&other.window)[0];
    let from = middle_on_screen(&mut pointer, &tab_widget(&test.window, 1));
    let target = tab_widget(&other.window, 0);
    let to = on_screen(
        &mut pointer,
        &target,
        (f64::from(target.width()) * 0.3, f64::from(target.height()) / 2.0),
    );
    assert!(
        to.0 > right,
        "the other window is beside this one: {to:?}, {right}"
    );
    assert!((to.1 - from.1).abs() <= 2, "the rows line up: {from:?}, {to:?}");

    press_and_glide(&mut pointer, from, to, Pace::Held);
    pointer.release(Button::Primary);
    wait_until("the tab to leave", || test.window.tab_count() == 1);

    assert_eq!(other.window.tab_count(), 2);
    assert_eq!(other.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(
        tab_ids(&other.window)[1],
        other_first,
        "the tab went before the first"
    );
}

/// A tab dragged along a row whose tabs do not all fit, held at its
/// visible right edge, scrolls the row there and goes on following the
/// pointer as the row moves under it, so it reaches the end, out of sight
/// when the drag began, and lands there.
///
/// parity: TAB-032
#[gtk::test]
fn a_tab_held_at_the_edge_of_a_row_that_does_not_fit_scrolls_it_and_lands_there() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let strip = test.window.tab_strip();
    let scroll = strip
        .tab_list()
        .parent()
        .and_downcast::<gtk::Viewport>()
        .and_then(|viewport| viewport.hadjustment())
        .expect("the row scrolls sideways");
    for _ in 0..60 {
        if scroll.upper() > scroll.page_size() + 300.0 {
            break;
        }
        test.window.add_tab(&fixture.uri()).expect("a folder");
        test.wait_for_listing("another tab");
        crate::test_support::harness::wait_for_frames(&test.window, 2);
    }
    scroll.set_value(0.0);
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    let first = tab_ids(&test.window)[0];
    let x = grab_point(&test.window, first);
    let edge = f64::from(strip.width()) - 5.0;
    let landed = Rc::new(Cell::new(None));

    strip.grab_tab(first, (x, 10.0));
    strip.follow_pointer(first, x + 40.0);
    strip.follow_pointer(first, edge);
    wait_until("the row to scroll to its end", || {
        scroll.value() >= scroll.upper() - scroll.page_size() - 0.5
    });
    // It follows the pointer to the visible end of the row, once laid out.
    let drawn_right = || {
        let drawn = strip.tab_bounds_in(first, strip).expect("a shown tab");
        f64::from(drawn.x() + drawn.width())
    };
    wait_until("the tab to reach the visible end of the row", || {
        (drawn_right() - f64::from(strip.width())).abs() <= 2.0
    });
    strip.let_go(
        true,
        glib::clone!(
            #[strong]
            landed,
            move |moved| landed.set(Some(moved))
        ),
    );
    wait_until("the tab to land", || landed.get().is_some());

    assert_eq!(landed.get(), Some(Some((first, None))), "it landed last");
}
