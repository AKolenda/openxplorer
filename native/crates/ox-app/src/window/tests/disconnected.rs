// SPDX-License-Identifier: AGPL-3.0-only
//! A drive or share going away under the tabs that show it: a USB stick
//! pulled out, a share disconnected by another program.
//!
//! A test cannot unplug a drive, so the window is told as the volume
//! monitor's `mount-removed` handler tells it, with the mount's root, and
//! a folder watch reports the unmount as its monitor thread does. A
//! fixture folder stands for the drive; marked as a kernel SMB mount
//! (`network_mounts`), it stands for a share.

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

/// Marks `fixture`'s folder as a share's mount point, as the window's
/// list of kernel SMB mounts does, so its going away is a share's.
fn as_share(test: &TestWindow, fixture: &Fixture) {
    test.window
        .imp()
        .locations
        .borrow_mut()
        .network_mounts
        .push(fixture.root().to_path_buf());
}

/// The tab in front, a tab behind it and a split pane on a drive that is
/// pulled out all go to the Desktop at once, with no message, as Windows
/// 11's window on a USB stick does (checked in the VM on 2026-11-22); a
/// tab on another drive stays where it is.
///
/// parity: DEV-014
#[gtk::test]
fn every_tab_on_a_pulled_out_drive_goes_to_the_desktop() {
    let drive = Fixture::standard();
    let other = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    let desktop = test.window.desktop_location();
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

    assert_eq!(test.window.current_uri().as_deref(), Some(desktop.as_str()));
    test.wait_for_listing("the Desktop");
    assert_eq!(test.window.load_error(), None, "no message");
    if test.window.folder_pane().page() == Some(PanePage::Empty) {
        assert_ne!(
            test.window.folder_pane().empty_page().title(),
            "This location is unavailable",
            "an empty Desktop, not the unavailable page"
        );
    }
    let uri_of = |id: TabId| {
        let session = test.window.imp().session.borrow();
        session.tab(id).map(|tab| tab.uri().to_owned())
    };
    assert_eq!(uri_of(behind).as_deref(), Some(desktop.as_str()));
    assert_eq!(error_of(&test, behind), None);
    let on_drive: Vec<String> = {
        let session = test.window.imp().session.borrow();
        session
            .panes()
            .map(|tab| tab.uri().to_owned())
            .filter(|uri| uri.starts_with(&drive.uri()))
            .collect()
    };
    assert!(
        on_drive.is_empty(),
        "the split pane left the drive too: {on_drive:?}"
    );
    assert_eq!(uri_of(elsewhere), Some(other.uri()));
    assert_eq!(error_of(&test, elsewhere), None);
    assert_eq!(rows_of(&test, elsewhere), elsewhere_rows);
}

/// A folder watch that sees its drive unmounted sends the tab to the
/// Desktop too, for drives the volume monitor does not show.
///
/// parity: DEV-014
#[gtk::test]
fn an_unmount_seen_by_the_folder_watch_goes_to_the_desktop() {
    let drive = Fixture::standard();
    let test = TestWindow::open(&drive.uri());
    let desktop = test.window.desktop_location();
    let watch = {
        let session = test.window.imp().session.borrow();
        let tab = session.active().expect("one tab");
        tab.watch.as_ref().expect("the folder is watched").id()
    };
    report_gone(watch);

    assert_eq!(test.window.current_uri().as_deref(), Some(desktop.as_str()));
    assert_eq!(test.window.load_error(), None);
}

/// Every tab on a share that went away drops its old rows and says the
/// share was disconnected; nothing reads it any more, and Try again lists
/// it once it is back.
///
/// parity: DEV-014
#[gtk::test]
fn a_tab_on_a_share_that_went_away_says_so_and_tries_again() {
    let share = Fixture::standard();
    let test = TestWindow::open(&share.uri());
    as_share(&test, &share);
    let id = test.active_tab().expect("one tab");

    test.window.mount_removed(&share.uri());

    let pane = test.window.folder_pane();
    assert_eq!(pane.page(), Some(PanePage::Empty));
    assert_eq!(pane.empty_page().title(), "This location is unavailable");
    assert_eq!(pane.empty_page().message(), DISCONNECTED);
    assert!(pane.empty_page().offers_try_again());
    assert!(test.names().is_empty(), "the old rows are gone");
    {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(id).expect("the tab stays open");
        assert!(tab.watch.is_none(), "the folder is no longer watched");
        assert!(tab.listing.is_none(), "nothing lists it");
    }
    test.activate("refresh", None);
    test.wait_for_listing("the folder again");
    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.names(), STANDARD_NAMES);
}

/// A folder watch that sees a share unmounted makes the tab say so.
///
/// parity: DEV-014
#[gtk::test]
fn an_unmount_of_a_share_seen_by_the_folder_watch_shows_the_message() {
    let share = Fixture::standard();
    let test = TestWindow::open(&share.uri());
    as_share(&test, &share);
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

/// On a share, a tab that already lists the folder again when shown, as
/// Sign out leaves its server's tabs, is not touched; a mount whose
/// folder is already reported is not reported twice.
///
/// parity: DEV-014
#[gtk::test]
fn tabs_waiting_to_be_listed_are_left_alone() {
    let share = Fixture::standard();
    let test = TestWindow::open(&share.uri());
    as_share(&test, &share);
    test.window
        .open_tab(&share.uri_of("Documents"), TabPlacement::Background)
        .expect("a tab behind");
    let waiting = {
        let session = test.window.imp().session.borrow();
        session.tabs().last().expect("two tabs").id
    };
    assert_eq!(rows_of(&test, waiting), 0, "a tab behind is listed when shown");

    test.window.mount_removed(&share.uri());
    test.window.mount_removed(&share.uri());

    assert_eq!(error_of(&test, waiting), None);
    test.activate_tab(waiting);
    test.wait_for_listing("the waiting tab");
    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.window.current_uri(), Some(share.uri_of("Documents")));
}
