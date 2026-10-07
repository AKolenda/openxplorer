// SPDX-License-Identifier: AGPL-3.0-only
//! A drive or share going away under the tabs that show it: a USB stick
//! pulled out, a share disconnected by another program.
//!
//! A test cannot unplug a drive, so the window is told as the volume
//! monitor's `mount-removed` handler tells it, with the mount's root, and
//! a folder watch reports the unmount as its monitor thread does.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::watch::report_gone;
use crate::test_support::harness::{Fixture, TestWindow, STANDARD_NAMES};
use crate::window::folder_pane::PanePage;
use crate::window::session::{PaneSide, TabId, TabPlacement};

const DISCONNECTED: &str = "The drive or network share that holds this folder was disconnected.";

/// The error tab `id` shows, as text.
fn error_of(test: &TestWindow, id: TabId) -> Option<String> {
    let session = test.window.imp().session.borrow();
    session
        .tab(id)
        .and_then(|tab| tab.error.as_ref().map(ToString::to_string))
}

/// How many rows tab `id` holds.
fn rows_of(test: &TestWindow, id: TabId) -> u32 {
    let session = test.window.imp().session.borrow();
    session.tab(id).map_or(0, |tab| tab.store.n_items())
}

/// The tab in front, a tab behind it and a split pane on the drive all
/// drop their old rows and say the drive was disconnected; a tab on
/// another drive keeps its rows.
///
/// parity: DEV-014
#[gtk::test]
fn every_tab_on_a_removed_drive_drops_its_rows_and_says_so() {
    let drive = Fixture::standard();
    let other = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    let front = test.active_tab().expect("one tab");
    test.activate("split-view", None);
    test.window.navigate_or_report(&drive.uri_of("Documents"));
    test.wait_for_listing("Documents in the right pane");
    test.window.activate_pane(PaneSide::Start);
    for uri in [drive.uri_of("Documents"), other.uri()] {
        test.window
            .open_tab(&uri, TabPlacement::Background)
            .expect("a tab behind");
    }
    let (behind, elsewhere) = {
        let session = test.window.imp().session.borrow();
        let ids: Vec<TabId> = session.tabs().iter().map(|tab| tab.id).collect();
        (ids[1], ids[2])
    };
    for id in [behind, elsewhere] {
        test.activate_tab(id);
        test.wait_for_listing("the tab behind");
    }
    test.activate_tab(front);
    test.window.activate_pane(PaneSide::Start);
    assert_eq!(test.names(), STANDARD_NAMES);
    let elsewhere_rows = rows_of(&test, elsewhere);

    test.window.mount_removed(&drive.uri());

    let pane = test.window.folder_pane();
    assert_eq!(pane.page(), Some(PanePage::Empty));
    assert_eq!(pane.empty_page().title(), "This location is unavailable");
    assert_eq!(pane.empty_page().message(), DISCONNECTED);
    assert!(pane.empty_page().offers_try_again());
    assert!(test.names().is_empty(), "the old rows are gone");
    let split = test.window.pane_on(PaneSide::End);
    assert_eq!(split.page(), Some(PanePage::Empty), "the split pane says so too");
    assert_eq!(split.empty_page().message(), DISCONNECTED);
    assert_eq!(error_of(&test, behind).as_deref(), Some(DISCONNECTED));
    assert_eq!(rows_of(&test, behind), 0);
    assert_eq!(error_of(&test, elsewhere), None);
    assert_eq!(rows_of(&test, elsewhere), elsewhere_rows);
}

/// Nothing is listed or watched on the gone drive any more, so nothing
/// fills the rows again behind the message or keeps the drive busy.
///
/// parity: DEV-014
#[gtk::test]
fn a_tab_on_a_removed_drive_stops_reading_it() {
    let drive = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    let id = test.active_tab().expect("one tab");

    test.window.mount_removed(&drive.uri());

    let session = test.window.imp().session.borrow();
    let tab = session.tab(id).expect("the tab stays open");
    assert!(tab.watch.is_none(), "the folder is no longer watched");
    assert!(tab.listing.is_none(), "nothing lists it");
    drop(session);
    assert!(test.names().is_empty());
}

/// A folder watch that sees its folder's drive unmounted makes the tab
/// say so, for mounts the volume monitor does not show.
///
/// parity: DEV-014
#[gtk::test]
fn an_unmount_seen_by_the_folder_watch_shows_the_message() {
    let drive = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    let watch = {
        let session = test.window.imp().session.borrow();
        let tab = session.active().expect("one tab");
        tab.watch.as_ref().expect("the folder is watched").id()
    };
    report_gone(watch);

    assert_eq!(test.window.load_error().as_deref(), Some(DISCONNECTED));
    assert_eq!(test.window.folder_pane().page(), Some(PanePage::Empty));
    assert!(test.names().is_empty());
}

/// Try again lists the folder again once the drive is back.
///
/// parity: DEV-014
#[gtk::test]
fn try_again_lists_the_folder_once_the_drive_is_back() {
    let drive = Fixture::standard();
    let test = TestWindow::open(&drive.uri());

    test.window.mount_removed(&drive.uri());
    test.activate("refresh", None);
    test.wait_for_listing("the folder again");

    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.names(), STANDARD_NAMES);
}

/// A tab that already lists the folder again when shown, as Sign out
/// leaves its server's tabs, and a tab elsewhere are not touched; a mount
/// whose folder is already reported is not reported twice.
///
/// parity: DEV-014
#[gtk::test]
fn tabs_waiting_to_be_listed_are_left_alone() {
    let drive = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    test.window
        .open_tab(&drive.uri_of("Documents"), TabPlacement::Background)
        .expect("a tab behind");
    let waiting = {
        let session = test.window.imp().session.borrow();
        session.tabs().last().expect("two tabs").id
    };
    assert_eq!(rows_of(&test, waiting), 0, "a tab behind is listed when shown");

    test.window.mount_removed(&drive.uri());
    test.window.mount_removed(&drive.uri());

    assert_eq!(error_of(&test, waiting), None);
    test.activate_tab(waiting);
    test.wait_for_listing("the waiting tab");
    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.window.current_uri(), Some(drive.uri_of("Documents")));
}
