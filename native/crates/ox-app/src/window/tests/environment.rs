// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar, the landing pages, pins and the details pane.

use std::time::Duration;

use gtk::prelude::*;
use ox_core::settings::{PinRequest, Settings};

use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::window::details_pane::PANE_WIDTH;
use crate::window::landing;

fn property<'a>(properties: &'a [(String, String)], key: &str) -> Option<&'a str> {
    properties
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

#[gtk::test]
fn the_sidebar_starts_with_home_and_lists_this_pc_and_network() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let labels = test.window.sidebar().labels();
    assert_eq!(labels.first().map(String::as_str), Some("Home"));
    for expected in ["This PC", "Local Disk", "Network"] {
        assert!(
            labels.contains(&expected.to_owned()),
            "{expected} is in the sidebar"
        );
    }
}

#[gtk::test]
fn sidebar_rows_are_named_by_their_label_and_described_by_their_address() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let rows = descendants::<gtk::ListBoxRow>(&test.window.sidebar().list);
    assert!(!rows.is_empty());
    assert_eq!(
        rows.len(),
        test.window.sidebar().labels().len(),
        "no separator rows to land on"
    );
    for row in &rows {
        assert!(gtk::test_accessible_has_property(
            row,
            gtk::AccessibleProperty::Label
        ));
        assert!(gtk::test_accessible_has_property(
            row,
            gtk::AccessibleProperty::Description
        ));
    }
}

#[gtk::test]
fn the_home_row_is_selected_in_the_home_folder() {
    let fixture = Fixture::standard();
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    let test = TestWindow::open(&fixture.uri());
    test.window.navigate(&home).expect("the home folder");
    test.wait_for_listing("the home folder");
    let selected = test.window.sidebar().list.selected_row();
    assert_eq!(selected.map(|row| row.index()), Some(0), "Home is the first row");
}

#[gtk::test]
fn this_pc_lists_quick_access_devices_and_network_locations() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = &test.window.content().landing;
    assert_eq!(
        landing::section_titles(landing),
        ["Quick access", "Devices and drives", "Network locations"]
    );
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    assert_eq!(landing::section_titles(landing), ["Connected & saved locations"]);
}

#[gtk::test]
fn the_status_bar_says_ready_on_a_page_and_counts_a_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(Page::ThisPc.uri());
    assert_eq!(
        test.window.chrome().status.texts(),
        ("Ready".to_owned(), String::new())
    );
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    test.window.folder_model().select_only(0);
    let (count, selection) = test.window.chrome().status.texts();
    assert_eq!(count, "4 items");
    assert_eq!(selection, "1 selected");
}

/// parity: SIDE-005, SIDE-007
#[gtk::test]
fn pinning_the_current_folder_adds_it_to_quick_access() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("pin-folder", None);
    wait_until("the pin to be saved", || {
        test.window
            .sidebar()
            .labels()
            .contains(&"Example projects".to_owned())
    });
    let pins = test.context.settings_data().pins;
    assert_eq!(
        pins.iter().map(|pin| pin.uri.as_str()).collect::<Vec<_>>(),
        [fixture.uri()]
    );
    test.activate("pin-folder", None);
    let message = test.window.chrome().message.text();
    assert_eq!(message.as_str(), "Already pinned to Quick access.");
}

#[gtk::test]
fn the_details_pane_keeps_its_width_for_long_names() {
    let fixture = Fixture::standard();
    let long_name = format!("{}.txt", "A very long file name ".repeat(8));
    fixture.write(&long_name);
    let test = TestWindow::open(&fixture.uri());
    if !test.window.details_pane().root.is_visible() {
        test.activate("details-pane", None);
    }
    let position = test.names().iter().position(|name| *name == long_name);
    let position = position.expect("the long name is listed");
    test.window
        .folder_model()
        .select_only(u32::try_from(position).expect("a short listing"));
    let pane = &test.window.details_pane().root;
    wait_until("the pane to be laid out", || pane.width() > 0);
    settle_layout(&test);
    // Its stylesheet adds 44 pixels of padding and a 1-pixel border.
    let widest = PANE_WIDTH + 45;
    assert!(pane.width() <= widest, "the pane is {} pixels wide", pane.width());
}

/// Waits for the window to lay out what just changed.
fn settle_layout(test: &TestWindow) {
    test.window.queue_resize();
    wait_for(Duration::from_millis(100));
}

#[gtk::test]
fn the_details_pane_lists_a_file_and_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.details_pane();
    let folder = pane.shown_properties();
    assert_eq!(property(&folder, "Items"), Some("5"), "hidden items count too");
    assert_eq!(property(&folder, "Storage"), Some("This computer"));
    test.window.folder_model().select_only(1);
    let file = pane.shown_properties();
    let keys: Vec<&str> = file.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["Type", "Size", "Modified", "Location"]);
    let folder_address = fixture.root().display().to_string();
    assert_eq!(property(&file, "Location"), Some(folder_address.as_str()));
}

/// parity: SIDE-022
#[gtk::test]
fn pins_another_process_saved_appear_after_a_refresh() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut python_app = Settings::open(test.settings_directory());
    let pin = PinRequest::new(fixture.uri_of("Documents"), "Pinned elsewhere");
    python_app
        .pin_many(&[pin], None, None)
        .expect("the settings file takes a pin");
    test.activate("refresh", None);
    wait_until("the pin to reach the sidebar", || {
        let labels = test.window.sidebar().labels();
        labels.contains(&"Pinned elsewhere".to_owned())
    });
}
