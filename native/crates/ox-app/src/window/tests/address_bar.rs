// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar: breadcrumbs, editing, and a width that never follows
//! the path.

use std::fs;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::locations::location_context;
use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};
use crate::window::address_bar::AddressMode;

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

/// parity: NAV-017
#[gtk::test]
fn the_crumbs_stay_scrolled_to_the_current_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let deep = deep_folder(&fixture);
    test.window
        .navigate(deep.to_str().expect("fixture paths are UTF-8"))
        .expect("valid folder");
    test.wait_for_listing("the deep folder");
    let adjustment = test.window.address_bar().crumb_adjustment();
    wait_until("the crumbs to overflow and scroll to the end", || {
        let end = adjustment.upper() - adjustment.page_size();
        end > 0.0 && (adjustment.value() - end).abs() < 1.0
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
