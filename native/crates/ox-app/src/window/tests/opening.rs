// SPDX-License-Identifier: AGPL-3.0-only
//! Opening typed addresses, command-line locations and files. Files are
//! recorded instead of started (see `AppContext::record_launches`).

use std::fs;

use gtk::subclass::prelude::*;

use crate::locations::Page;
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::{Tab, TabPlacement};

fn can_go_back(test: &TestWindow) -> bool {
    let session = test.window.imp().session.borrow();
    session.active().is_some_and(|tab| tab.history.can_go_back())
}

/// A command that fails says why in the toast instead of failing
/// silently: here Enter on an address that does not exist, which leaves
/// the tab where it was.
///
/// parity: CMD-018
#[gtk::test]
fn a_failing_command_says_why_in_the_toast() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let missing = fixture.path("Missing folder");

    test.window
        .submit_address(missing.to_str().expect("fixture paths are UTF-8"));

    wait_until("the toast", || !test.window.shown_message().is_empty());
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.context.recorded_launches().is_empty());
}

/// A typed address whose lookup answers after the tab navigated elsewhere
/// is dropped: the file does not open and the tab stays where the user
/// went.
///
/// Ported from `desktop/tests/ui_regressions.cjs::Navigation supersedes a delayed activation`
///
/// parity: SAFE-013
#[gtk::test]
fn navigating_drops_a_typed_address_that_answers_late() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let file = fixture.path("Notes 2.txt");
    let typed = file.to_str().expect("fixture paths are UTF-8");

    // The lookup answers on a later main-loop turn, after this navigation.
    test.window.submit_address(typed);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("the fixture folder");
    wait_until("the lookup to answer", || test.window.answered_activations() == 1);

    assert!(test.context.recorded_launches().is_empty());
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.window.submit_address(typed);
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
}

/// Lookups belong to their own tab: a background tab that finds its
/// location is a file does not cancel the address the user typed in the
/// front tab, while switching away from a tab and back drops its lookup.
///
/// parity: SAFE-013
#[gtk::test]
fn a_lookup_belongs_to_its_tab_and_a_tab_switch_drops_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let typed = fixture.path("Notes 2.txt");
    let typed = typed.to_str().expect("fixture paths are UTF-8");
    let front = test.window.imp().session.borrow().active_id().expect("a tab");
    test.window
        .open_tab(&fixture.uri_of("Documents"), TabPlacement::Background)
        .expect("the fixture folder");
    let background = test.window.imp().session.borrow().tabs()[1].id;

    test.window.submit_address(typed);
    test.window
        .open_file_location(background, &fixture.uri_of("Notes 10.txt"));
    wait_until("both lookups to answer", || {
        test.window.answered_activations() == 2
    });
    let mut launches = test.context.recorded_launches();
    launches.sort();
    assert_eq!(
        launches,
        [fixture.uri_of("Notes 10.txt"), fixture.uri_of("Notes 2.txt")]
    );

    test.window.submit_address(typed);
    test.window.switch_tab(background);
    test.window.switch_tab(front);
    wait_until("the lookup to answer", || test.window.answered_activations() == 3);
    assert_eq!(
        test.context.recorded_launches().len(),
        2,
        "the dropped lookup opens nothing"
    );
}

/// parity: NAV-033, NAV-040
#[gtk::test]
fn a_typed_file_path_opens_the_file_and_leaves_the_folder_and_history() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let file = fixture.path("Notes 2.txt");
    test.window
        .submit_address(file.to_str().expect("fixture paths are UTF-8"));
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 2.txt")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(!can_go_back(&test), "opening a file adds no history entry");
    assert_eq!(test.window.load_error(), None);
}

/// parity: NAV-040
#[gtk::test]
fn a_tab_opened_on_a_file_shows_its_folder_and_opens_the_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Notes 10.txt"));
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.window.load_error(), None);
    assert_eq!(test.names(), STANDARD_NAMES);
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 10.txt")]);
}

#[gtk::test]
fn a_folder_replaced_by_a_file_is_never_opened_by_a_reload() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Swap")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri_of("Swap"));
    fs::remove_dir(fixture.path("Swap")).expect("the subfolder is empty");
    fixture.write("Swap");
    test.window.refresh();
    wait_until("the tab to show the parent folder", || {
        test.window.current_uri() == Some(fixture.uri()) && !test.window.is_loading()
    });
    wait_for(std::time::Duration::from_millis(100));
    assert!(
        test.context.recorded_launches().is_empty(),
        "a reload never starts an application"
    );
}

/// parity: NAV-034
#[gtk::test]
fn a_typed_page_title_prefers_a_folder_of_that_name() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Network")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri());
    test.window.submit_address("Network");
    wait_until("the folder called Network", || {
        test.window.current_uri() == Some(fixture.uri_of("Network"))
    });
    test.window.submit_address("This PC");
    wait_until("the This PC page", || {
        test.window.current_uri().as_deref() == Some(Page::ThisPc.uri())
    });
}

#[gtk::test]
fn command_line_locations_open_in_the_current_tab_then_in_new_tabs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let locations = vec![
        fixture.uri_of("Documents"),
        fixture.uri(),
        fixture.uri_of("Notes 2.txt"),
    ];
    test.window.open_locations(locations);
    wait_until("every location to open", || {
        test.window.tab_count() == 2 && !test.context.recorded_launches().is_empty()
    });
    test.wait_for_listing("the new tab");
    let session = test.window.imp().session.borrow();
    let uris: Vec<&str> = session.tabs().iter().map(Tab::uri).collect();
    assert_eq!(uris, [fixture.uri_of("Documents"), fixture.uri()]);
    assert!(
        session.tabs()[0].history.can_go_back(),
        "the first location keeps the tab's history"
    );
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Notes 2.txt")]);
}
