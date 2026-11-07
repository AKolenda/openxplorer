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

/// parity: NAV-015
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

/// Quick access stays in the navigation pane; This PC lists the drives,
/// and network locations once one is saved (owner decision 2026-11-06).
#[gtk::test]
fn this_pc_lists_devices_and_drives_and_no_quick_access() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();
    assert_eq!(landing::section_titles(landing), ["Devices and drives"]);
}

/// The status bar reads as Windows 11 Explorer's: "4 items", then, past
/// a divider shown only with a selection, "1 item selected  20 bytes",
/// the size only while every selected item is a file.
///
/// parity: VIEW-050
#[gtk::test]
fn the_status_bar_says_ready_on_a_page_and_counts_a_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(Page::ThisPc.uri());
    let status_bar = test.window.status_bar();
    assert_eq!(status_bar.texts(), ("Ready".to_owned(), String::new()));
    assert!(!status_bar.shows_selection_divider());
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    assert_eq!(status_bar.texts(), ("4 items".to_owned(), String::new()));
    assert!(!status_bar.shows_selection_divider(), "nothing is selected");
    let model = test.window.folder_model();
    assert_eq!(model.name_at(1).as_deref(), Some("Notes 2.txt"));
    model.select_only(1);
    assert_eq!(
        status_bar.texts(),
        ("4 items".to_owned(), "1 item selected  20 bytes".to_owned())
    );
    assert!(status_bar.shows_selection_divider());
    model.select_only(0);
    assert_eq!(
        status_bar.texts().1,
        "1 item selected",
        "a folder has no size here"
    );
    model.select_all();
    assert_eq!(
        status_bar.texts().1,
        "4 items selected",
        "nor has a selection with a folder"
    );
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
    // Taking the pin request proves the first one has ended.
    wait_until("the pin request to end", || test.window.start_pinning());
    test.window.end_pinning();
    assert_eq!(
        test.window.shown_message().as_str(),
        "",
        "pinning shows no message"
    );
    test.activate("pin-folder", None);
    let message = test.window.shown_message();
    assert_eq!(
        message.as_str(),
        "",
        "pinning again does nothing, as Windows Explorer"
    );
    assert_eq!(test.context.settings_data().pins.len(), 1, "nor pins it twice");
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

/// As Windows 11's Details pane: the folder's name and item count with
/// nothing selected, and a selected file's rows of names and values,
/// without buttons.
///
/// parity: PROP-009
#[gtk::test]
fn the_details_pane_lists_a_file_and_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.open_details_pane();
    let pane = test.window.details_pane();
    // Hidden items count only while they are shown, as in the status bar.
    assert_eq!(pane.shown_summary(), "4 items", "hidden items are not listed");
    assert!(pane.shown_properties().is_empty());
    test.activate("hidden", None);
    assert_eq!(pane.shown_summary(), "5 items", "shown hidden items count");
    test.activate("hidden", None);
    test.select_named("Notes 2.txt");
    assert_eq!(pane.shown_summary(), "", "no count under an item's name");
    let file = pane.shown_properties();
    let names: Vec<&str> = file
        .iter()
        .map(|property| property.name.as_str())
        // Only a file system that records creation times has the row.
        .filter(|name| *name != "Date created")
        .collect();
    assert_eq!(names, ["Type", "Size", "Date modified", "Availability"]);
    assert_eq!(property(&file, "Type"), Some("Text Document"));
    assert_eq!(property(&file, "Availability"), Some("Available on this device"));
    let buttons = descendants::<gtk::Button>(pane);
    let shown: Vec<&gtk::Button> = buttons.iter().filter(|button| button.is_visible()).collect();
    assert_eq!(shown.len(), 1, "only the close button: {shown:?}");
    assert!(shown[0].has_css_class("detail-close"));
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
    super::support::show_file_name_extensions(&test);
    test.open_details_pane();
    let pane = test.window.details_pane();
    for label in ["Show the item under the pointer", "Size"] {
        let menu = pane.show_options_menu(1.0, 1.0);
        assert!(menu.checked_labels().contains(&"Type".to_owned()));
        menu.row(label).emit_activate();
    }
    let menu = pane.show_options_menu(1.0, 1.0);
    let checked = menu.checked_labels();
    assert!(checked.contains(&"Show the item under the pointer".to_owned()));
    assert!(!checked.contains(&"Size".to_owned()));
    menu.popdown();

    let view = test.window.folder_pane().view_widget();
    let position = 1;
    // The rows can still be moving while the list settles, so point at
    // the row again until the pane names the item drawn there.
    wait_until("the pane to describe the item under the pointer", || {
        let point = test
            .window
            .folder_pane()
            .owners()
            .widget_at(position)
            .and_then(|row| row.compute_point(&view, &gtk::graphene::Point::new(2.0, 2.0)));
        let Some(point) = point else {
            return false;
        };
        test.window
            .pointer_over_items(&view, Some((f64::from(point.x()), f64::from(point.y()))));
        test.window.folder_model().name_at(position).as_deref() == Some(pane.shown_name().as_str())
    });
    let shown = pane.shown_properties();
    assert!(property(&shown, "Type").is_some());
    assert!(property(&shown, "Size").is_none(), "Size is turned off");
    test.window.pointer_over_items(&view, None);
    assert!(!pane.shown_summary().is_empty(), "the folder again");
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
