// SPDX-License-Identifier: AGPL-3.0-only
//! Split view and how folders open into tabs: F3, the two panes, a folder
//! opened with modifiers held, and folders dropped beside the tabs.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::test_support::harness::{capture, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::PaneSide;
use crate::window::WindowAction;

/// F3 opens a second pane at the same folder, in front; each pane keeps
/// its own folder, the address follows the active one, and F3 again
/// closes the active pane.
///
/// parity: VIEW-059
#[gtk::test]
fn each_pane_keeps_its_folder_and_the_window_follows_the_active_one() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("split-view", None);
    let split_state = test.window.window_action_state(WindowAction::SplitView);
    let opened_on = test.window.active_side();
    test.window.navigate_or_report(&fixture.uri_of("Documents"));
    test.wait_for_listing("Documents in the right pane");
    test.window.activate_pane(PaneSide::Start);
    capture(&test.window, "native-split-view.png");
    let left_names = test.names();
    let right_count = test.window.pane_on(PaneSide::End).model().n_items();

    assert_eq!(split_state.and_then(|state| state.get::<bool>()), Some(true));
    assert_eq!(opened_on, PaneSide::End, "the new pane is in front");
    assert!(test.window.imp().end_pane_column.is_visible());
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(left_names, STANDARD_NAMES);
    assert_eq!(right_count, 0, "Documents is empty");
    assert_eq!(test.window.tab_count(), 1, "both panes are one tab");

    test.activate("split-view", None);

    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert!(!test.window.imp().end_pane_column.is_visible());
    assert_eq!(test.window.active_side(), PaneSide::Start);
}

/// A folder opened with Ctrl held goes to a new tab behind the current
/// one, which stays where it is.
///
/// parity: TAB-026
#[gtk::test]
fn ctrl_opens_a_folder_in_a_new_tab_behind() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.window
        .imp()
        .view_modifiers
        .set(Some(gdk::ModifierType::CONTROL_MASK));
    test.window.activate_from_view(test.position_of("Documents"));

    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// Folders dropped on the tab strip beside the tabs open as new tabs at
/// the end; files dropped with them do not.
///
/// parity: TAB-018
#[gtk::test]
fn folders_dropped_beside_the_tabs_open_as_new_tabs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let items = vec![fixture.uri_of("Documents"), fixture.uri_of("Notes 2.txt")];

    glib::MainContext::default().block_on(test.window.open_dropped_folders(items));
    wait_until("the dropped folder's tab", || test.window.tab_count() == 2);

    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the tab opens behind"
    );
    test.activate("next-tab", None);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}
