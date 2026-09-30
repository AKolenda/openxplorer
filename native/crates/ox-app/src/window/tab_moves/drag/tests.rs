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

/// parity: TAB-034, TAB-035
#[gtk::test]
fn only_the_source_window_well_below_its_strip_tears_a_tab_out() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    let other = test.open_beside(&fixture.uri());
    let bottom = f64::from(test.window.title_bar_bottom().expect("a laid out title bar"));
    let dragged = start_dragging(&test.window, 1);

    let in_the_gap = test.window.tab_drop_spot(&test.window, 300.0, bottom + 20.0);
    let well_below = test.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
    let in_another_window = other.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
    let taken = test
        .window
        .settle_tab_drag(dragged, &test.window, TabDropSpot::TearOut);
    test.window.end_tab_drag();
    let opened = OpenedWindows::only(&[&test.window, &other.window]);

    assert_eq!(in_the_gap, None);
    assert_eq!(well_below, Some(TabDropSpot::TearOut));
    assert_eq!(in_another_window, None, "another window's body refuses the tab");
    assert!(taken);
    assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 1);
}

/// How a drag ends without a window of this app taking the tab.
struct UnclaimedEnd {
    /// The cancel reason GTK reports first, or `None` when the drop
    /// was performed (GNOME Shell's desktop drop).
    cancel: Option<gdk::DragCancelReason>,
    /// A window of this app refused the tab where it was released.
    refused_here: bool,
    /// The tab tears out into a new window.
    tears_out: bool,
}

/// parity: TAB-034, TAB-035, TAB-036
#[gtk::test]
fn a_tab_released_outside_every_window_tears_out_and_a_cancelled_one_stays() {
    let cases = [
        UnclaimedEnd {
            cancel: None,
            refused_here: false,
            tears_out: true,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::NoTarget),
            refused_here: false,
            tears_out: true,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::NoTarget),
            refused_here: true,
            tears_out: false,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::UserCancelled),
            refused_here: false,
            tears_out: false,
        },
        UnclaimedEnd {
            cancel: Some(gdk::DragCancelReason::Error),
            refused_here: false,
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

        test.window.note_tab_refusal(case.refused_here);
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
            assert_eq!(test.window.shown_message(), TabMoveRefusal::Cancelled.to_string());
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

/// parity: TAB-032, TAB-034
#[gtk::test]
fn a_tab_drag_shows_where_the_tab_would_go() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let [_, second, third] = tab_ids(&test.window)[..] else {
        panic!("three tabs are open");
    };
    let dragged = start_dragging(&test.window, 0);
    test.window.tab_strip().show_dragged_tab(Some(dragged));

    test.window
        .show_tab_drop_spot(Some(TabDropSpot::Strip { before: Some(second) }));
    let before_second = classes_of(&test.window, second);
    let strip_marked = test.window.tab_strip().has_css_class("tab-drop-active");
    let fades_the_dragged_tab = classes_of(&test.window, dragged);
    test.window
        .show_tab_drop_spot(Some(TabDropSpot::Strip { before: None }));
    let after_last = classes_of(&test.window, third);
    test.window.show_tab_drop_spot(Some(TabDropSpot::TearOut));
    let tear_out_hint = test.window.folder_pane().drag_hint();
    test.window.end_tab_drag();

    assert!(before_second.contains(&"tab-insert-before".to_owned()));
    assert!(strip_marked);
    assert!(fades_the_dragged_tab.contains(&"tab-drag-source".to_owned()));
    assert!(after_last.contains(&"tab-insert-after".to_owned()));
    assert_eq!(tear_out_hint.as_deref(), Some(TEAR_OUT_HINT));
    assert!(
        !test.window.tab_strip().has_css_class("tab-drop-active"),
        "the marks go with the drag"
    );
    assert_eq!(test.window.folder_pane().drag_hint(), None);
    assert!(!classes_of(&test.window, dragged).contains(&"tab-drag-source".to_owned()));
}

/// The style classes of tab `id` of `window`.
fn classes_of(window: &BrowserWindow, id: TabId) -> Vec<String> {
    let classes = window.tab_strip().tab_classes();
    classes
        .into_iter()
        .find_map(|(tab, classes)| (tab == id).then_some(classes))
        .expect("the tab is shown")
}

/// With `OX_NATIVE_CAPTURE_DIR` set, saves a tab drag's insertion mark
/// and its tear-out note in both themes.
#[gtk::test]
fn a_tab_drags_marks_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let second = tab_ids(&test.window)[1];
    let dragged = start_dragging(&test.window, 0);
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        // A new theme draws the tabs anew; a drag's next motion would
        // mark the new ones.
        wait_for_frames(&test.window, 3);
        test.window.tab_strip().show_dragged_tab(Some(dragged));
        test.window
            .show_tab_drop_spot(Some(TabDropSpot::Strip { before: Some(second) }));
        capture(&test.window, &format!("native-tab-drag-insert-{theme}.png"));
        test.window.show_tab_drop_spot(Some(TabDropSpot::TearOut));
        capture(&test.window, &format!("native-tab-drag-tear-out-{theme}.png"));
    }
    test.window.end_tab_drag();
}
