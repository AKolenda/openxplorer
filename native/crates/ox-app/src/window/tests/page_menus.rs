// SPDX-License-Identifier: AGPL-3.0-only
//! The right-click menus of the Recycle Bin, of a ZIP opened like a
//! folder, and of the blank space of This PC, Home and Network, opened
//! with the real pointer on check.py's private display, as Windows 11
//! File Explorer lists them (checked on Windows 11 build 26200,
//! 2026-11-21). The app's own Refresh stays where it was.

use ox_core::settings::ContextMenu;

use super::command_bar_switch::Session;
use super::context_menus::choose_menu_style;
use super::menu_sweep::{close, open_menu, summary};
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture};
use crate::test_support::pointer::Button;
use crate::window::menu_popover::MenuPopover;

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

/// Shows `uri` and waits until it is drawn.
fn show(session: &Session, uri: &str) {
    session.test.activate("go-to", Some(uri));
    wait_until(&format!("{uri} to show"), || {
        session.test.window.current_uri().as_deref() == Some(uri)
    });
    wait_for_frames(&session.test.window, 6);
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
