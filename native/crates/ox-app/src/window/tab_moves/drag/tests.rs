// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of dragging tabs.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::*;
use crate::test_support::harness::{
    capture, settle, wait_for, wait_for_frames, wait_until, windows_besides, Fixture, OpenedWindows,
    TestWindow, ThemeGuard,
};
use crate::window::tab_moves::TabMoveRefusal;

/// parity: TAB-040
#[test]
fn a_tab_drag_offers_the_tab_to_this_process_only() {
    let content = tab_drag_content(&DraggedTab {
        source: glib::WeakRef::new(),
        tab: TabId::from_raw(1),
    });

    let formats = content.formats();

    assert!(formats.contains_type(DraggedTab::static_type()));
    let mime_types: Vec<String> = formats.mime_types().iter().map(ToString::to_string).collect();
    assert_eq!(mime_types, [ROOT_WINDOW_DROP], "no file, text or location format");
    let served = formats.union_serialize_mime_types();
    let served: Vec<String> = served.mime_types().iter().map(ToString::to_string).collect();
    assert_eq!(
        served,
        [ROOT_WINDOW_DROP],
        "GTK serializes nothing else for other apps"
    );
}

fn tab_ids(window: &BrowserWindow) -> Vec<TabId> {
    let session = window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

/// The tab widget at `index` of `window`'s strip.
fn tab_widget(window: &BrowserWindow, index: usize) -> gtk::Widget {
    crate::window::widget_tree::children(&window.tab_strip().tab_list())
        .nth(index)
        .expect("the window shows the tab")
}

/// The middle of `widget` in `ancestor`'s coordinates.
fn middle_in(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let bounds = widget
        .compute_bounds(ancestor)
        .expect("a shown widget has bounds");
    let x = bounds.x() + bounds.width() / 2.0;
    let y = bounds.y() + bounds.height() / 2.0;
    (f64::from(x), f64::from(y))
}

/// Starts dragging the tab at `index`, as a press and a move there do;
/// the dragged tab.
fn start_dragging(window: &BrowserWindow, index: usize) -> TabId {
    wait_for_frames(window, 3);
    let (x, y) = middle_in(&tab_widget(window, index), window.tab_strip());
    window.prepare_tab_drag(x, y).expect("the tab can be dragged");
    let outgoing = window.imp().outgoing_tab.borrow();
    outgoing
        .as_ref()
        .map(|outgoing| outgoing.tab)
        .expect("the drag is prepared")
}

/// A window with tabs on `fixture`, its Documents folder and
/// `fixture` again.
fn three_tabs(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    for uri in [fixture.uri_of("Documents"), fixture.uri()] {
        test.window.add_tab(&uri).expect("a folder");
        test.wait_for_listing("the new tab");
    }
    test
}

/// parity: TAB-032
#[gtk::test]
fn a_tab_dropped_on_its_own_strip_goes_before_the_tab_right_of_the_pointer() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let [first, second, third] = tab_ids(&test.window)[..] else {
        panic!("three tabs are open");
    };
    let dragged = start_dragging(&test.window, 0);
    let past_the_last = tab_widget(&test.window, 2)
        .compute_bounds(&test.window)
        .expect("a shown tab");
    let x = f64::from(past_the_last.x() + past_the_last.width() - 2.0);
    let (_, y) = middle_in(&tab_widget(&test.window, 2), &test.window);

    let spot = test.window.tab_drop_spot(&test.window, x, y);
    let taken = spot.is_some_and(|spot| test.window.settle_tab_drag(dragged, &test.window, spot));
    test.window.end_tab_drag();
    settle();

    assert_eq!(spot, Some(TabDropSpot::Strip { before: None }));
    assert!(taken);
    assert_eq!(tab_ids(&test.window), [second, third, first]);
    assert!(
        windows_besides(&[&test.window]).is_empty(),
        "reordering opens no window"
    );
}

/// parity: TAB-033, TAB-039
#[gtk::test]
fn a_tab_dropped_on_another_windows_strip_moves_there_and_leaves_after_the_drag() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    let other = test.open_beside(&fixture.uri());
    let dragged = start_dragging(&test.window, 1);
    wait_for_frames(&other.window, 3);
    let (x, y) = middle_in(&tab_widget(&other.window, 0), &other.window);
    let other_first = tab_ids(&other.window)[0];

    let spot = other.window.tab_drop_spot(&test.window, f64::min(x, 20.0), y);
    let taken = spot.is_some_and(|spot| test.window.settle_tab_drag(dragged, &other.window, spot));
    let kept_while_dragging = test.window.tab_count();
    test.window.end_tab_drag();
    wait_until("the original to go", || test.window.tab_count() == 1);

    assert_eq!(
        spot,
        Some(TabDropSpot::Strip {
            before: Some(other_first)
        })
    );
    assert!(taken);
    assert_eq!(kept_while_dragging, 2, "the original stays until the drag ends");
    assert_eq!(other.window.tab_count(), 2);
    assert_eq!(other.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(
        tab_ids(&other.window)[1],
        other_first,
        "the tab went before the first"
    );
}

/// Released anywhere in a window of this app outside its tab row, in the
/// source window just below its strip or in another window's file list, a
/// tab tears out, as in Chrome; a busy window refuses it.
///
/// parity: TAB-034, TAB-035, TAB-037
#[gtk::test]
fn outside_every_tab_row_a_dropped_tab_tears_out() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    let other = test.open_beside(&fixture.uri());
    let bottom = f64::from(test.window.title_bar_bottom().expect("a laid out title bar"));
    let dragged = start_dragging(&test.window, 1);

    let just_below = test.window.tab_drop_spot(&test.window, 300.0, bottom + 5.0);
    let well_below = test.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
    let in_another_window = other.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
    let operation = other.window.begin_operation("Preparing copy…");
    let in_a_busy_window = other.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
    other.window.end_operation();
    let taken = test
        .window
        .settle_tab_drag(dragged, &test.window, TabDropSpot::TearOut);
    test.window.end_tab_drag();
    let opened = OpenedWindows::only(&[&test.window, &other.window]);

    assert_eq!(just_below, Some(TabDropSpot::TearOut));
    assert_eq!(well_below, Some(TabDropSpot::TearOut));
    assert_eq!(in_another_window, Some(TabDropSpot::TearOut));
    assert!(operation.is_some());
    assert_eq!(in_a_busy_window, None, "a busy window refuses the tab");
    assert!(taken);
    assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 1);
}

/// How a drag ends without a window of this app taking the tab.
struct UnclaimedEnd {
    /// The cancel reason GTK reports first, or `None` when the drop
    /// was performed (GNOME Shell's desktop drop).
    cancel: Option<gdk::DragCancelReason>,
    /// The window of this app under the pointer at the end, `Some(true)`
    /// when it takes the tab, or `None` outside every one.
    over: Option<bool>,
    /// The tab tears out into a new window.
    tears_out: bool,
}

/// A tab released outside every window of this app tears out, as in
/// Chrome, whether GNOME Shell takes it as a desktop drop, X11 reports no
/// target or Wayland cancels the drag. Escape keeps it, and so does a
/// busy window that refused it, which says so. Wayland reports Escape like
/// a release outside, so Escape keeps the tab there only while the pointer
/// is over a window of this app.
///
/// parity: TAB-034, TAB-035, TAB-036
#[gtk::test]
fn a_tab_released_outside_every_window_tears_out_and_a_cancelled_one_stays() {
    let cases = [
        UnclaimedEnd {
            cancel: None,
            over: None,
            tears_out: true,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::NoTarget),
            over: None,
            tears_out: true,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::Error),
            over: None,
            tears_out: true,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::NoTarget),
            over: Some(false),
            tears_out: false,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::Error),
            over: Some(true),
            tears_out: false,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::UserCancelled),
            over: None,
            tears_out: false,
        },
    ];
    let fixture = Fixture::standard();
    for case in cases {
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let dragged = start_dragging(&test.window, 1);

        test.window.note_tab_hover(case.over);
        if let Some(reason) = case.cancel {
            test.window.cancel_tab_drag(reason);
        }
        test.window.end_tab_drag();
        let late_drop = test
            .window
            .settle_tab_drag(dragged, &test.window, TabDropSpot::TearOut);

        assert!(!late_drop, "a drop after the drag ended takes nothing");
        if case.tears_out {
            let opened = OpenedWindows::only(&[&test.window]);
            assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
            assert_eq!(test.window.tab_count(), 1);
        } else {
            wait_for(std::time::Duration::from_millis(100));
            assert!(windows_besides(&[&test.window]).is_empty());
            assert_eq!(test.window.tab_count(), 2, "the tab stays");
            let refused = case.over == Some(false);
            let message = test.window.shown_message();
            if refused {
                assert_eq!(message, TabMoveRefusal::DestinationBusy.to_string());
            } else {
                assert_ne!(message, TabMoveRefusal::DestinationBusy.to_string());
            }
        }
    }
}

/// parity: TAB-003
#[gtk::test]
fn a_dragged_tab_neither_closes_nor_navigates() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    start_dragging(&test.window, 1);

    test.activate("close-tab", None);
    test.window.navigate(&fixture.uri()).expect("a folder");
    let message = test.window.shown_message();
    test.window.cancel_tab_drag(gdk::DragCancelReason::UserCancelled);
    test.window.end_tab_drag();

    assert_eq!(message, "Wait for this tab to finish moving.");
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.activate("close-tab", None);
    assert_eq!(test.window.tab_count(), 1, "the tab closes once the drag ended");
}

/// parity: TAB-031
#[gtk::test]
fn no_tab_drag_starts_from_a_close_button_or_while_the_window_is_busy() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let tab = tab_widget(&test.window, 0);
    let close = tab.last_child().expect("the tab has a close button");
    let (close_x, close_y) = middle_in(&close, test.window.tab_strip());
    let (x, y) = middle_in(&tab, test.window.tab_strip());

    let from_close_button = test.window.prepare_tab_drag(close_x, close_y);
    let operation = test.window.begin_operation("Preparing copy…");
    let while_busy = test.window.prepare_tab_drag(x, y);
    let busy_message = test.window.shown_message();
    test.window.end_operation();

    assert!(from_close_button.is_none());
    assert!(operation.is_some());
    assert!(while_busy.is_none());
    assert_eq!(busy_message, TabMoveRefusal::SourceBusy.to_string());
    assert!(
        test.window.prepare_tab_drag(x, y).is_some(),
        "a tab drags once the operation ended"
    );
}

/// Over another window's strip the tabs open a gap as wide as the dragged
/// tab where it would land; nothing is written over the window, and
/// away from the strip the gap closes.
///
/// parity: TAB-033, TAB-034
#[gtk::test]
fn a_tab_over_another_windows_strip_opens_a_gap_and_nothing_is_written() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let other = test.open_beside(&fixture.uri());
    other
        .window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    other.wait_for_listing("the other window's second tab");
    wait_for_frames(&other.window, 3);
    let dragged = start_dragging(&test.window, 0);
    let width = test.window.tab_strip().tab_width(dragged).expect("a shown tab");
    let second = tab_widget(&other.window, 1);
    let second_left = left_in_row(&other.window, &second);
    let (x, _) = middle_in(&second, other.window.tab_strip());

    other.window.show_tab_drop_spot(
        &test.window,
        dragged,
        Some(TabDropSpot::Strip { before: None }),
        x - 20.0,
    );
    wait_for(std::time::Duration::from_millis(300));
    let opened_left = left_in_row(&other.window, &second);
    other
        .window
        .show_tab_drop_spot(&test.window, dragged, Some(TabDropSpot::TearOut), 300.0);
    let hint_away = other.window.folder_pane().drag_hint();
    wait_for(std::time::Duration::from_millis(300));
    let closed_left = left_in_row(&other.window, &second);
    test.window.cancel_tab_drag(gdk::DragCancelReason::UserCancelled);
    test.window.end_tab_drag();

    assert!(
        (opened_left - (second_left + width + 2.0)).abs() < 1.0,
        "the tab after the gap moved by the dragged tab's width: {second_left} -> {opened_left}"
    );
    assert_eq!(hint_away, None, "no text over the file list");
    assert!((closed_left - second_left).abs() < 1.0, "the gap closed");
}

/// The left edge of `tab` in `window`'s row of tabs, where it is drawn.
fn left_in_row(window: &BrowserWindow, tab: &gtk::Widget) -> f64 {
    let bounds = tab
        .compute_bounds(&window.tab_strip().tab_list())
        .expect("a shown tab");
    f64::from(bounds.x())
}

/// With `OX_NATIVE_CAPTURE_DIR` set, saves a strip that opened a gap for a
/// tab from another window, in both themes.
#[gtk::test]
fn a_tab_drags_gap_is_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let other = test.open_beside(&fixture.uri());
    let dragged = start_dragging(&test.window, 0);
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        // A new theme draws the tabs anew.
        wait_for_frames(&other.window, 3);
        other.window.show_tab_drop_spot(
            &test.window,
            dragged,
            Some(TabDropSpot::Strip { before: None }),
            10.0,
        );
        wait_for(std::time::Duration::from_millis(300));
        capture(&other.window, &format!("native-tab-drag-gap-{theme}.png"));
        other
            .window
            .show_tab_drop_spot(&test.window, dragged, Some(TabDropSpot::TearOut), 10.0);
        wait_for(std::time::Duration::from_millis(300));
    }
    test.window.cancel_tab_drag(gdk::DragCancelReason::UserCancelled);
    test.window.end_tab_drag();
}
