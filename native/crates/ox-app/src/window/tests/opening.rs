// SPDX-License-Identifier: AGPL-3.0-only
//! Opening typed addresses, command-line locations and files. Files are
//! recorded instead of started (see `AppContext::record_launches`).

use std::fs;

use gtk::subclass::prelude::*;

use crate::locations::Page;
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::Tab;

fn can_go_back(test: &TestWindow) -> bool {
    let session = test.window.imp().session.borrow();
    session.active().is_some_and(|tab| tab.history.can_go_back())
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

/// Locations another program passes (`openxplorer %U`, as GNOME does for
/// the mount root of inserted media when `OpenXplorer` is the folder
/// handler) open in the running instance's active window.
///
/// parity: INT-024
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
