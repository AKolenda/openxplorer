// SPDX-License-Identifier: AGPL-3.0-only
//! The right-click menus of the Recycle Bin, of a ZIP opened like a
//! folder, and of the blank space of This PC, Home and Network, opened
//! with the real pointer on check.py's private display, as Windows 11
//! File Explorer lists them (checked on Windows 11 build 26200,
//! 2026-11-21). The app's own Refresh stays where it was.

use gtk::prelude::*;
use ox_core::location::{HOME_URI, NETWORK_URI, PC_URI, TRASH_URI};
use ox_core::settings::ContextMenu;

use super::command_bar_switch::Session;
use super::context_menus::{choose_menu_style, position_of};
use super::menu_sweep::{close, on_blank, on_item, open_menu, summary};
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture};
use crate::test_support::pointer::Button;
use crate::window::menu_popover::{MenuPopover, MenuStyle};

/// Right-clicks the screen point `at` with the real pointer and returns
/// the menu that opens, in `style`.
fn right_click_at(session: &mut Session, at: (i32, i32), what: &str) -> MenuPopover {
    let window = session.test.window.clone();
    for _ in 0..2 {
        session.pointer.move_to(at.0, at.1);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        if let Some(menu) = open_menu(&window) {
            eprintln!("MENU {what}: {}", summary(&menu));
            return menu;
        }
    }
    panic!("a right-click on {what} opens a menu");
}

/// The screen point on blank space at the bottom right of the page shown
/// in the folder pane: the landing page's, or the file list's.
fn on_page_blank(session: &Session) -> (i32, i32) {
    let pane = session.test.window.folder_pane().clone();
    session.on_screen(
        &pane,
        (f64::from(pane.width()) - 16.0, f64::from(pane.height()) - 16.0),
    )
}

/// Shows `uri` and waits until it is drawn.
fn show(session: &Session, uri: &str) {
    session.test.activate("go-to", Some(uri));
    wait_until(&format!("{uri} to show"), || {
        session.test.window.current_uri().as_deref() == Some(uri)
    });
    wait_for_frames(&session.test.window, 6);
}

/// Whether the row `label` of `menu` can be chosen.
fn enabled(menu: &MenuPopover, label: &str) -> bool {
    menu.row(label).is_sensitive()
}

/// Deletes `name` from the folder shown, so the Recycle Bin holds it and
/// Undo can take the deletion back.
fn delete(session: &Session, name: &str) {
    let position = position_of(&session.test, name);
    session.test.window.folder_model().select_only(position);
    session.test.activate("trash", None);
    wait_until("the file to go", || {
        !session.test.names().contains(&name.to_owned())
    });
}

/// The Recycle Bin's item and blank-space menus, compact and classic.
///
/// parity: OPS-040, CMD-010
#[gtk::test]
fn the_recycle_bin_menus_are_windows_11s() {
    let fixture = Fixture::real_world();
    let mut session = Session::open_on(fixture, None);
    delete(&session, "Meeting notes.odt");
    show(&session, TRASH_URI);
    wait_until("the deleted file", || !session.test.names().is_empty());

    choose_menu_style(&session.test, ContextMenu::Win11);
    let at = on_item(&session, "Meeting notes.odt");
    let menu = right_click_at(&mut session, at, "a Recycle Bin item");
    assert_eq!(menu.strip_labels(), ["Cut", "Delete"]);
    let rows: Vec<String> = menu
        .row_labels()
        .into_iter()
        .filter(|label| label != "Share with")
        .collect();
    assert_eq!(
        rows,
        ["Restore", "Compress to", "Properties", "-", "Show more options"]
    );
    // Restore, Delete and Properties run; Cut, Compress to and Share with
    // are grayed, saying why: the app compresses and shares files, not
    // the Recycle Bin's entries (Compress to did nothing there before).
    assert_eq!(
        menu.strip_states(),
        [("Cut".to_owned(), false), ("Delete".to_owned(), true)]
    );
    assert!(enabled(&menu, "Restore") && enabled(&menu, "Properties"));
    assert!(!enabled(&menu, "Compress to"));
    assert_eq!(
        menu.disabled_reason_of("Compress to").as_deref(),
        Some("Restore the item to compress or share it.")
    );
    close(&mut session, &menu);

    session.test.window.folder_model().select_none();
    let at = on_blank(&session);
    let menu = right_click_at(&mut session, at, "the Recycle Bin's blank space");
    assert_eq!(
        menu.row_labels(),
        [
            "View",
            "Sort by",
            "Group by",
            "Refresh",
            "-",
            "Empty Recycle Bin",
            "Undo Delete",
            "Properties",
            "-",
            "Show more options"
        ]
    );
    close(&mut session, &menu);

    // "Show more options" keeps the Recycle Bin's classic menus.
    choose_menu_style(&session.test, ContextMenu::Win10);
    let at = on_item(&session, "Meeting notes.odt");
    let menu = right_click_at(&mut session, at, "a Recycle Bin item, classic");
    assert_eq!(
        menu.row_labels(),
        ["Restore", "Delete permanently", "-", "Properties"]
    );
    close(&mut session, &menu);
}

/// Inside a ZIP opened like a folder: an item has a short classic menu in
/// either style, Open in bold, with no "Show more options"; blank space
/// has View, Sort by and Group by, then Extract all…, then "Show more
/// options" in the compact style.
///
/// parity: ARC-026, CMD-010
#[gtk::test]
fn the_menus_inside_a_zip_are_windows_11s() {
    let fixture = super::archives::fixture_with_zip();
    let mut session = Session::open_on(fixture, None);
    let choose = ox_core::settings::PreferencesUpdate {
        zip_opening: Some(ox_core::settings::ZipOpening::Folder),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    session
        .test
        .context
        .update_preferences(choose, |result| result.expect("saved"));
    session.test.select_named("Bundle.zip");
    session.test.activate("open", None);
    wait_until("readme.txt", || {
        session.test.names().contains(&"readme.txt".to_owned())
    });
    for style in [ContextMenu::Win11, ContextMenu::Win10] {
        choose_menu_style(&session.test, style);
        let at = on_item(&session, "readme.txt");
        let menu = right_click_at(&mut session, at, "an item inside a ZIP");
        assert_eq!(menu.style(), MenuStyle::Classic, "{style:?}");
        assert_eq!(
            menu.row_labels(),
            [
                "Open",
                "-",
                "Cut",
                "Copy",
                "Copy as path",
                "-",
                "Delete",
                "-",
                "Extract all…",
                "-",
                "Properties"
            ],
            "{style:?}"
        );
        assert_eq!(menu.emphasised_labels(), ["Open"]);
        close(&mut session, &menu);
    }
    choose_menu_style(&session.test, ContextMenu::Win11);
    session.test.window.folder_model().select_none();
    let at = on_blank(&session);
    let menu = right_click_at(&mut session, at, "blank space inside a ZIP");
    assert_eq!(
        menu.row_labels(),
        [
            "View",
            "Sort by",
            "Group by",
            "Refresh",
            "-",
            "Extract all…",
            "-",
            "Show more options"
        ]
    );
    close(&mut session, &menu);
}

/// This PC's, Home's and Network's blank space, compact: This PC has Add
/// a network location, then Undo and Redo only while they can run; Home
/// has Sort by and Group by grayed and Undo and Redo while they can run;
/// Network has View, Sort by, Group by and the app's Refresh, without
/// Open in Terminal.
///
/// parity: CMD-011, HOME-013, CMD-010
#[gtk::test]
fn the_blank_space_of_this_pc_home_and_network_has_windows_11s_menu() {
    let fixture = Fixture::real_world();
    let root = fixture.uri();
    let mut session = Session::open_on(fixture, None);
    super::home_page::open_quick_access_on_home(&session.test, true);
    choose_menu_style(&session.test, ContextMenu::Win11);

    show(&session, PC_URI);
    let at = on_page_blank(&session);
    let menu = right_click_at(&mut session, at, "This PC's blank space");
    let rows: Vec<String> = menu
        .row_labels()
        .into_iter()
        .filter(|label| label != "Properties")
        .collect();
    assert_eq!(
        rows,
        [
            "View",
            "Sort by",
            "Group by",
            "-",
            "Add a network location",
            "-",
            "Show more options"
        ]
    );
    close(&mut session, &menu);

    show(&session, HOME_URI);
    let at = on_page_blank(&session);
    let menu = right_click_at(&mut session, at, "Home's blank space");
    assert_eq!(
        menu.row_labels(),
        ["View", "Sort by", "Group by", "-", "Show more options"]
    );
    assert!(enabled(&menu, "View"));
    assert!(!enabled(&menu, "Sort by") && !enabled(&menu, "Group by"));
    close(&mut session, &menu);

    show(&session, NETWORK_URI);
    session.test.window.folder_model().select_none();
    let at = on_page_blank(&session);
    let menu = right_click_at(&mut session, at, "Network's blank space");
    assert_eq!(
        menu.row_labels(),
        ["View", "Sort by", "Group by", "Refresh", "-", "Show more options"]
    );
    close(&mut session, &menu);

    // Something to undo and redo: a deletion taken back.
    show(&session, &root);
    delete(&session, "Holiday song.mp3");
    session.test.activate("undo", None);
    wait_until("the file back", || {
        session.test.names().contains(&"Holiday song.mp3".to_owned())
    });
    delete(&session, "Beach clip.mp4");
    for (uri, place) in [(PC_URI, "This PC"), (HOME_URI, "Home")] {
        show(&session, uri);
        let at = on_page_blank(&session);
        let menu = right_click_at(&mut session, at, &format!("{place}'s blank space"));
        let rows = menu.row_labels();
        assert!(rows.contains(&"Undo Delete".to_owned()), "{place}: {rows:?}");
        close(&mut session, &menu);
    }

    // Shift+F10's classic menu is the same, without "Show more options".
    choose_menu_style(&session.test, ContextMenu::Win10);
    show(&session, PC_URI);
    let at = on_page_blank(&session);
    let menu = right_click_at(&mut session, at, "This PC's blank space, classic");
    let rows = menu.row_labels();
    assert!(rows.contains(&"Add a network location".to_owned()), "{rows:?}");
    assert!(!rows.contains(&"Show more options".to_owned()), "{rows:?}");
    close(&mut session, &menu);
}

/// The navigation pane's Home row offers Windows 11's "Pin current folder
/// to Quick access" after Open in new tab and Open in new window, which
/// pins the folder shown, chosen with a real click.
///
/// parity: SIDE-010, SIDE-007
#[gtk::test]
fn home_in_the_navigation_pane_pins_the_current_folder() {
    let fixture = Fixture::real_world();
    let documents = fixture.uri_of("Documents");
    let mut session = Session::open_on(fixture, None);
    show(&session, &documents);
    for style in [ContextMenu::Win11, ContextMenu::Win10] {
        choose_menu_style(&session.test, style);
        let at = super::menu_sweep::on_sidebar_row(&session, "Home");
        let menu = right_click_at(&mut session, at, "Home's row");
        let rows = menu.row_labels();
        let tab = rows.iter().position(|row| row == "Open in new tab");
        let pin = rows
            .iter()
            .position(|row| row == "Pin current folder to Quick access");
        assert!(
            tab.is_some() && pin == tab.map(|tab| tab + 2),
            "{style:?}: {rows:?}"
        );
        close(&mut session, &menu);
    }
    choose_menu_style(&session.test, ContextMenu::Win11);
    let at = super::menu_sweep::on_sidebar_row(&session, "Home");
    let menu = right_click_at(&mut session, at, "Home's row");
    let row = session.middle_of(&menu.row("Pin current folder to Quick access"));
    session.click_at(row);
    let sidebar = session.test.window.sidebar().clone();
    wait_until("Documents pinned", || session.test.window.is_pinned(&documents));
    assert!(
        sidebar
            .labels()
            .iter()
            .filter(|label| *label == "Documents")
            .count()
            >= 1
    );
}

/// A network computer's row in the navigation pane offers Windows 11's
/// Pin to Quick access after Open in new window (checked on build 26200),
/// which pins the computer, chosen with a real click; it has no
/// Properties, as Windows' has none.
///
/// parity: SIDE-020, SIDE-007
#[gtk::test]
fn a_network_computer_in_the_navigation_pane_pins_to_quick_access() {
    let mut session = Session::open_on(Fixture::real_world(), None);
    session.test.window.context().remember_network("smb://nas/");
    let sidebar = session.test.window.sidebar().clone();
    wait_until("the computer's row", || {
        sidebar.labels().contains(&"nas".to_owned())
    });
    wait_for_frames(&session.test.window, 3);
    for style in [ContextMenu::Win11, ContextMenu::Win10] {
        choose_menu_style(&session.test, style);
        let at = super::menu_sweep::on_sidebar_row(&session, "nas");
        let menu = right_click_at(&mut session, at, "a network computer's row");
        let rows = menu.row_labels();
        assert_eq!(
            rows[..4],
            [
                "Open",
                "Open in new tab",
                "Open in new window",
                "Pin to Quick access"
            ],
            "{style:?}"
        );
        assert!(!rows.contains(&"Properties".to_owned()), "{rows:?}");
        close(&mut session, &menu);
    }
    choose_menu_style(&session.test, ContextMenu::Win11);
    let at = super::menu_sweep::on_sidebar_row(&session, "nas");
    let menu = right_click_at(&mut session, at, "a network computer's row");
    let row = session.middle_of(&menu.row("Pin to Quick access"));
    session.click_at(row);
    wait_until("the computer pinned", || {
        session.test.window.is_pinned("smb://nas/")
    });
}
