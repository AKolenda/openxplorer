// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a slow location, as Windows Explorer shows it: the address bar
//! shows its progress and Refresh turns into Stop at once, the file area
//! says "Working on it..." in place of the previous folder once the
//! location has taken a moment, Stop cancels, Enter on the same address
//! does not start it over, and another address replaces it so that no
//! late result lands. Each location is held (see
//! [`crate::test_support::held`]) as a slow share holds its reply.

use std::time::Instant;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::network::NetworkError;

use crate::test_support::harness::{settle, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::test_support::held;
use crate::window::session::TabPlacement;
use crate::window::slow_delay::APPEARANCE_DELAY;

/// What the file area says while a location is opened.
const WORKING: &str = "Working on it...";

/// The button before the address bar: its action and tooltip.
fn refresh_or_stop(test: &TestWindow) -> (String, String) {
    let row = &*test.window.imp().navigation_buttons;
    let button = row
        .last_child()
        .and_downcast::<gtk::Button>()
        .expect("Refresh or Stop ends the row");
    let action = button
        .action_name()
        .map(|name| name.to_string())
        .unwrap_or_default();
    let tooltip = button
        .tooltip_text()
        .map(|text| text.to_string())
        .unwrap_or_default();
    (action, tooltip)
}

/// Whether the button before the address bar is Stop.
fn shows_stop(test: &TestWindow) -> bool {
    refresh_or_stop(test) == ("win.stop".to_owned(), "Stop".to_owned())
}

/// Whether the button before the address bar is Refresh.
fn shows_refresh(test: &TestWindow) -> bool {
    refresh_or_stop(test) == ("win.refresh".to_owned(), "Refresh (F5)".to_owned())
}

/// Whether the address bar shows that a location is being opened.
fn address_shows_progress(test: &TestWindow) -> bool {
    test.window.address_bar().has_css_class("loading")
}

/// Whether the file area says "Working on it..." in place of rows.
fn says_working(test: &TestWindow) -> bool {
    let pane = test.window.folder_pane();
    pane.shown_note_text().as_deref() == Some(WORKING) && (pane.rows_hidden() || pane.model().n_items() == 0)
}

/// Types `path` into the address bar and presses Enter.
fn enter_address(test: &TestWindow, path: &str) {
    test.activate("location", None);
    test.window.address_bar().submit_text(path);
    settle();
}

/// Whether nothing shows that a location is being opened.
fn shows_nothing_busy(test: &TestWindow) -> bool {
    shows_refresh(test) && !address_shows_progress(test) && !says_working(test)
}

/// A typed address on a slow location shows its progress and Stop at
/// once, "Working on it..." in place of the previous folder after a
/// moment, and its own folder once it answers.
///
/// parity: VIEW-047, VIEW-049, VIEW-068
#[gtk::test]
fn a_slow_typed_address_shows_progress_stop_and_working_on_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);

    let started = Instant::now();
    enter_address(&test, &fixture.path("Documents").display().to_string());

    assert!(shows_stop(&test), "Refresh turns into Stop at once");
    assert!(
        address_shows_progress(&test),
        "the address bar shows progress at once"
    );
    assert!(!says_working(&test), "no message before the delay");
    wait_until("Working on it...", || says_working(&test));
    assert!(started.elapsed() >= APPEARANCE_DELAY, "the message waited");
    assert!(
        test.window.folder_pane().rows_hidden() || test.names().is_empty(),
        "the previous folder's rows are gone"
    );

    drop(hold);
    wait_until("the typed folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test), "the folder is listed");
    assert!(!test.window.folder_pane().rows_hidden());
}

/// Opening a slow folder from the list shows the same: progress and Stop
/// at once, then "Working on it...".
///
/// parity: VIEW-068
#[gtk::test]
fn a_slow_folder_opened_from_the_list_shows_progress_and_working_on_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);

    test.window.activate_item(test.position_of("Documents"));
    settle();

    assert!(shows_stop(&test), "Refresh turns into Stop at once");
    assert!(address_shows_progress(&test));
    wait_until("Working on it...", || says_working(&test));

    drop(hold);
    wait_until("the opened folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test));
}

/// Stop cancels a slow address: the tab stays where it was, with its
/// rows, and the address answering later changes nothing.
///
/// parity: VIEW-049, VIEW-068
#[gtk::test]
fn stop_cancels_a_slow_address_and_its_late_answer_lands_nowhere() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let hold = held::hold(&fixture.uri_of("Documents"));
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_until("Working on it...", || says_working(&test));

    test.activate("stop", None);
    settle();

    assert!(shows_nothing_busy(&test), "Stop ends it at once");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(!test.window.folder_pane().rows_hidden(), "the rows come back");
    assert_eq!(test.names(), STANDARD_NAMES);

    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "nothing late");
    assert_eq!(test.names(), STANDARD_NAMES);
    assert!(shows_nothing_busy(&test));
}

/// Stop cancels a slow listing at once, too: the button and the address
/// bar answer straight away, and the listing's rows never arrive.
///
/// parity: VIEW-049, VIEW-068
#[gtk::test]
fn stop_cancels_a_slow_listing_at_once() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);
    test.window.navigate(&documents).expect("a folder");
    settle();
    assert!(shows_stop(&test), "Stop shows as soon as the listing starts");
    assert!(address_shows_progress(&test));

    test.activate("stop", None);
    settle();

    assert!(shows_nothing_busy(&test));
    assert!(!test.window.is_loading());
    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert!(test.names().is_empty(), "the stopped listing's rows never land");
}

/// Enter on the address being opened does not start it over, neither
/// while it is looked up nor while it is listed.
///
/// parity: VIEW-068
#[gtk::test]
fn enter_again_on_the_same_slow_address_does_not_start_over() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let path = fixture.path("Documents").display().to_string();
    let hold = held::hold(&documents);

    enter_address(&test, &path);
    wait_until("the lookup to wait", || held::waits(&documents) == 1);
    enter_address(&test, &path);
    wait_for(APPEARANCE_DELAY);
    assert_eq!(held::waits(&documents), 1, "the lookup was not started over");
    assert!(shows_stop(&test));
    drop(hold);
    wait_until("the typed folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });

    // The same while the folder is listed.
    let notes = fixture.uri_of("Documents/Notes");
    std::fs::create_dir(fixture.path("Documents/Notes")).expect("a new folder");
    let hold = held::hold(&notes);
    test.window.navigate(&notes).expect("a folder");
    wait_until("the listing to wait", || held::waits(&notes) == 1);
    enter_address(&test, &fixture.path("Documents/Notes").display().to_string());
    wait_for(APPEARANCE_DELAY);
    assert!(shows_stop(&test));
    assert_eq!(held::waits(&notes), 1, "the listing was not started over");
    drop(hold);
    test.wait_for_listing("the new folder");
    assert_eq!(test.window.current_uri(), Some(notes));
}

/// Another address entered while one is opened replaces it: the first is
/// cancelled, and its late answer never lands in the tab.
///
/// parity: NAV-016, VIEW-068
#[gtk::test]
fn another_address_replaces_a_slow_one_and_nothing_late_lands() {
    let fixture = Fixture::standard();
    let empty = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);
    enter_address(&test, &fixture.path("Documents").display().to_string());
    assert!(shows_stop(&test));

    enter_address(&test, &empty.path("").display().to_string());
    wait_until("the second address", || {
        test.window.current_uri() == Some(empty.uri()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test), "the first address is cancelled");

    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert_eq!(test.window.current_uri(), Some(empty.uri()), "nothing late");
    assert!(test.names().is_empty());
    assert!(shows_nothing_busy(&test));
}

/// Another folder entered while a slow one is listed replaces it: none
/// of the slow folder's rows land.
///
/// parity: NAV-016, VIEW-068
#[gtk::test]
fn another_address_replaces_a_slow_listing() {
    let fixture = Fixture::standard();
    let empty = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);
    test.window.navigate(&documents).expect("a folder");
    wait_until("the listing to wait", || held::waits(&documents) == 1);

    enter_address(&test, &empty.path("").display().to_string());
    wait_until("the second address", || {
        test.window.current_uri() == Some(empty.uri()) && !test.window.is_loading()
    });
    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert_eq!(test.window.current_uri(), Some(empty.uri()));
    assert!(test.names().is_empty(), "no late rows");
    assert!(shows_nothing_busy(&test));
}

/// Escape while a typed address is opened ends editing, as it always
/// does, and cancels the address, whose answer then lands nowhere.
///
/// parity: NAV-027, VIEW-068
#[gtk::test]
fn escape_in_the_address_bar_cancels_a_slow_address() {
    use gtk::gdk;
    use gtk::glib::translate::IntoGlib;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let hold = held::hold(&fixture.uri_of("Documents"));
    enter_address(&test, &fixture.path("Documents").display().to_string());
    assert!(shows_stop(&test));

    let entry = test.window.address_bar().entry();
    let keys: Vec<gtk::EventControllerKey> = entry
        .observe_controllers()
        .into_iter()
        .filter_map(|controller| controller.ok().and_downcast::<gtk::EventControllerKey>())
        .collect();
    let no_keycode = 0_u32;
    let taken = keys.iter().any(|keys| {
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gdk::Key::Escape.into_glib(),
                &no_keycode,
                &gdk::ModifierType::empty(),
            ],
        )
    });
    settle();

    assert!(taken, "the address bar takes Escape");
    assert!(shows_nothing_busy(&test));
    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// A share mounted for the first time shows the same while it mounts,
/// and Stop cancels the mount; a failed mount shows the usual error and
/// no message.
///
/// parity: NET-004, VIEW-068
#[gtk::test]
fn a_slow_first_mount_shows_working_on_it_and_stop_cancels_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let share = "smb://example.invalid/share";
    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    let hold = held::hold_mount(share);

    test.window.navigate(share).expect("an SMB share");
    wait_until("the mount to wait", || held::waits(&held::mount_key(share)) == 1);
    assert!(shows_stop(&test));
    assert!(address_shows_progress(&test));
    wait_until("Working on it...", || says_working(&test));
    assert!(test.window.load_error().is_none(), "no error while it mounts");

    test.activate("stop", None);
    settle();
    assert!(shows_nothing_busy(&test));
    drop(hold);
    wait_for(APPEARANCE_DELAY * 2);
    assert!(
        test.window.load_error().is_none(),
        "the cancelled mount says nothing"
    );

    // Opening it again, the mount fails: the app's usual error, with no
    // message left.
    let hold = held::hold_mount(share);
    test.window.navigate(share).expect("an SMB share");
    wait_until("Working on it...", || says_working(&test));
    drop(hold);
    wait_until("the failed mount", || test.window.load_error().is_some());
    settle();
    assert!(test.window.folder_pane().shows_unavailable_page());
    assert!(shows_nothing_busy(&test));
}

/// Each tab keeps its own: another tab shown while one opens a slow
/// location is usable, with Refresh and no progress, and the slow tab
/// still opens its location when it is shown again.
///
/// parity: TAB-056, VIEW-068
#[gtk::test]
fn switching_tabs_while_a_slow_address_opens_keeps_each_tabs_state() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let slow_tab = test.active_tab().expect("a tab");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_until("Working on it...", || says_working(&test));

    test.window
        .open_tab(&fixture.uri(), TabPlacement::Foreground)
        .expect("another tab");
    test.wait_for_listing("the other tab");
    settle();
    assert!(shows_nothing_busy(&test), "the other tab is not busy");
    assert_eq!(test.names(), STANDARD_NAMES);

    test.activate_tab(slow_tab);
    settle();
    assert!(shows_stop(&test), "the slow tab is still being opened");
    assert!(address_shows_progress(&test));
    drop(hold);
    wait_until("the slow tab's folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test));
}

/// Widgets of the window that draw a second progress bar over the items,
/// visible now. Windows Explorer shows only the address bar's.
fn visible_progress_over_the_items(test: &TestWindow) -> usize {
    crate::window::widget_tree::descendants::<gtk::Widget>(&test.window)
        .iter()
        .filter(|widget| widget.has_css_class("loading-line") && widget.is_visible())
        .count()
}

/// Only the address bar shows progress, as in Windows Explorer: no line
/// runs over the items while a location is opened, listed or refreshed,
/// however long it takes.
///
/// parity: VIEW-047, VIEW-068
#[gtk::test]
fn only_the_address_bar_shows_progress_while_a_location_loads() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");

    // A typed address, looked up and then listed.
    let hold = held::hold(&documents);
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_until("Working on it...", || says_working(&test));
    assert!(address_shows_progress(&test));
    assert_eq!(visible_progress_over_the_items(&test), 0, "a typed address");
    drop(hold);
    wait_until("the typed folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });

    // A refresh of a slow folder keeps its page, with the address bar's
    // progress alone.
    let hold = held::hold(&documents);
    test.window.refresh();
    wait_for(APPEARANCE_DELAY * 2);
    assert!(test.window.is_loading(), "the refresh is held");
    assert!(address_shows_progress(&test));
    assert!(shows_stop(&test));
    assert_eq!(visible_progress_over_the_items(&test), 0, "a refresh");
    drop(hold);
    test.wait_for_listing("the refresh");

    // A folder opened from the list.
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    let hold = held::hold(&documents);
    test.window.activate_item(test.position_of("Documents"));
    wait_until("Working on it...", || says_working(&test));
    assert_eq!(visible_progress_over_the_items(&test), 0, "a folder click");
    drop(hold);
    wait_until("the opened folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test));
}
