// SPDX-License-Identifier: AGPL-3.0-only
//! Tabs: opening in front or behind, closing, switching and their history.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::session::TabId;

fn tab_ids(test: &TestWindow) -> Vec<TabId> {
    let session = test.window.imp().session.borrow();
    session.tabs.iter().map(|tab| tab.id).collect()
}

fn is_listed(test: &TestWindow, id: TabId) -> bool {
    let session = test.window.imp().session.borrow();
    session.tab(id).is_some_and(|tab| tab.loaded)
}

fn open_three_tabs(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    for uri in [fixture.uri_of("Documents"), fixture.uri()] {
        test.window.add_tab(&uri).expect("valid folder");
        test.wait_for_listing("the new tab");
    }
    test
}

/// parity: TAB-002
#[gtk::test]
fn closing_the_active_tab_shows_the_tab_to_its_right() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [_, middle, right] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate_tab(middle);
    test.activate("close-tab", None);
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.imp().session.borrow().active, Some(right));
}

/// parity: TAB-005
#[gtk::test]
fn ctrl_tab_cycles_through_the_tabs_and_wraps() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [left, middle, _] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate("next-tab", None);
    assert_eq!(test.window.imp().session.borrow().active, Some(left));
    test.activate("next-tab", None);
    assert_eq!(test.window.imp().session.borrow().active, Some(middle));
    test.activate("previous-tab", None);
    test.activate("previous-tab", None);
    assert_eq!(test.window.imp().session.borrow().active, Some(tab_ids(&test)[2]));
}

#[gtk::test]
fn a_background_tab_leaves_the_current_tab_alone_and_lists_when_first_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.activate("open-tab-background", Some(&fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the source tab stays in front"
    );
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "the source tab keeps its selection"
    );
    let background = tab_ids(&test)[1];
    wait_for(std::time::Duration::from_millis(50));
    assert!(
        !is_listed(&test, background),
        "a background tab is listed only when shown"
    );
    test.activate_tab(background);
    test.wait_for_listing("the background tab");
    assert!(is_listed(&test, background));
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

#[gtk::test]
fn shift_middle_click_opens_the_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("open-tab", Some(&fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

#[gtk::test]
fn switching_back_to_a_tab_restores_its_scroll_position() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    let content = test.window.content();
    let last = content.model.n_items() - 1;
    content.reveal(last);
    wait_until("the view to scroll", || content.scroll_position() > 0.0);
    wait_for(std::time::Duration::from_millis(100));
    let scrolled = content.scroll_position();
    let first = tab_ids(&test)[0];
    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    assert!(content.scroll_position() < 1.0, "a new tab starts at the top");
    test.activate_tab(first);
    wait_until("the first tab's scroll position", || {
        (content.scroll_position() - scrolled).abs() < 1.0
    });
}

/// parity: NAV-001, NAV-005, NAV-010
#[gtk::test]
fn back_forward_and_up_walk_the_tab_history() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(!test.window.is_action_enabled("back"));
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the subfolder");
    test.activate("back", None);
    test.wait_for_listing("the folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.window.is_action_enabled("forward"));
    test.activate("forward", None);
    test.wait_for_listing("the subfolder again");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.activate("up", None);
    test.wait_for_listing("the parent folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.names(), STANDARD_NAMES);
}

#[gtk::test]
fn closing_a_background_tab_keeps_the_active_tab_and_its_filter() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    test.activate("previous-tab", None);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    test.window.chrome().search.set_text("Notes 2");
    wait_until("the active filter", || test.names() == ["Notes 2.txt"]);
    let background = tab_ids(&test)[1];
    test.activate_tab_close(background);
    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.window.chrome().search.text().as_str(), "Notes 2");
    assert_eq!(test.names(), ["Notes 2.txt"]);
}

/// parity: TAB-010
#[gtk::test]
fn tabs_are_announced_as_tabs_with_their_selected_state() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    let tab_list = test.window.chrome().tabs.tab_list();
    assert_eq!(tab_list.accessible_role(), gtk::AccessibleRole::TabList);
    assert!(gtk::test_accessible_has_property(
        tab_list,
        gtk::AccessibleProperty::Label
    ));
    let tabs: Vec<gtk::Button> = descendants::<gtk::Button>(tab_list)
        .into_iter()
        .filter(|button| button.accessible_role() == gtk::AccessibleRole::Tab)
        .collect();
    assert_eq!(tabs.len(), 2);
    let titles: Vec<Option<String>> = tabs
        .iter()
        .map(|tab| tab.tooltip_text().map(|text| text.to_string()))
        .collect();
    let folder_address = fixture.root().display().to_string();
    assert_eq!(titles[0].as_deref(), Some(folder_address.as_str()));
    for tab in &tabs {
        assert!(gtk::test_accessible_has_state(
            tab,
            gtk::AccessibleState::Selected
        ));
    }
}

impl TestWindow {
    /// Shows tab `id`, as clicking it does.
    fn activate_tab(&self, id: TabId) {
        WidgetExt::activate_action(&self.window, "win.select-tab", Some(&id.to_variant()))
            .expect("the window has the action");
    }

    /// Closes tab `id`, as its close button does.
    fn activate_tab_close(&self, id: TabId) {
        WidgetExt::activate_action(&self.window, "win.close-tab-by-id", Some(&id.to_variant()))
            .expect("the window has the action");
    }
}

#[gtk::test]
fn a_new_tab_opens_the_home_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("new-tab", None);
    test.wait_for_listing("the home folder");
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    assert_eq!(test.window.current_uri(), Some(home));
    assert_eq!(test.window.tab_count(), 2);
}
