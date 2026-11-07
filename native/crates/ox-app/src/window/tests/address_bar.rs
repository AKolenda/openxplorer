// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar: breadcrumbs, editing, and a width that never follows
//! the path.

use std::fs;
use std::path::PathBuf;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use ox_core::settings::{PreferencesUpdate, Settings};

use crate::locations::location_context;
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};
use crate::window::address_bar::AddressMode;
use crate::window::menu_popover::MenuPopover;

use super::address_input::press_key;
use super::narrow_windows::resize;

/// The window's minimum width.
fn minimum_width(test: &TestWindow) -> i32 {
    let (minimum, _, _, _) = test.window.measure(gtk::Orientation::Horizontal, -1);
    minimum
}

/// A folder fourteen long names deep inside `fixture`.
fn deep_folder(fixture: &Fixture) -> PathBuf {
    let mut path = fixture.root().to_path_buf();
    for level in 0..14 {
        path.push(format!("A rather long folder name at level {level}"));
    }
    fs::create_dir_all(&path).expect("deep fixture folders");
    path
}

/// parity: NAV-017
#[gtk::test]
fn crumbs_name_their_folder_and_show_its_full_address() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let last = crumbs.last().expect("a crumb per ancestor");
    assert_eq!(last.label().as_deref(), Some("Documents"));
    let address = fixture.path("Documents").display().to_string();
    assert_eq!(last.tooltip_text().as_deref(), Some(address.as_str()));
    assert!(crumbs.len() > 2, "every ancestor has a crumb");
    for crumb in &crumbs {
        assert!(gtk::test_accessible_has_property(
            crumb,
            gtk::AccessibleProperty::Label
        ));
        assert_eq!(crumb.action_name().as_deref(), Some("win.go-to"));
    }
    assert!(
        gtk::test_accessible_has_property(last, gtk::AccessibleProperty::Description),
        "the last crumb is announced as the current location"
    );
}

#[gtk::test]
fn a_deep_path_and_many_tabs_do_not_widen_the_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let narrowest = minimum_width(&test);
    let deep = deep_folder(&fixture);
    test.window
        .navigate(deep.to_str().expect("fixture paths are UTF-8"))
        .expect("valid folder");
    test.wait_for_listing("the deep folder");
    assert!(
        minimum_width(&test) <= narrowest + 1,
        "a deep path scrolls instead"
    );
    for _ in 0..16 {
        test.window.add_tab(&fixture.uri()).expect("valid folder");
    }
    test.wait_for_listing("the last tab");
    assert!(minimum_width(&test) <= narrowest + 1, "many tabs scroll instead");
}

/// The address bar's menu, once it shows.
fn open_menu(test: &TestWindow) -> MenuPopover {
    let address = test.window.address_bar();
    let menu = || {
        descendants::<MenuPopover>(address)
            .into_iter()
            .find(WidgetExt::is_visible)
    };
    wait_until("the menu", || menu().is_some());
    menu().expect("the menu")
}

/// As in Windows 11 File Explorer: the crumbs that do not fit collapse,
/// from the first on, into a « button in place of the first chevron,
/// recomputed as the window's width changes, and the current folder's
/// crumb always shows in full. The « lists them, the nearest last, then
/// the top-level places.
///
/// parity: NAV-017, NAV-023
#[gtk::test]
fn a_long_path_collapses_its_first_crumbs_into_a_button_as_the_width_allows() {
    let fixture = Fixture::standard();
    let deep = deep_folder(&fixture);
    let test = TestWindow::open(&ox_core::location::file_uri(&deep));
    let address = test.window.address_bar();
    let (chevron, overflow) = address.strip_buttons();
    resize(&test, 1320);
    wait_until("the first crumbs to collapse", || {
        !address.hidden_crumbs().is_empty() && overflow.is_child_visible() && !chevron.is_child_visible()
    });
    let crumbs = address.crumb_buttons();
    let hidden = address.hidden_crumbs();
    assert_eq!(hidden, crumbs[..hidden.len()], "the leading crumbs");
    let last = crumbs.last().expect("the current folder's crumb");
    assert!(last.is_child_visible());
    let bounds = last.compute_bounds(address).expect("a shown crumb");
    let overflow_end = overflow.compute_bounds(address).expect("a shown «");
    assert!(
        bounds.x() >= overflow_end.x() + overflow_end.width(),
        "after the «"
    );
    let end = f64::from(bounds.x() + bounds.width());
    assert!(end <= f64::from(address.width()), "in full");
    assert!(
        descendants::<gtk::ScrolledWindow>(address).is_empty(),
        "nothing scrolls"
    );

    resize(&test, 800);
    wait_until("more crumbs to collapse", || {
        address.hidden_crumbs().len() > hidden.len()
    });
    resize(&test, 1320);
    wait_until("them to come back", || address.hidden_crumbs() == hidden);

    overflow.emit_clicked();
    let menu = open_menu(&test);
    let mut expected: Vec<String> = hidden
        .iter()
        .map(|crumb| crumb.label().expect("a named crumb").to_string())
        .collect();
    let nearest = expected.last().expect("a hidden crumb").clone();
    expected.extend(["-", "Home", "This PC", "Network", "Recycle Bin"].map(String::from));
    assert_eq!(menu.row_labels(), expected);
    let nearest_uri = hidden
        .last()
        .and_then(ActionableExt::action_target_value)
        .and_then(|target| target.get::<String>());
    menu.row(&nearest).activate();
    wait_until("the nearest hidden folder", || {
        test.window.current_uri() == nearest_uri
    });
}

/// The « takes the keyboard as a crumb does: Shift+Tab and Left reach
/// it from the first crumb shown, and Down opens its menu, so the
/// collapsed crumbs stay a few keys away. Hidden, the chevron is no stop.
///
/// parity: NAV-018, NAV-023
#[gtk::test]
fn the_keyboard_reaches_the_collapsed_crumbs_through_the_overflow_button() {
    let fixture = Fixture::standard();
    let deep = deep_folder(&fixture);
    let test = TestWindow::open(&ox_core::location::file_uri(&deep));
    let address = test.window.address_bar();
    let (chevron, overflow) = address.strip_buttons();
    resize(&test, 1320);
    wait_until("the first crumbs to collapse", || {
        overflow.is_child_visible() && overflow.is_mapped() && !address.hidden_crumbs().is_empty()
    });
    assert!(!chevron.can_focus(), "the hidden chevron is no stop");
    let hidden = address.hidden_crumbs();
    let first_shown = address.crumb_buttons()[hidden.len()].clone();
    let strip = first_shown.parent().expect("crumbs sit in their strip");

    first_shown.grab_focus();
    test.window.emit_move_focus(gtk::DirectionType::TabBackward);
    assert!(overflow.has_focus(), "Shift+Tab reaches the «");

    first_shown.grab_focus();
    assert!(press_key(&strip, gdk::Key::Left));
    assert!(overflow.has_focus(), "Left reaches the «");
    assert!(press_key(&strip, gdk::Key::Left));
    assert!(overflow.has_focus(), "and stops there");
    press_key(&strip, gdk::Key::Right);
    assert!(first_shown.has_focus());

    overflow.grab_focus();
    assert!(press_key(&overflow, gdk::Key::Down), "Down opens the « menu");
    let menu = open_menu(&test);
    let first_hidden = hidden[0].label().expect("a named crumb").to_string();
    assert_eq!(menu.row_labels().first(), Some(&first_hidden));
    menu.popdown();
}

/// As in Windows 11 File Explorer, a chevron follows the location icon,
/// before the first crumb, and lists the top-level places.
///
/// parity: NAV-017
#[gtk::test]
fn the_chevron_after_the_icon_lists_the_top_level_places() {
    let _fixture = Fixture::standard();
    let test = TestWindow::open(ox_core::location::PC_URI);
    let address = test.window.address_bar();
    let (chevron, overflow) = address.strip_buttons();
    wait_until("the chevron before This PC", || {
        chevron.is_child_visible() && !overflow.is_child_visible() && chevron.width() > 0
    });
    let first = &address.crumb_buttons()[0];
    let chevron_bounds = chevron.compute_bounds(address).expect("a shown chevron");
    let first_bounds = first.compute_bounds(address).expect("a shown crumb");
    assert!(chevron_bounds.x() + chevron_bounds.width() <= first_bounds.x());
    assert!(gtk::test_accessible_has_property(
        &chevron,
        gtk::AccessibleProperty::Label
    ));

    chevron.emit_clicked();
    let menu = open_menu(&test);
    assert_eq!(menu.row_labels(), ["Home", "This PC", "Network", "Recycle Bin"]);
    menu.row("Home").activate();
    let home = test.window.imp().locations.borrow().home_uri();
    wait_until("the home folder", || {
        test.window.current_uri() == Some(home.clone())
    });
}

#[gtk::test]
fn editing_ends_when_focus_leaves_the_address() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    test.activate("location", None);
    assert_eq!(address.mode(), AddressMode::Entry);
    assert_eq!(
        address.entry().text().as_str(),
        fixture.root().display().to_string()
    );
    test.window.folder_pane().focus_view();
    assert_eq!(address.mode(), AddressMode::Crumbs);
}

#[gtk::test]
fn navigating_while_editing_returns_to_the_crumbs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("location", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the subfolder");
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
}

#[gtk::test]
fn left_and_right_move_focus_between_crumbs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let [.., parent, current] = crumbs.as_slice() else {
        panic!("a folder has several crumbs");
    };
    parent.grab_focus();
    test.window.emit_move_focus(gtk::DirectionType::Right);
    assert_eq!(GtkWindowExt::focus(&test.window), Some(current.clone().upcast()));
    test.window.emit_move_focus(gtk::DirectionType::Left);
    assert_eq!(GtkWindowExt::focus(&test.window), Some(parent.clone().upcast()));
}

#[gtk::test]
fn escape_discards_the_typed_address() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    test.activate("location", None);
    address.entry().set_text("/somewhere else");
    test.window.finish_address();
    assert_eq!(address.mode(), AddressMode::Crumbs);
    test.activate("location", None);
    assert_eq!(
        address.entry().text().as_str(),
        fixture.root().display().to_string()
    );
}

/// Emptying the address offers the protocols, as Dolphin's location bar
/// does; picking one types its `scheme://`.
///
/// parity: NET-029
#[gtk::test]
fn an_empty_address_offers_the_network_protocols() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    let chooser = address.protocol_chooser();
    test.activate("location", None);
    assert!(!chooser.is_visible(), "a typed address hides the chooser");

    address.entry().set_text("");
    wait_until("the chooser opens", || chooser.is_visible());
    let buttons = crate::test_support::harness::descendants::<gtk::Button>(&chooser);
    assert_eq!(
        buttons.len(),
        7,
        "SMB, SFTP, FTP, FTPS, WebDAV, secure WebDAV and NFS"
    );
    let keys = address.entry().observe_controllers();
    let down = gtk::gdk::Key::Down.into_glib();
    for key in keys.iter::<glib::Object>().flatten() {
        if let Ok(key) = key.downcast::<gtk::EventControllerKey>() {
            key.emit_by_name::<bool>("key-pressed", &[&down, &0u32, &gtk::gdk::ModifierType::empty()]);
        }
    }
    assert!(buttons[0].is_focus(), "Down moves into the list");
    assert!(chooser.is_visible(), "and the list stays open");
    assert_eq!(address.mode(), AddressMode::Entry);
    buttons[1].emit_clicked();

    assert_eq!(address.entry().text().as_str(), "sftp://");
    assert!(!chooser.is_visible());
    assert_eq!(address.mode(), AddressMode::Entry, "the user goes on typing");
}

/// parity: TAB-010, NAV-017
#[gtk::test]
fn the_title_crumbs_and_address_call_a_phone_by_its_mount_name() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let phone = VolumeRow {
        label: "Pixel 7".into(),
        kind: VolumeKind::Device,
        state: VolumeState::Mounted {
            uri: "mtp://[usb:001,010]/".into(),
            controls: MountControls::UNMOUNTABLE,
        },
    };
    let context = location_context(gtk::glib::home_dir(), &[phone]);
    test.window.imp().locations.replace(context);
    {
        let mut session = test.window.imp().session.borrow_mut();
        let tab = session.active_mut().expect("one tab");
        tab.history.push("mtp://[usb:001,010]/Internal%20storage/DCIM");
    }
    test.window.render_navigation();
    assert_eq!(test.window.title().as_deref(), Some("DCIM — OpenXplorer"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let labels: Vec<String> = crumbs
        .iter()
        .filter_map(|crumb| crumb.label().map(|label| label.to_string()))
        .collect();
    assert_eq!(labels, ["Pixel 7", "Internal storage", "DCIM"]);
    let entry = test.window.address_bar().entry();
    assert_eq!(entry.text().as_str(), "Pixel 7 / Internal storage/DCIM");
}

/// "Show full path in the title bar" titles the window with the folder's
/// path instead of its name.
///
/// parity: SET-011, TAB-046
#[gtk::test]
fn the_title_shows_the_full_path_when_the_settings_ask() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    assert_eq!(test.window.title().as_deref(), Some("Documents — OpenXplorer"));
    let update = PreferencesUpdate {
        full_path_in_title: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");

    test.context.reload_settings();

    let path = fixture.path("Documents").display().to_string();
    wait_until("the title to show the path", || {
        test.window.title().as_deref() == Some(&format!("{path} — OpenXplorer"))
    });
}
