// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar, the landing pages, pins and the details pane.

use std::time::Duration;

use gtk::prelude::*;
use ox_core::settings::{BookmarkRequest, Settings};

use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::window::details_pane::{ShownProperty, PANE_WIDTH};
use crate::window::landing;

/// The value of the property called `name` among `properties`.
fn property<'a>(properties: &'a [ShownProperty], name: &str) -> Option<&'a str> {
    let property = properties.iter().find(|property| property.name == name)?;
    Some(property.value.as_str())
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
    let rows = descendants::<gtk::ListBoxRow>(test.window.sidebar().list());
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
    let selected = test.window.sidebar().list().selected_row();
    assert_eq!(selected.map(|row| row.index()), Some(0), "Home is the first row");
}

#[gtk::test]
fn this_pc_lists_quick_access_devices_and_network_locations() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();
    assert_eq!(
        landing::section_titles(landing),
        ["Quick access", "Devices and drives", "Network locations"]
    );
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    assert_eq!(
        landing::section_titles(landing),
        ["Discovered servers", "Connected & saved locations"]
    );
}

#[gtk::test]
fn the_status_bar_says_ready_on_a_page_and_counts_a_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(Page::ThisPc.uri());
    assert_eq!(
        test.window.status_bar().texts(),
        ("Ready".to_owned(), String::new())
    );
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    test.window.folder_model().select_only(0);
    let (count, selection) = test.window.status_bar().texts();
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
    let message = test.window.shown_message();
    assert_eq!(message.as_str(), "Already pinned to Quick access.");
}

#[gtk::test]
fn the_details_pane_keeps_its_width_for_long_names() {
    let fixture = Fixture::standard();
    let long_name = format!("{}.txt", "A very long file name ".repeat(8));
    fixture.write(&long_name);
    let test = TestWindow::open(&fixture.uri());
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    let position = test.names().iter().position(|name| *name == long_name);
    let position = position.expect("the long name is listed");
    test.window
        .folder_model()
        .select_only(u32::try_from(position).expect("a short listing"));
    let pane = test.window.details_pane();
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
    // The Python backend lists hidden items only while they are shown
    // (`enumerate_folder(uri, showHidden)`), and app.js counts that list.
    assert_eq!(
        property(&folder, "Items"),
        Some("4"),
        "hidden items are not listed"
    );
    assert_eq!(property(&folder, "Storage"), Some("This computer"));
    test.activate("hidden", None);
    let with_hidden = pane.shown_properties();
    assert_eq!(
        property(&with_hidden, "Items"),
        Some("5"),
        "shown hidden items count"
    );
    test.activate("hidden", None);
    test.window.folder_model().select_only(1);
    let file = pane.shown_properties();
    let names: Vec<&str> = file.iter().map(|property| property.name.as_str()).collect();
    assert_eq!(names, ["Type", "Size", "Modified", "Location"]);
    let folder_address = fixture.root().display().to_string();
    assert_eq!(property(&file, "Location"), Some(folder_address.as_str()));
}

/// One selected image shows its picture and dimensions in the details
/// pane.
///
/// parity: PROP-011, PROP-013
#[gtk::test]
fn the_details_pane_previews_an_image_and_names_its_dimensions() {
    let fixture = Fixture::standard();
    let pixels = gtk::glib::Bytes::from_owned(vec![0x40_u8; 4 * 3 * 4]);
    let texture = gtk::gdk::MemoryTexture::new(4, 3, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 16);
    texture
        .save_to_png(fixture.path("Beach.png"))
        .expect("a synthetic image");
    let test = TestWindow::open(&fixture.uri());
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    let pane = test.window.details_pane();
    test.select_named("Beach.png");

    wait_until("the picture", || !descendants::<gtk::Picture>(pane).is_empty());
    wait_until("the dimensions", || {
        property(&pane.shown_properties(), "Dimensions") == Some("4 × 3 pixels")
    });
    // A redraw for another reason, such as a size scan's progress, keeps
    // the picture and its dimensions instead of reading them again.
    let picture = descendants::<gtk::Picture>(pane).remove(0);
    test.window.update_details_pane();
    assert_eq!(descendants::<gtk::Picture>(pane), [picture]);
    assert_eq!(
        property(&pane.shown_properties(), "Dimensions"),
        Some("4 × 3 pixels")
    );
    test.activate("properties", None);
    let frame = test.wait_for_dialog("the Properties dialog");
    wait_until("the dimensions in Properties", || {
        super::item_dialogs::texts(&frame).contains(&"4 × 3 pixels".to_owned())
    });
    frame.close();
    test.window.folder_model().select_none();
    wait_until("the folder's art again", || {
        descendants::<gtk::Picture>(pane).is_empty()
    });
}

/// The pane's menu makes it follow the pointer and drop a field, and
/// the choice is saved.
///
/// parity: PROP-010
#[gtk::test]
fn the_details_pane_follows_the_pointer_and_shows_the_chosen_fields() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.details_pane();
    let menu = pane.show_options_menu(1.0, 1.0);
    for label in ["Show the item under the pointer", "Size"] {
        let check = descendants::<gtk::CheckButton>(&menu)
            .into_iter()
            .find(|check| check.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("a {label} choice"));
        check.set_active(!check.is_active());
    }
    menu.popdown();

    let view = test.window.folder_pane().view_widget();
    let position = 1;
    let row = test
        .window
        .folder_pane()
        .owners()
        .widget_at(position)
        .expect("a shown row");
    let point = row
        .compute_point(&view, &gtk::graphene::Point::new(2.0, 2.0))
        .expect("the row is in the view");
    test.window
        .pointer_over_items(&view, Some((f64::from(point.x()), f64::from(point.y()))));
    let hovered = test.window.folder_model().name_at(position).expect("a name");
    let shown = pane.shown_properties();
    assert_eq!(
        pane.shown_name(),
        hovered,
        "the pane describes the item under the pointer"
    );
    assert!(property(&shown, "Type").is_some());
    assert!(property(&shown, "Size").is_none(), "Size is turned off");
    test.window.pointer_over_items(&view, None);
    assert!(
        property(&pane.shown_properties(), "Items").is_some(),
        "the folder again"
    );
    wait_until("the options saved", || {
        let saved = Settings::open(test.settings_directory())
            .data()
            .preferences
            .clone();
        saved.details_pane_options.follow_hover && !saved.details_pane_options.shows("Size")
    });
}

/// parity: SIDE-022
#[gtk::test]
fn pins_another_process_saved_appear_after_a_refresh() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut python_app = Settings::open(test.settings_directory());
    let pin = BookmarkRequest::new(fixture.uri_of("Documents"), "Pinned elsewhere");
    python_app
        .pin_many(&[pin], None, None)
        .expect("the settings file takes a pin");
    test.activate("refresh", None);
    wait_until("the pin to reach the sidebar", || {
        let labels = test.window.sidebar().labels();
        labels.contains(&"Pinned elsewhere".to_owned())
    });
}
