// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's behaviour: opening and reloading places, the highlight of
//! the open place, Quick access pins, drives that still have to be
//! mounted, and the resizer.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, Settings};

use super::file_ops_support::is_enabled;
use crate::icons::{Art, ArtImage, Icon};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::sidebar::entries::{RowLevel, RowTarget, Section, SidebarEntry};

/// The sidebar row labelled `label`.
fn row_named(test: &TestWindow, label: &str) -> gtk::ListBoxRow {
    let labels = test.window.sidebar().labels();
    let index = labels.iter().position(|shown| shown == label);
    let index = index.unwrap_or_else(|| panic!("{label} is in the sidebar: {labels:?}"));
    let index = i32::try_from(index).expect("a short sidebar");
    test.window
        .sidebar()
        .list()
        .row_at_index(index)
        .expect("a row for every label")
}

/// Waits until the sidebar shows a row labelled `label`.
fn wait_for_row(test: &TestWindow, label: &str) {
    wait_until(label, || {
        test.window.sidebar().labels().iter().any(|shown| shown == label)
    });
}

/// A window on the fixture with the fixture folder pinned.
fn with_fixture_pinned(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    test.activate("pin-folder", None);
    wait_for_row(&test, "Example projects");
    test
}

/// parity: SIDE-001
#[gtk::test]
fn the_group_chevrons_never_collapse_and_quick_access_has_no_heading() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let rows = descendants::<gtk::ListBoxRow>(sidebar.list());
    assert_eq!(rows.len(), sidebar.labels().len(), "every row is a place");
    let texts: Vec<String> = descendants::<gtk::Label>(sidebar)
        .iter()
        .map(|label| label.text().to_string())
        .collect();
    assert!(
        !texts.iter().any(|text| text.contains("Quick access")),
        "{texts:?}"
    );
    for group in ["This PC", "Network"] {
        let buttons = descendants::<gtk::Button>(&row_named(&test, group));
        assert!(buttons.is_empty(), "{group}'s chevron is no button");
    }

    assert!(row_named(&test, "This PC").activate());
    test.wait_for_listing("This PC");

    assert_eq!(test.window.current_uri().as_deref(), Some(Page::ThisPc.uri()));
    assert!(sidebar.labels().contains(&"Local Disk".to_owned()));
}

/// parity: SIDE-002
#[gtk::test]
fn clicking_the_open_place_reloads_it_and_clears_the_filter() {
    let fixture = Fixture::standard();
    let test = with_fixture_pinned(&fixture);
    test.search_for("notes");
    assert_eq!(test.names().len(), 2);
    fixture.write("Later.txt");

    assert!(row_named(&test, "Example projects").activate());
    test.wait_for_listing("the folder again");

    assert!(!test.window.is_searching());
    assert_eq!(test.window.search_box().entry().text(), "");
    wait_until("the new file", || test.names().contains(&"Later.txt".to_owned()));
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: SIDE-003
#[gtk::test]
fn the_open_place_is_highlighted_and_every_row_is_titled_with_its_path() {
    let fixture = Fixture::standard();
    let test = with_fixture_pinned(&fixture);
    let pin = row_named(&test, "Example projects");
    assert!(pin.is_selected());
    let path = fixture.root().to_string_lossy().into_owned();
    assert_eq!(pin.tooltip_text().as_deref(), Some(path.as_str()));

    test.window.sidebar().select(&format!("{}/", fixture.uri()));
    assert!(pin.is_selected(), "a trailing slash is the same place");
    test.window.sidebar().select(&fixture.uri_of("Documents"));
    assert!(!pin.is_selected(), "only the place itself is highlighted");
}

/// A pin on a share shows the network pipe titled "Network share" and
/// the pin mark; a middle-click opens a pin in a background tab.
///
/// parity: SIDE-005
#[gtk::test]
fn pins_show_the_pin_mark_and_open_in_a_background_tab_on_a_middle_click() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut python_app = Settings::open(test.settings_directory());
    for (uri, label) in [("smb://nas/work", "Work"), (fixture.uri().as_str(), "Projects")] {
        python_app
            .bookmark(
                BookmarkAction::Add,
                BookmarkKind::Pin,
                &BookmarkRequest::new(uri, label),
            )
            .expect("the settings file takes a pin");
    }
    test.activate("refresh", None);
    wait_for_row(&test, "Work");

    let work = row_named(&test, "Work");
    let art = descendants::<ArtImage>(&work)
        .into_iter()
        .find(|image| matches!(image.art(), Some(Art::Network(_))))
        .expect("a share pin shows the network pipe");
    assert_eq!(art.tooltip_text().as_deref(), Some("Network share"));
    let pin_marks = descendants::<gtk::Image>(&work)
        .into_iter()
        .filter(|image| image.has_css_class("pin"))
        .count();
    assert_eq!(pin_marks, 1);

    let list = test.window.sidebar().list().clone();
    let middle = list
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .find(|gesture| gesture.button() == gdk::BUTTON_MIDDLE)
        .expect("the sidebar opens places on a middle-click");
    // Rows are found by position once they are laid out.
    wait_until("the rows to be laid out", || {
        let y = test.window.sidebar().middle_of("Projects");
        test.window.sidebar().location_at(y).is_some()
    });
    let y = test.window.sidebar().middle_of("Projects");
    middle.emit_by_name::<()>("released", &[&1_i32, &5.0_f64, &y]);

    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the new tab stays behind"
    );
}

/// parity: SIDE-007
#[gtk::test]
fn only_folders_are_pinned_one_request_at_a_time_and_not_from_pages() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .folder_model()
        .select_only(test.position_of("Notes 2.txt"));
    test.activate("pin-selected", None);
    assert_eq!(
        test.window.shown_message().as_str(),
        "Only folders and network shares can be pinned. Select folders only."
    );

    test.activate("pin-folder", None);
    assert!(test.window.imp().pinning.get(), "the first request runs");
    test.window
        .pin_dropped(&[fixture.uri_of("Documents")], None)
        .expect("a folder can be dropped");
    wait_for_row(&test, "Example projects");
    wait_until("the request to end", || !test.window.imp().pinning.get());
    let pins = test.context.settings_data().pins;
    assert_eq!(pins.len(), 1, "the drop during the first request was ignored");

    test.window.navigate(Page::ThisPc.uri()).expect("a page");
    test.wait_for_listing("This PC");
    assert!(!is_enabled(&test, "pin-folder"), "a page cannot be pinned");
}

/// parity: SIDE-008
#[gtk::test]
fn dragging_a_pin_before_another_moves_it_there() {
    let fixture = Fixture::standard();
    for name in ["Alpha", "Beta"] {
        fs::create_dir(fixture.path(name)).expect("fixture folder");
    }
    let test = TestWindow::open(&fixture.uri());
    let (alpha, beta) = (fixture.uri_of("Alpha"), fixture.uri_of("Beta"));
    test.window
        .pin_dropped(&[alpha.clone(), beta.clone()], None)
        .expect("folders can be dropped");
    wait_for_row(&test, "Beta");
    wait_until("the first drop to end", || !test.window.imp().pinning.get());
    let order = |test: &TestWindow| {
        let labels = test.window.sidebar().labels();
        let alpha = labels.iter().position(|label| label == "Alpha");
        let beta = labels.iter().position(|label| label == "Beta");
        (alpha, beta)
    };
    let (Some(first), Some(second)) = order(&test) else {
        panic!("both pins are shown");
    };
    assert_eq!(second, first + 1);

    test.window
        .pin_dropped(&[beta], Some(alpha))
        .expect("a pin can be dropped");
    wait_until("the new order", || {
        let (alpha, beta) = order(&test);
        beta < alpha
    });
    assert_eq!(test.context.settings_data().pins.len(), 2, "the pin moved");
}

/// A volume that still has to be mounted mounts when clicked, but a
/// middle-click, a drag or a drop does nothing on it; a mounted drive
/// takes drops.
///
/// parity: SIDE-016
#[gtk::test]
fn a_volume_to_mount_is_neither_dragged_nor_dropped_on() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let sidebar = test.window.sidebar();
    let local_disk = sidebar.middle_of("Local Disk");
    assert_eq!(sidebar.location_at(local_disk).as_deref(), Some("file:///"));
    assert!(sidebar.drop_spot_at(local_disk).is_some());

    let volume = SidebarEntry {
        section: Section::ThisPc,
        level: RowLevel::Child,
        label: "Backup".into(),
        icon: Art::Glyph(Icon::HardDrive),
        target: RowTarget::MountVolume("uuid-1".into()),
        tooltip: "Backup".into(),
        pinned: false,
        menu: None,
        eject: None,
    };
    sidebar.set_entries(vec![volume]);
    wait_for_frames(&test.window, 3);
    let row = sidebar.list().row_at_index(0).expect("the volume's row");
    let middle = sidebar.middle_of("Backup");

    assert_eq!(row.action_name().as_deref(), Some("win.mount-volume"));
    assert_eq!(sidebar.location_at(middle), None, "nothing to open or drag");
    assert_eq!(sidebar.drop_spot_at(middle), None);
}

/// parity: SIDE-023, ACC-001, ACC-006
#[gtk::test]
fn the_resizer_is_a_titled_separator_that_the_keys_move() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let resizer = test.window.sidebar_resizer();
    let handle = test.window.sidebar_handle();
    assert_eq!(resizer.accessible_role(), gtk::AccessibleRole::Separator);
    assert_eq!(
        handle.tooltip_text().as_deref(),
        Some("Drag to resize sidebar · double-click to reset")
    );
    assert!(resizer.grab_focus(), "the resizer is a Tab stop");
    assert!(handle.has_css_class("keyboard-focus"), "focus lights the handle");
    let workspace = test.window.workspace();
    assert_eq!(workspace.position(), 210);

    assert!(test.window.resize_sidebar_by_key(gdk::Key::Right, true));
    assert_eq!(workspace.position(), 250);
    assert!(test.window.resize_sidebar_by_key(gdk::Key::Left, false));
    assert_eq!(workspace.position(), 240);
    wait_until("the width to be saved", || {
        test.context.settings_data().preferences.sidebar_width == Some(240)
    });
    assert!(test.window.resize_sidebar_by_key(gdk::Key::Home, false));
    assert_eq!(workspace.position(), 210);
    for _ in 0..10 {
        test.window.resize_sidebar_by_key(gdk::Key::Left, true);
    }
    assert_eq!(workspace.position(), 140, "never narrower than 140");
    assert!(resizer.request_value(300.0), "a screen reader can set the width");
    assert_eq!(workspace.position(), 300);
}
