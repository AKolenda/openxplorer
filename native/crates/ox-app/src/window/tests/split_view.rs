// SPDX-License-Identifier: AGPL-3.0-only
//! Split view and how folders open into tabs: F3, the two panes, a folder
//! opened with modifiers held, and folders dropped beside the tabs.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::test_support::harness::{capture, settle, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::{PaneSide, TabId};
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

/// Scrolling the inactive pane with the wheel does not activate it, but
/// leaving the split tab must still save where that pane was scrolled.
///
/// parity: VIEW-059, TAB-057
#[gtk::test]
fn switching_tabs_keeps_the_inactive_panes_scroll() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.wait_for_listing("the right pane");
    settle();
    let split = test
        .window
        .imp()
        .session
        .borrow()
        .active_id()
        .expect("the split tab");
    let inactive = test.window.pane_on(PaneSide::Start);
    inactive.details().vadjustment().set_value(700.0);
    let scroll = inactive.scroll_position();
    assert!(scroll > 0.0, "the inactive pane has scrolled");
    assert_eq!(test.window.active_side(), PaneSide::End);

    test.window.add_tab(&fixture.uri()).expect("a local folder");
    test.wait_for_listing("the new tab");
    test.window.switch_tab(split);
    settle();

    assert!(
        (inactive.scroll_position() - scroll).abs() < 1.0,
        "the inactive pane keeps its scroll: expected {scroll}, got {}",
        inactive.scroll_position()
    );
}

/// Gives the inactive pane a simulated network listing in progress. No
/// SMB I/O runs: notifications must only schedule its next listing.
fn simulated_network_pane(test: &TestWindow) -> (TabId, TabId) {
    let mut session = test.window.imp().session.borrow_mut();
    let active = session.active_id().expect("the split tab");
    let beside = session.beside_active().expect("the other pane");
    let tab = session.tab_mut(beside).expect("the other pane");
    tab.history = crate::history::History::new("smb://example.invalid/share");
    tab.listing_state.begin();
    (active, beside)
}

/// The inactive pane is still visible, so a simulated SMB notification
/// must schedule a refresh just as it does in the active pane.
///
/// parity: VIEW-059, TAB-056
#[gtk::test]
fn a_visible_inactive_network_pane_schedules_its_refresh() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.wait_for_listing("the right pane");
    let (_, beside) = simulated_network_pane(&test);

    test.window.folder_changed(beside);

    let session = test.window.imp().session.borrow();
    let tab = session.tab(beside).expect("the other pane");
    assert!(!tab.changed_while_hidden, "the pane is visible");
    assert!(
        tab.listing_state.has_pending_reload(),
        "refresh after its current listing"
    );
}

/// A notification received while the entire split tab is hidden waits;
/// showing that tab consumes it for both panes, including the inactive one.
///
/// parity: VIEW-059, TAB-056
#[gtk::test]
fn showing_a_split_tab_refreshes_its_dirty_inactive_pane() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.wait_for_listing("the right pane");
    let (active, beside) = simulated_network_pane(&test);
    test.window.add_tab(&fixture.uri()).expect("a local folder");
    test.wait_for_listing("the new tab");
    test.window.folder_changed(beside);
    {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(beside).expect("the other pane");
        assert!(tab.changed_while_hidden, "the whole tab is hidden");
        assert!(!tab.listing_state.has_pending_reload(), "wait until visible");
    }

    test.window.switch_tab(active);

    let session = test.window.imp().session.borrow();
    let tab = session.tab(beside).expect("the other pane");
    assert!(!tab.changed_while_hidden, "the pane is visible again");
    assert!(
        tab.listing_state.has_pending_reload(),
        "refresh after its current listing"
    );
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
