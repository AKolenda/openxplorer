// SPDX-License-Identifier: AGPL-3.0-only
//! Opening typed addresses, command-line locations and files. Files are
//! recorded instead of started (see `AppContext::record_launches`).

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::integration::{Tool, NO_DIFF_TOOL, NO_SEARCH_TOOL};
use crate::locations::Page;
use crate::test_support::harness::{application, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::Tab;
use crate::window::tests::file_ops_support::{open_dialog, select_names};

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

/// The URIs of `test`'s tabs, left to right.
fn tab_uris(test: &TestWindow) -> Vec<String> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
}

/// An item is read again when it is opened: one deleted since the
/// folder was listed says why in "Could not open the item".
///
/// parity: OPEN-001
#[gtk::test]
fn an_item_is_read_again_when_opened_and_a_failure_is_a_dialog() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let position = test.position_of("Notes 2.txt");
    fs::remove_file(fixture.path("Notes 2.txt")).expect("the fixture file can be removed");

    test.window.activate_item(position);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Could not open the item");
    assert!(test.context.recorded_launches().is_empty(), "nothing opened");
    dialog.press("OK");
}

/// A folder opened just before another tab came to the front opens in
/// the tab it was opened from; one whose tab moved on is dropped.
///
/// parity: OPEN-004
#[gtk::test]
fn an_opened_folder_stays_with_the_tab_that_asked_for_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.activate_item(test.position_of("Documents"));
    test.window
        .add_tab(&fixture.uri())
        .expect("the fixture is a folder");
    wait_until("the first tab to open Documents", || {
        tab_uris(&test)[0] == fixture.uri_of("Documents")
    });
    assert_eq!(tab_uris(&test)[1], fixture.uri(), "the tab in front stays");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));

    test.wait_for_listing("the second tab");
    test.window.activate_item(test.position_of("Documents"));
    test.window.navigate_or_report(&fixture.uri_of("Documents"));
    test.window.navigate_or_report(&fixture.uri());
    wait_for(std::time::Duration::from_millis(300));
    assert_eq!(
        tab_uris(&test)[1],
        fixture.uri(),
        "a tab that moved on drops the result"
    );
}

/// Open in Terminal says in the message line why a location has no
/// terminal, such as a server listing, and starts nothing.
///
/// parity: OPEN-017
#[gtk::test]
fn open_in_terminal_explains_a_refusal_in_the_message_line() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("open-in-terminal-of", Some("smb://studio-nas/"));

    wait_until("the refusal", || {
        test.window.shown_message()
            == "Open a network share first. A server listing is not a terminal directory."
    });
}

/// "Open in new tab" of a folder, a pin, a network entry or a share card
/// opens that folder in a new tab in front.
///
/// parity: OPEN-016
#[gtk::test]
fn open_in_new_tab_opens_the_folder_in_a_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("open-tab", Some(&fixture.uri_of("Documents")));

    assert_eq!(tab_uris(&test), [fixture.uri(), fixture.uri_of("Documents")]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// Shift+F4 opens a terminal in the folder shown and Shift+Alt+F4 one per
/// folder of the selection, a file standing for its folder; more than
/// five are asked about first, and Cancel opens none.
///
/// parity: OPEN-021
#[gtk::test]
fn open_terminal_here_opens_one_per_folder_and_asks_for_many() {
    let fixture = Fixture::standard();
    let names = ["A", "B", "C", "D", "E", "F"];
    for name in names {
        fs::create_dir(fixture.path(name)).expect("fixture subfolder");
    }
    let test = TestWindow::open(&fixture.uri());
    let keys = |action: &str| application().accels_for_action(&format!("win.{action}"));
    assert_eq!(keys("open-terminal"), ["<Shift>F4"]);
    assert_eq!(keys("open-terminal-here"), ["<Shift><Alt>F4"]);

    select_names(&test, &["Notes 2.txt", "Résumé.txt", "Documents"]);
    assert_eq!(
        test.window.terminal_folders(),
        [fixture.uri_of("Documents"), fixture.uri()]
    );

    select_names(&test, &names);
    test.activate("open-terminal-here", None);
    let dialog = open_dialog(&test);
    assert_eq!(
        dialog.message_text(),
        "Are you sure you want to open 6 terminals?"
    );
    dialog.press("Cancel");
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(test.window.shown_message(), "", "no terminal was started");
}

/// Opening a `.desktop` link to a folder browses that folder.
///
/// parity: OPEN-009
#[gtk::test]
fn a_desktop_link_to_a_folder_opens_the_folder() {
    let fixture = Fixture::standard();
    let link = format!(
        "[Desktop Entry]\nType=Link\nName=Documents\nURL={}\n",
        fixture.uri_of("Documents")
    );
    fs::write(fixture.path("Documents link.desktop"), link).expect("the fixture is writable");
    let test = TestWindow::open(&fixture.uri());

    test.window
        .activate_item(test.position_of("Documents link.desktop"));

    wait_until("the linked folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    assert!(
        test.context.recorded_launches().is_empty(),
        "the link file is not opened"
    );
}

/// Compare files hands the two selected files to the installed comparison
/// tool, or says that none is installed; the item menu offers it for two
/// files when a tool is there.
///
/// parity: OPEN-023
#[gtk::test]
fn compare_files_hands_two_files_to_the_comparison_tool() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);

    test.activate("compare-files", None);

    if Tool::Diff.installed().is_some() {
        let pair = format!(
            "{} {}",
            fixture.uri_of("Notes 2.txt"),
            fixture.uri_of("Notes 10.txt")
        );
        assert_eq!(test.context.recorded_launches(), [pair]);
    } else {
        assert_eq!(test.window.shown_message(), NO_DIFF_TOOL);
    }
}

/// Ctrl+Shift+F opens the installed search tool at the folder shown, or
/// says that none is installed.
///
/// parity: OPEN-024
#[gtk::test]
fn ctrl_shift_f_opens_the_search_tool_at_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let keys = application().accels_for_action("win.search-tool");
    assert_eq!(keys, ["<Shift><Control>f"]);

    test.activate("search-tool", None);

    if Tool::Search.installed().is_some() {
        assert_eq!(test.context.recorded_launches(), [fixture.uri()]);
    } else {
        assert_eq!(test.window.shown_message(), NO_SEARCH_TOOL);
    }
}

/// When no application opens a file's type, "Could not open the item"
/// offers to find one in Software, while Software is installed.
///
/// parity: OPEN-010
#[gtk::test]
fn a_file_without_an_application_offers_a_search_in_software() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut entry = crate::test_support::file_entry("scene.blend");
    entry.content_type = Some("application/x-blender".to_owned());
    let reason = ox_core::integration::OpenError::NoApplication.to_string();

    test.window.report_open_failure(&reason, &entry);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.message_text(), reason);
    let offers_software = dialog
        .button_labels()
        .contains(&"Find an app in Software".to_owned());
    assert_eq!(offers_software, crate::window::software_search::is_available());
    dialog.press("OK");
}
