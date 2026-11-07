// SPDX-License-Identifier: AGPL-3.0-only
//! Search results as a step of the tab's history, as in Windows 11 File
//! Explorer: "Search Results in <folder>" in the tab, the title and the
//! address bar, and Back and Forward between the folder, the results and
//! a folder opened from them.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::history::navigation_button;
use super::support::middle_click_at;
use crate::test_support::harness::{capture, settle, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::Direction;

/// What a search for "notes" in the standard fixture finds, with one
/// more match in Documents.
const RESULTS: [&str; 3] = ["Notes 2.txt", "Notes 10.txt", "notes inside.txt"];

/// The fixture folder's name, as its tab is titled.
fn root_title(fixture: &Fixture) -> String {
    fixture
        .root()
        .file_name()
        .expect("the fixture folder has a name")
        .to_string_lossy()
        .into_owned()
}

/// A window on the standard fixture, with a match for "notes" in
/// Documents too.
fn window_with_notes(fixture: &Fixture) -> TestWindow {
    fs::write(fixture.path("Documents/notes inside.txt"), b"x").expect("fixture file");
    TestWindow::open(&fixture.uri())
}

impl TestWindow {
    /// The active tab's title in the tab strip.
    fn active_tab_title(&self) -> String {
        let active = self.window.imp().session.borrow().active_id();
        let active = active.expect("the window has a tab");
        let view = self.window.tab_strip().tab_view(active);
        view.expect("the active tab is shown").title
    }

    /// Every tab's title in the tab strip, in order.
    fn tab_titles(&self) -> Vec<String> {
        let ids: Vec<_> = {
            let session = self.window.imp().session.borrow();
            session.tabs().iter().map(|tab| tab.id).collect()
        };
        let strip = self.window.tab_strip();
        let views = ids
            .into_iter()
            .map(|id| strip.tab_view(id).expect("the tab is shown"));
        views.map(|view| view.title).collect()
    }

    /// The labels of the address bar's crumbs.
    fn crumb_labels(&self) -> Vec<String> {
        let buttons = self.window.address_bar().crumb_buttons();
        let labels = buttons.iter().filter_map(ButtonExt::label);
        labels.map(|label| label.to_string()).collect()
    }

    /// Whether the last crumb is drawn inside the address bar, its
    /// crumbs scrolled to it.
    fn last_crumb_is_in_view(&self) -> bool {
        let bar = self.window.address_bar();
        let crumbs = bar.crumb_buttons();
        let bounds = crumbs.last().and_then(|crumb| crumb.compute_bounds(bar));
        bounds.is_some_and(|bounds| {
            bounds.x() >= 0.0 && f64::from(bounds.x() + bounds.width()) <= f64::from(bar.width())
        })
    }

    /// The search box's text.
    fn search_text(&self) -> String {
        self.window.search_box().entry().text().to_string()
    }
}

/// A search is a step after its folder: the tab, the window title, the
/// address bar and the history menus say "Search Results in <folder>";
/// Back returns to the folder with the box empty, Forward to the results,
/// and Back from a folder opened from them shows them again.
///
/// parity: SRCH-042, NAV-001, NAV-006
#[gtk::test]
fn back_and_forward_walk_between_a_folder_its_results_and_a_folder_opened_from_them() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    let results = format!("Search Results in {root}");

    test.search_for("notes");

    assert_eq!(test.names(), RESULTS);
    assert_eq!(test.active_tab_title(), results);
    let title = test.window.title().map(|title| title.to_string());
    assert_eq!(title, Some(format!("{results} — OpenXplorer")));
    let crumbs = test.crumb_labels();
    assert_eq!(crumbs[crumbs.len() - 2..], [root.clone(), results.clone()]);
    wait_until("the results' crumb to scroll into view", || {
        test.last_crumb_is_in_view()
    });
    capture(&test.window, "search-results-step.png");

    test.activate("back", None);

    test.wait_for_listing("the folder");
    wait_until("the folder's items", || test.names() == STANDARD_NAMES);
    assert_eq!(test.search_text(), "");
    assert_eq!(test.active_tab_title(), root);
    assert_eq!(test.crumb_labels().last(), Some(&root));

    test.activate("forward", None);

    wait_until("the results again", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
    assert_eq!(test.active_tab_title(), results);

    // As activating a folder result does.
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the folder opened from the results");
    assert_eq!(test.search_text(), "");
    assert_eq!(test.active_tab_title(), "Documents");
    let back_menu = test.window.history_menu_entries(Direction::Backward);
    assert_eq!(back_menu.len(), 2, "the results, then their folder");
    let back = test
        .window
        .popup_history_menu(test.window.address_bar().upcast_ref(), Direction::Backward)
        .expect("Back has a menu");
    assert_eq!(back.row_labels(), [results.clone(), root.clone()]);
    back.popdown();

    test.activate("back", None);

    wait_until("the results once more", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
    assert_eq!(test.active_tab_title(), results);
}

/// Emptying the box on results goes back to the folder, as Back does, and
/// Forward returns to them.
///
/// parity: SRCH-042, SRCH-001
#[gtk::test]
fn emptying_the_box_on_results_goes_back_to_the_folder() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    test.search_for("notes");

    test.window.search_box().entry().emit_stop_search();

    wait_until("the folder", || {
        test.names() == STANDARD_NAMES && test.active_tab_title() == root
    });
    assert!(!test.window.shows_search_step());
    test.activate("forward", None);
    wait_until("the results again", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
}

/// A tab on results shows them again when it comes back to the front.
///
/// parity: SRCH-042
#[gtk::test]
fn a_tab_on_results_shows_them_again_when_it_comes_back() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    test.search_for("notes");

    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    assert_eq!(test.search_text(), "");
    assert_eq!(test.names(), STANDARD_NAMES);
    test.activate("previous-tab", None);

    wait_until("the first tab's results", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
}

/// A tab on results comes back with the results it had selected still
/// selected and scrolled where they were, though they are searched again.
///
/// parity: SRCH-042
#[gtk::test]
fn a_tab_on_results_keeps_its_selection_and_scroll_position() {
    let fixture = Fixture::standard();
    for number in 1..=80 {
        fs::write(fixture.path(&format!("notes {number:02}.txt")), b"x").expect("fixture file");
    }
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    wait_until("every result", || test.names().len() == 83);
    let pane = test.window.folder_pane();
    let chosen = vec![fixture.uri_of("notes 70.txt")];
    pane.model().select_uris(&chosen);
    pane.restore_scroll_position(300.0);
    wait_until("the results to scroll", || {
        (pane.scroll_position() - 300.0).abs() < 1.0
    });

    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    test.activate("previous-tab", None);

    let pane = test.window.folder_pane();
    wait_until("the results, selected and scrolled as they were", || {
        test.names().len() == 83
            && pane.model().selected_uris() == chosen
            && (pane.scroll_position() - 300.0).abs() < 1.0
    });
    assert_eq!(test.search_text(), "notes");
}

/// A middle-click on Back to search results, or on them in Back's menu,
/// opens them in a tab behind, "Search Results in <folder>" after the
/// folder, not the folder alone; the tab searches again when it comes to
/// the front, and its Back returns to the folder.
///
/// parity: NAV-007, SRCH-042
#[gtk::test]
fn a_middle_click_on_back_to_results_opens_the_results_in_a_tab() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    let results = format!("Search Results in {root}");
    test.search_for("notes");
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the folder opened from the results");

    let back = navigation_button(&test, "back");
    middle_click_at(&back, (1.0, 1.0));
    let menu = test
        .window
        .popup_history_menu(back.upcast_ref(), Direction::Backward)
        .expect("Back has a menu");
    wait_until("the menu to show", || {
        menu.rows().iter().all(|row| row.height() > 0)
    });
    menu.middle_click_row(&results);
    menu.popdown();

    assert_eq!(test.tab_titles(), ["Documents", &results, &results]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.activate("next-tab", None);
    wait_until("the results in the new tab", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
    test.activate("back", None);
    test.wait_for_listing("the folder");
    wait_until("the folder's items", || test.names() == STANDARD_NAMES);
    assert_eq!(test.active_tab_title(), root);
}

/// Back and Forward show each step as it was, also while the strip's pin
/// keeps searches when changing folders: Back from results shows the
/// folder, not the kept search.
///
/// parity: SRCH-042, SRCH-005
#[gtk::test]
fn back_from_results_shows_the_folder_while_the_pin_keeps_searches() {
    let fixture = Fixture::standard();
    let test = window_with_notes(&fixture);
    let root = root_title(&fixture);
    test.search_for("notes");
    test.window.search_strip().set_keeps_search(true);

    test.activate("back", None);

    test.wait_for_listing("the folder");
    wait_until("the folder's items", || test.names() == STANDARD_NAMES);
    // Past the idle callback through which the pin would search again.
    settle();
    assert_eq!(test.search_text(), "");
    assert_eq!(test.names(), STANDARD_NAMES);
    assert_eq!(test.active_tab_title(), root);
}

/// A folder that may not be read, opened from search results, keeps the
/// tab on the results, as Windows keeps it where it was: the search is
/// shown again, not the folder without it. Tests run as a normal user:
/// root reads the folder anyway.
///
/// parity: SRCH-042, VIEW-047
#[gtk::test]
fn a_denied_folder_opened_from_results_returns_to_the_results() {
    use std::os::unix::fs::PermissionsExt;

    /// Gives the folder its permissions back, also when the test fails.
    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Private")).expect("a folder");
    let _unlock = Unlock(fixture.path("Private"));
    fs::set_permissions(fixture.path("Private"), fs::Permissions::from_mode(0o000)).expect("mode");
    if fs::read_dir(fixture.path("Private")).is_ok() {
        return; // Running as root, which reads anything.
    }
    let test = window_with_notes(&fixture);
    test.search_for("notes");
    assert_eq!(test.names(), RESULTS);

    test.window
        .navigate(&fixture.uri_of("Private"))
        .expect("valid folder");

    let dialog = super::file_ops_support::open_dialog(&test);
    assert_eq!(dialog.title_text(), "Location is not available");
    dialog.press("OK");
    super::file_ops_support::wait_for_no_dialog(&test);
    wait_until("the results again", || test.names() == RESULTS);
    assert_eq!(test.search_text(), "notes");
    assert!(
        !test
            .window
            .imp()
            .session
            .borrow()
            .active()
            .expect("a tab")
            .history
            .can_go_forward(),
        "the folder is not in the history"
    );
}
