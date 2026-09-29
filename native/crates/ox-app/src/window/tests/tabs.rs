// SPDX-License-Identifier: AGPL-3.0-only
//! Tabs: opening in front or behind, closing, switching and their history.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::support::{click_at, Release};
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::address_bar::AddressMode;
use crate::folder_view::item::FileItem;
use crate::window::session::{TabId, TabPlacement};
use ox_core::network::NetworkError;

fn tab_ids(test: &TestWindow) -> Vec<TabId> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

fn is_listed(test: &TestWindow, id: TabId) -> bool {
    let session = test.window.imp().session.borrow();
    session.tab(id).is_some_and(|tab| tab.listing_state.is_listed())
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
    assert_eq!(test.active_tab(), Some(right));
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
    assert_eq!(test.active_tab(), Some(left));
    test.activate("next-tab", None);
    assert_eq!(test.active_tab(), Some(middle));
    test.activate("previous-tab", None);
    test.activate("previous-tab", None);
    assert_eq!(test.active_tab(), Some(tab_ids(&test)[2]));
}

/// parity: TAB-009
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

/// parity: TAB-021
#[gtk::test]
fn shift_middle_click_opens_the_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("open-tab", Some(&fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// A tab keeps its own scroll position and selection; showing it resets
/// type-to-select and the search box.
///
/// parity: TAB-008, TAB-057
#[gtk::test]
fn switching_back_to_a_tab_restores_its_scroll_position() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let last = pane.model().n_items() - 1;
    pane.reveal(last);
    wait_until("the view to scroll", || pane.scroll_position() > 0.0);
    wait_for(std::time::Duration::from_millis(100));
    let scrolled = pane.scroll_position();
    let first = tab_ids(&test)[0];
    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    assert!(pane.scroll_position() < 1.0, "a new tab starts at the top");
    test.window.search_box().entry().set_text("file");
    test.window.folder_model().select_only(0);
    let second_selection = test.selected_names();
    pane.reveal(1);
    wait_until("the second tab's focused item", || pane.focused_position() == Some(1));
    test.activate_tab(first);
    wait_until("the first tab's scroll position", || {
        (pane.scroll_position() - scrolled).abs() < 1.0
    });
    wait_until("the first tab's focused item", || pane.focused_position() == Some(last));
    assert_eq!(test.window.search_box().entry().text().as_str(), "");
    assert!(
        test.selected_names().is_empty(),
        "the first tab had nothing selected"
    );
    test.activate("next-tab", None);
    assert_eq!(
        test.selected_names(),
        second_selection,
        "each tab keeps its selection"
    );
    wait_until("the second tab's focused item again", || pane.focused_position() == Some(1));
}

/// The tab widget at `index` of the strip.
fn tab_widget(test: &TestWindow, index: usize) -> gtk::Widget {
    crate::window::widget_tree::children(&test.window.tab_strip().tab_list())
        .nth(index)
        .expect("the window shows the tab")
}

/// A click or Enter on a tab shows it, and ends editing the address.
///
/// parity: TAB-004
#[gtk::test]
fn a_click_or_enter_on_a_tab_shows_it() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [left, middle, _] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate("location", None);
    assert_eq!(test.window.address_bar().mode(), AddressMode::Entry);

    click_at(
        &tab_widget(&test, 0),
        gtk::gdk::BUTTON_PRIMARY,
        (4.0, 4.0),
        Release::Released,
    );
    let after_click = test.active_tab();
    let address_mode = test.window.address_bar().mode();
    let middle_tab = tab_widget(&test, 1);
    let keys = middle_tab
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("a tab takes keys");
    let handled = keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Return.into_glib(),
            &0_u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    );

    assert_eq!(after_click, Some(left));
    assert_eq!(
        address_mode,
        AddressMode::Crumbs,
        "switching ends address editing"
    );
    assert!(handled);
    assert_eq!(test.active_tab(), Some(middle));
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
    test.window.search_box().entry().set_text("Notes 2");
    wait_until("the active filter", || test.names() == ["Notes 2.txt"]);
    let background = tab_ids(&test)[1];
    test.activate_tab_close(background);
    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.window.search_box().entry().text().as_str(), "Notes 2");
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
    let tab_list = test.window.tab_strip().tab_list();
    assert_eq!(tab_list.accessible_role(), gtk::AccessibleRole::TabList);
    assert!(gtk::test_accessible_has_property(
        &tab_list,
        gtk::AccessibleProperty::Label
    ));
    let tabs: Vec<gtk::Box> = descendants::<gtk::Box>(&tab_list)
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

/// parity: TAB-001
#[gtk::test]
fn the_new_tab_button_opens_the_home_folder_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let button = &test.window.imp().new_tab_button;
    assert_eq!(button.tooltip_text().as_deref(), Some("New tab (Ctrl+T)"));
    button.emit_clicked();
    test.wait_for_listing("the home folder");
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    assert_eq!(test.window.current_uri(), Some(home));
    assert_eq!(test.window.tab_count(), 2);
}

/// Ctrl+Tab and Ctrl+T stay out of text fields, and while the active
/// tab's Properties are open only the tab keys work.
///
/// parity: TAB-005
#[gtk::test]
fn tab_keys_stay_out_of_text_fields_but_switch_away_from_properties() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();
    assert!(test.window.tab_keys_apply());
    assert!(test.window.new_tab_key_applies());

    test.window.search_box().entry().grab_focus();
    let in_search = (test.window.tab_keys_apply(), test.window.new_tab_key_applies());
    test.window.folder_pane().focus_view();
    test.activate("properties", None);
    test.wait_for_dialog("the Properties dialog");

    assert_eq!(in_search, (false, false), "a text field keeps the keys");
    assert!(test.window.tab_keys_apply(), "Ctrl+Tab leaves the dialog's tab");
    assert!(
        !test.window.new_tab_key_applies(),
        "other keys wait for the dialog"
    );
}

/// Many tabs keep the strip within 70% of the window; they shrink and the
/// strip scrolls without a scrollbar.
///
/// parity: TAB-019
#[gtk::test]
fn many_tabs_keep_the_strip_within_seventy_percent_of_the_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for _ in 0..12 {
        test.window.add_tab(&fixture.uri()).expect("valid folder");
    }
    let strip = test.window.tab_strip();
    let limit = test.window.width() * 7 / 10;
    wait_until("the strip to fit its share", || {
        strip.width() > 0 && strip.width() <= limit
    });
    // The border box: the content width leaves out the tab's padding.
    let tab = tab_widget(&test, 0);
    let width = tab.compute_bounds(&tab).expect("a shown tab").width();
    assert!(width < 215.0, "the tabs shrank");
    assert!(width >= 100.0, "down to the narrowest tab");
}

/// A change to a background tab's folder lists it again behind, so it is
/// current when it comes back, with its selection kept.
///
/// parity: TAB-056
#[gtk::test]
fn a_background_tab_shows_changes_to_its_folder_and_keeps_its_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    let first = tab_ids(&test)[0];
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");

    std::fs::write(fixture.root().join("Added later.txt"), b"new").expect("a new file");
    wait_until("the background tab to list the new file", || {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(first).expect("the first tab is open");
        let mut items = tab.store.iter::<FileItem>().filter_map(Result::ok);
        items.any(|item| item.entry().name == "Added later.txt")
    });
    assert_ne!(test.active_tab(), Some(first), "it changed behind");
    test.activate_tab(first);

    assert!(test.names().iter().any(|name| name == "Added later.txt"));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// A change to a network folder in a background tab only marks the tab;
/// it is listed again when shown, so no sign-in appears behind.
///
/// parity: TAB-056
#[gtk::test]
fn a_background_network_tab_is_listed_again_only_when_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    test.window
        .open_tab("smb://example.invalid/share", TabPlacement::Background)
        .expect("an SMB share");
    let share = tab_ids(&test)[1];
    let tab_state = |test: &TestWindow| {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(share).expect("the share's tab is open");
        (tab.changed_while_hidden, tab.listing_state.is_listing())
    };
    {
        // As if the share had been listed when it was last shown.
        let mut session = test.window.imp().session.borrow_mut();
        let tab = session.tab_mut(share).expect("the share's tab is open");
        tab.listing_state.begin();
        tab.listing_state.finish();
    }

    test.window.folder_changed(share);
    let behind = tab_state(&test);
    test.activate_tab(share);

    assert_eq!(behind, (true, false), "marked, not listed");
    assert_eq!(tab_state(&test), (false, true), "listed once shown");
}
