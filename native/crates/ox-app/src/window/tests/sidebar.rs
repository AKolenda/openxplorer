// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's behaviour: opening and reloading places, the highlight of
//! the open place, Quick access pins, drives that still have to be
//! mounted, and the resizer.

use std::fs;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, Settings};

use super::file_ops_support::{is_enabled, open_dialog, wait_for_no_dialog};
use super::support::middle_click_at;
use crate::icons::{Art, ArtImage, Icon};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::{MenuEntry, MenuPopover};
use crate::window::sidebar::entries::{RowLevel, RowTarget, Section, SidebarEntry};
use crate::window::WindowAction;

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
    assert!(pin.is_selected(), "the closest place holding it (SIDE-004)");
    test.window.sidebar().select("sftp://elsewhere/home");
    assert!(!pin.is_selected(), "no place holds it");
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

    // Rows are found by position once they are laid out.
    wait_until("the rows to be laid out", || {
        let y = test.window.sidebar().middle_of("Projects");
        test.window.sidebar().location_at(y).is_some()
    });
    let y = test.window.sidebar().middle_of("Projects");
    middle_click_at(test.window.sidebar().list(), (5.0, y));

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
/// A mounted drive's row shows a capacity bar under its name.
///
/// parity: SIDE-016, SIDE-018
#[gtk::test]
fn a_volume_to_mount_is_neither_dragged_nor_dropped_on() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let sidebar = test.window.sidebar();
    let local_disk = sidebar.middle_of("Local Disk");
    assert_eq!(sidebar.location_at(local_disk).as_deref(), Some("file:///"));
    assert!(sidebar.drop_spot_at(local_disk).is_some());
    wait_until("Local Disk's capacity bar", || {
        let bars = descendants::<gtk::ProgressBar>(&row_named(&test, "Local Disk"));
        bars.iter().any(|bar| {
            bar.has_css_class("capacity") && bar.tooltip_text().is_some_and(|t| t.contains(" free of "))
        })
    });

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

/// parity: SIDE-024
#[gtk::test]
fn the_navigation_pane_hides_and_a_places_button_lists_its_places() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let places = test.window.places_button().clone();
    assert!(!places.is_visible(), "no Places button beside the pane");

    test.activate("sidebar", None);

    assert!(!test.window.sidebar().is_visible());
    assert!(!test.window.sidebar_resizer().is_visible());
    assert!(places.is_visible());
    wait_until("the choice to be saved", || {
        test.context.settings_data().preferences.hide_sidebar
    });
    places.popup();
    let menu = places
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("the places menu");
    let labels = menu.row_labels();
    assert!(labels.contains(&"Home".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Local Disk".to_owned()), "{labels:?}");
    places.popdown();

    test.activate("sidebar", None);
    assert!(test.window.sidebar().is_visible());
    assert!(!places.is_visible());
}

/// Fills the add or edit dialog's Label and Location and saves.
fn answer_place_dialog(test: &TestWindow, label: &str, location: &str) {
    let dialog = open_dialog(test);
    let fields = descendants::<gtk::Entry>(&dialog);
    let [label_field, location_field] = fields.as_slice() else {
        panic!("the dialog asks for a label and a location: {}", fields.len());
    };
    label_field.set_text(label);
    location_field.set_text(location);
    let answer = if dialog.title_text() == "Add entry" {
        "Add"
    } else {
        "Save"
    };
    dialog.press(answer);
    wait_for_no_dialog(test);
}

/// "Add entry…" on empty space pins any location without going there;
/// "Edit…" on a pin renames and moves it in place.
///
/// parity: SIDE-031, SIDE-011
#[gtk::test]
fn add_entry_pins_a_typed_location_and_edit_changes_it_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let menu = test
        .window
        .sidebar()
        .menu_entries_at(100_000.0)
        .expect("a menu on empty space");
    let MenuEntry::Item(add) = &menu[0] else {
        panic!("Add entry… comes first");
    };
    assert_eq!(add.label, "Add entry…");

    test.activate("add-place", None);
    answer_place_dialog(&test, "Docs", &fixture.path("Documents").to_string_lossy());
    wait_for_row(&test, "Docs");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "the window stays");

    test.activate("edit-pin", Some(&fixture.uri_of("Documents")));
    answer_place_dialog(&test, "Projects", &fixture.uri());
    wait_for_row(&test, "Projects");
    let labels = test.window.sidebar().labels();
    assert!(!labels.contains(&"Docs".to_owned()), "{labels:?}");
    let pins = test.context.settings_data().pins;
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert!(same_location(&pins[0].uri, &fixture.uri()));
}

/// parity: SIDE-012
#[gtk::test]
fn the_icon_size_chosen_on_empty_space_redraws_the_rows_and_is_saved() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let menu = test
        .window
        .sidebar()
        .menu_entries_at(100_000.0)
        .expect("a menu on empty space");
    let sizes: Vec<String> = menu
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) if item.action == WindowAction::SidebarIconSize.into() => {
                Some(item.label.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        sizes,
        [
            "Automatic icon size",
            "Small icons",
            "Medium icons",
            "Large icons",
            "Huge icons"
        ]
    );
    let home_icon_width = || {
        let image = descendants::<ArtImage>(&row_named(&test, "Home"))
            .into_iter()
            .next()
            .expect("Home has an icon");
        image.measure(gtk::Orientation::Horizontal, -1).0
    };
    let automatic = home_icon_width();

    test.activate("sidebar-icon-size", Some("48"));

    assert!(
        home_icon_width() >= 48,
        "{automatic} grew to {}",
        home_icon_width()
    );
    wait_until("the size to be saved", || {
        test.context.settings_data().preferences.sidebar_icon_size == 48
    });
}

/// "Hide section" hides a group for good; "Show all entries" lists it
/// dimmed with "Show section", and a hidden standard folder with "Show".
///
/// parity: SIDE-010
#[gtk::test]
fn hidden_sections_and_folders_are_listed_dimmed_and_shown_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let menu_labels = |label: &str| {
        // Rows redrawn a moment ago are found by position once laid out.
        wait_for_frames(&test.window, 3);
        let menu = sidebar.right_click_row(label);
        let labels = menu.row_labels();
        menu.popdown();
        labels
    };
    assert!(menu_labels("This PC").contains(&"Hide section \u{201c}This PC\u{201d}".to_owned()));
    let known = test.context.known_folders();
    let standard = known.iter().find(|place| sidebar.labels().contains(&place.label));

    test.activate("hide-section", Some("thisPc"));
    wait_until("This PC to go", || {
        !sidebar.labels().contains(&"This PC".to_owned())
    });
    assert!(!sidebar.labels().contains(&"Local Disk".to_owned()));
    if let Some(folder) = standard {
        test.activate("unpin", Some(&folder.uri));
        wait_until("the folder to go", || !sidebar.labels().contains(&folder.label));
    }

    test.activate("sidebar-show-all", None);
    let this_pc = row_named(&test, "This PC");
    assert!(this_pc.has_css_class("hidden-place"), "dimmed");
    assert_eq!(menu_labels("This PC"), ["Show section \u{201c}This PC\u{201d}"]);
    if let Some(folder) = standard {
        assert_eq!(menu_labels(&folder.label), ["Show"]);
        test.activate("show-place", Some(&folder.uri));
        wait_until("the folder to be shown", || {
            !row_named(&test, &folder.label).has_css_class("hidden-place")
        });
    }
    test.activate("show-section", Some("thisPc"));
    wait_until("This PC to be shown", || {
        !row_named(&test, "This PC").has_css_class("hidden-place")
    });
    test.activate("sidebar-show-all", None);
    assert!(sidebar.labels().contains(&"Local Disk".to_owned()));
    assert!(test
        .context
        .settings_data()
        .preferences
        .hidden_sidebar_sections
        .is_empty());
}
