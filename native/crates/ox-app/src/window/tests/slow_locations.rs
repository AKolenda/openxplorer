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

use super::icons::pixel_rows;
use crate::test_support::harness::{
    settle, wait_for, wait_for_frames, wait_until, Fixture, OpenedWindows, TestWindow, STANDARD_NAMES,
};
use crate::test_support::held;
use crate::window::address_bar::AddressMode;
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

/// Whether the button before the address bar is Refresh, saying so and
/// its key.
fn shows_refresh(test: &TestWindow) -> bool {
    let (action, tooltip) = refresh_or_stop(test);
    action == "win.refresh" && tooltip.starts_with("Refresh") && tooltip.ends_with(" (F5)")
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

/// Waits until the tab in front says "Working on it...", step by step, so
/// a failure on a busy runner names the step that never came: the tab
/// going to its location, the delay passing, or the file area saying so.
fn wait_for_working(test: &TestWindow) {
    wait_until("the tab to be busy", || test.window.is_busy());
    wait_until("the location to count as slow", || test.window.is_slow());
    wait_until("Working on it...", || says_working(test));
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
    wait_for_working(&test);
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
    wait_for_working(&test);

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
    wait_for_working(&test);

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
    wait_for_working(&test);
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
    wait_for_working(&test);
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
    wait_for_working(&test);

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
    wait_for_working(&test);
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
    wait_for_working(&test);
    assert_eq!(visible_progress_over_the_items(&test), 0, "a folder click");
    drop(hold);
    wait_until("the opened folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test));
}

/// Another address entered while a folder from the list is still being
/// opened replaces it, and the tab still opens items afterwards: the
/// replaced opening no longer holds the tab's one activation.
///
/// parity: OPEN-001, VIEW-068
#[gtk::test]
fn another_address_while_a_folder_opens_from_the_list_keeps_items_openable() {
    let fixture = Fixture::standard();
    let other = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let hold = held::hold(&fixture.uri_of("Documents"));
    test.window.activate_item(test.position_of("Documents"));
    settle();
    assert!(shows_stop(&test), "the folder is being opened");

    enter_address(&test, &other.path("").display().to_string());
    wait_until("the other folder", || {
        test.window.current_uri() == Some(other.uri()) && !test.window.is_loading()
    });
    drop(hold);

    test.window.activate_item(test.position_of("Documents"));
    let documents = other.uri_of("Documents");
    wait_until("a folder opened from the list", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
}

/// A location whose lookup and listing each take less than the delay,
/// but together more, says "Working on it..." once the delay has passed
/// since Enter, and keeps saying it from the lookup into the listing.
///
/// parity: VIEW-068
#[gtk::test]
fn working_on_it_carries_over_from_the_lookup_to_the_listing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let lookup = held::hold(&held::lookup_key(&documents));
    let listing = held::hold(&held::listing_key(&documents));
    let phase = APPEARANCE_DELAY * 2 / 3;

    let started = Instant::now();
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_for(phase);
    // However late a busy runner checks, it can only say so once the
    // delay has passed since Enter.
    assert!(
        !says_working(&test) || started.elapsed() >= APPEARANCE_DELAY,
        "not before the delay"
    );
    let lookup_answered = Instant::now();
    drop(lookup);
    wait_until("the listing to run", || {
        held::waits(&held::listing_key(&documents)) == 1
    });
    // The listing goes on from the lookup's time, not its own.
    let since = test
        .window
        .imp()
        .session
        .borrow()
        .active()
        .and_then(|tab| tab.busy_clock.since());
    assert!(
        since.is_some_and(|since| since < lookup_answered),
        "the time runs on from Enter"
    );
    wait_for_working(&test);
    assert!(started.elapsed() >= APPEARANCE_DELAY, "it waited for the delay");
    let shown = Instant::now();
    while shown.elapsed() < phase {
        settle();
        assert!(
            says_working(&test),
            "Working on it... went away while the folder was still listed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(listing);
    wait_until("the folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(shows_nothing_busy(&test));
}

/// Whether the status bar says the folder is loading.
fn status_says_loading(test: &TestWindow) -> bool {
    test.window.status_bar().texts().0.ends_with(" · Loading…")
}

/// Each tab keeps its own time: a slow tab shown again says "Working on
/// it..." and "Loading…" at once, without waiting for the delay again,
/// and a tab whose location finished in the background shows none of it.
///
/// parity: TAB-056, VIEW-050, VIEW-068
#[gtk::test]
fn each_tab_keeps_its_own_slow_state() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&held::listing_key(&documents));
    test.window.navigate(&documents).expect("a folder");
    wait_for_working(&test);
    let slow_tab = test.active_tab().expect("a tab");

    test.window
        .open_tab(&fixture.uri(), TabPlacement::Foreground)
        .expect("another tab");
    test.wait_for_listing("the other tab");
    settle();
    let fast_tab = test.active_tab().expect("the other tab");
    assert!(shows_nothing_busy(&test), "the other tab is not busy");
    assert!(!status_says_loading(&test));

    test.activate_tab(slow_tab);
    settle();
    assert!(says_working(&test), "the slow tab says so at once");
    assert!(status_says_loading(&test));
    assert!(address_shows_progress(&test));

    // It finishes while another tab is in front.
    test.activate_tab(fast_tab);
    settle();
    drop(hold);
    wait_until("the background listing", || {
        let session = test.window.imp().session.borrow();
        session
            .tab(slow_tab)
            .is_some_and(|tab| !tab.listing_state.is_listing())
    });
    test.activate_tab(slow_tab);
    settle();
    assert!(shows_nothing_busy(&test), "nothing is left of its slow state");
    assert!(!status_says_loading(&test));
    assert_eq!(test.window.current_uri(), Some(documents));
}

/// Enter on the folder the tab is listing for the first time does not
/// list it again, but ends the edit: the address bar returns to its
/// crumbs, as for any address it opens.
///
/// parity: NAV-027, VIEW-068
#[gtk::test]
fn enter_on_the_folder_being_listed_returns_the_address_bar_to_its_crumbs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&held::listing_key(&documents));
    test.window.navigate(&documents).expect("a folder");
    wait_until("the listing to wait", || {
        held::waits(&held::listing_key(&documents)) == 1
    });

    enter_address(&test, &fixture.path("Documents").display().to_string());

    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
    assert!(shows_stop(&test), "the listing goes on");
    drop(hold);
    test.wait_for_listing("the folder");
    assert_eq!(
        held::waits(&held::listing_key(&documents)),
        1,
        "the listing was not started over"
    );
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
}

/// A slow lookup says "Loading…" in the status bar with "Working on
/// it...", as a slow listing does.
///
/// parity: VIEW-050, VIEW-068
#[gtk::test]
fn a_slow_lookup_says_loading_in_the_status_bar() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&held::lookup_key(&documents));

    enter_address(&test, &fixture.path("Documents").display().to_string());
    assert!(!status_says_loading(&test), "not before the delay");
    wait_for_working(&test);

    assert!(status_says_loading(&test));
    drop(hold);
    wait_until("the typed folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();
    assert!(!status_says_loading(&test));
}

/// A tab moved to another window while its address is looked up goes on
/// to that address there, showing that it is working, rather than
/// staying where it was.
///
/// parity: TAB-029, VIEW-068
#[gtk::test]
fn a_tab_moved_while_a_slow_address_opens_goes_on_to_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.add_tab(&fixture.uri()).expect("a folder");
    test.wait_for_listing("the second tab");
    let moved = test.active_tab().expect("a tab");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&held::lookup_key(&documents));
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_until("the lookup to wait", || {
        held::waits(&held::lookup_key(&documents)) == 1
    });

    test.window.move_tab_to_new_window(moved);
    let opened = OpenedWindows::only(&[&test.window]);
    let window = opened.window();
    settle();

    assert!(
        window.address_bar().has_css_class("loading"),
        "the moved tab still shows that it is going there"
    );
    assert_eq!(test.window.tab_count(), 1, "the original went");
    drop(hold);
    wait_until("the address in the new window", || {
        window.current_uri().as_deref() == Some(documents.as_str()) && window.is_listed()
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// A typed address answered after the user moved to the other pane of a
/// split tab still opens in its own pane, which is listed at once since
/// it stays on screen.
///
/// parity: VIEW-059, VIEW-068
#[gtk::test]
fn a_slow_address_answered_after_moving_to_the_other_pane_lists_its_pane() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.wait_for_listing("the right pane");
    let slow_side = test.window.active_side();
    let documents = fixture.uri_of("Documents");
    std::fs::write(fixture.path("Documents/Agenda.txt"), "agenda").expect("a file");
    let hold = held::hold(&held::lookup_key(&documents));
    enter_address(&test, &fixture.path("Documents").display().to_string());
    wait_until("the lookup to wait", || {
        held::waits(&held::lookup_key(&documents)) == 1
    });

    test.window.activate_pane(slow_side.other());
    settle();
    drop(hold);

    let pane = test.window.pane_on(slow_side);
    wait_until("the typed folder listed in its pane", || {
        pane.model().n_items() == 1
    });
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the active pane stays"
    );
}

/// An address on a share that is not mounted, answered after the user
/// looked at another tab, opens nothing there: it is not known to be a
/// folder, and mounting it may find a file, which must not be launched.
///
/// parity: SAFE-013, VIEW-068
#[gtk::test]
fn an_unmounted_address_answered_after_leaving_the_tab_opens_nothing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let slow_tab = test.active_tab().expect("a tab");
    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    let address = "smb://example.invalid/share/Report.pdf";
    let hold = held::hold(&held::lookup_key(address));
    enter_address(&test, address);
    wait_until("the lookup to wait", || {
        held::waits(&held::lookup_key(address)) == 1
    });

    test.window
        .open_tab(&fixture.uri(), TabPlacement::Foreground)
        .expect("another tab");
    test.wait_for_listing("the other tab");
    test.activate_tab(slow_tab);
    settle();
    drop(hold);
    wait_until("the lookup to answer", || !test.window.is_tab_busy(slow_tab));
    settle();

    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "nothing opened");
}

/// Enter again on a typed file address whose lookup lost its right to
/// open a file, as the user looked at another tab meanwhile, opens the
/// file once it answers: the new Enter asks for it.
///
/// parity: SAFE-013, VIEW-068
#[gtk::test]
fn enter_again_after_leaving_the_tab_opens_the_typed_file() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("agenda.txt"), "agenda").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let slow_tab = test.active_tab().expect("a tab");
    let agenda = fixture.uri_of("agenda.txt");
    let path = fixture.path("agenda.txt").display().to_string();
    let hold = held::hold(&held::lookup_key(&agenda));
    enter_address(&test, &path);
    wait_until("the lookup to wait", || {
        held::waits(&held::lookup_key(&agenda)) == 1
    });
    test.window
        .open_tab(&fixture.uri(), TabPlacement::Foreground)
        .expect("another tab");
    test.wait_for_listing("the other tab");
    test.activate_tab(slow_tab);
    settle();

    enter_address(&test, &path);
    drop(hold);

    wait_until("the typed file to open", || {
        test.context.recorded_launches().len() == 1
    });
}

/// Enter on a typed file address that is being looked up as a pasted one
/// opens the file: the new Enter decides, rather than the paste's "show
/// it in its folder".
///
/// parity: NAV-032, VIEW-068
#[gtk::test]
fn enter_on_a_file_being_gone_to_as_pasted_opens_it() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("agenda.txt"), "agenda").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let agenda = fixture.uri_of("agenda.txt");
    let path = fixture.path("agenda.txt").display().to_string();
    let hold = held::hold(&held::lookup_key(&agenda));
    test.window.go_to_pasted_address(&path);
    wait_until("the lookup to wait", || {
        held::waits(&held::lookup_key(&agenda)) == 1
    });

    enter_address(&test, &path);
    drop(hold);

    wait_until("the typed file to open", || {
        test.context.recorded_launches().len() == 1
    });
}

/// A folder opened from the list with the keyboard keeps the keyboard in
/// the list: hiding the previous folder's rows while it opens slowly does
/// not leave keyboard focus nowhere.
///
/// parity: VIEW-068
#[gtk::test]
fn the_list_keeps_the_keyboard_across_a_slow_opening() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("Documents/Agenda.txt"), "agenda").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&held::lookup_key(&documents));
    test.window.folder_pane().focus_view();
    settle();
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the list has the keyboard"
    );

    test.window.activate_item(test.position_of("Documents"));
    wait_for_working(&test);
    drop(hold);
    wait_until("the opened folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    settle();

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the list has the keyboard again"
    );
}

/// How many colours the window draws now where the details view's column
/// titles are: one while nothing is drawn there. The window is painted as
/// it goes to the screen, from the drawings GTK keeps of its widgets, so a
/// drawing GTK failed to renew shows as it would on screen.
fn colours_drawn_on_the_titles(test: &TestWindow) -> usize {
    wait_for_frames(&test.window, 3);
    let titles = test
        .window
        .folder_pane()
        .details()
        .header()
        .expect("the details view has column titles");
    let outer = test.window.compute_bounds(&test.window).expect("a drawn window");
    let bounds = titles.compute_bounds(&test.window).expect("laid-out titles");
    let area = gtk::graphene::Rect::new(
        bounds.x() - outer.x(),
        bounds.y() - outer.y(),
        bounds.width(),
        bounds.height(),
    );
    let native = test.window.upcast_ref::<gtk::Native>();
    let paintable = gtk::WidgetPaintable::new(Some(native));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(paintable.intrinsic_width()),
        f64::from(paintable.intrinsic_height()),
    );
    let node = snapshot.to_node().expect("the window draws");
    let renderer = native.renderer().expect("a drawn window");
    let texture = renderer.render_texture(&node, Some(&area));
    let colours: std::collections::HashSet<_> = pixel_rows(&texture).concat().into_iter().collect();
    colours.len()
}

/// The rows "Working on it..." hid while a slow folder opened are drawn
/// again once it is listed, column titles and all, and so are those of
/// the folder gone back to. GTK 4.14 left the whole list blank from then
/// on, in every folder, with the status bar counting its items.
///
/// parity: VIEW-068
#[gtk::test]
fn rows_hidden_while_a_slow_folder_opens_are_drawn_again() {
    let fixture = Fixture::standard();
    fixture.write("Documents/Plan.txt");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    assert!(colours_drawn_on_the_titles(&test) > 1, "the titles are drawn");
    let documents = fixture.uri_of("Documents");
    let hold = held::hold(&documents);

    test.window.activate_item(test.position_of("Documents"));
    wait_for_working(&test);
    assert!(test.window.folder_pane().rows_hidden());
    assert_eq!(colours_drawn_on_the_titles(&test), 1, "the titles are hidden");

    drop(hold);
    wait_until("the opened folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str()) && !test.window.is_loading()
    });
    assert_eq!(test.names(), ["Plan.txt"]);
    assert!(
        colours_drawn_on_the_titles(&test) > 1,
        "the opened folder's titles are drawn"
    );

    test.activate("back", None);
    test.wait_for_listing("the first folder again");
    assert_eq!(test.names(), STANDARD_NAMES);
    assert!(
        colours_drawn_on_the_titles(&test) > 1,
        "the first folder's titles are drawn"
    );
}
