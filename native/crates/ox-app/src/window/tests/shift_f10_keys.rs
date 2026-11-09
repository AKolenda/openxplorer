// SPDX-License-Identifier: AGPL-3.0-only
//! Shift+F10 opens the same menu as a right-click and the Menu key, as in
//! Windows 11 File Explorer, where the Windows 10 menu shows only through
//! "Show more options" or Shift+right-click (CMD-013). The keys are
//! pressed with check.py's private display's real keyboard, with the
//! pointer left somewhere else, so the menu has to point at the item, not
//! at the pointer.

use gtk::gdk;
use gtk::prelude::*;

use super::context_menus::choose_menu_style;
use super::file_ops_support::select_names;
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Pointer};
use crate::window::menu_popover::{MenuPopover, MenuStyle};
use ox_core::settings::ContextMenu;

/// How long the keys get to take effect.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Presses Shift+F10 on the real keyboard.
pub(super) fn press_shift_f10(pointer: &mut Pointer) {
    pointer.hold_key(gdk::Key::Shift_L);
    pointer.tap_key(gdk::Key::F10);
    pointer.let_go_key(gdk::Key::Shift_L);
}

/// Leaves the pointer over the window's bottom right corner, away from
/// every item, where a menu that followed the pointer would open.
pub(super) fn park_pointer(pointer: &mut Pointer, test: &TestWindow) {
    let corner = (
        f64::from(test.window.width()) - 20.0,
        f64::from(test.window.height()) - 20.0,
    );
    let (x, y) = screen_point(pointer, &test.window, corner, (720, 500));
    pointer.move_to(x, y);
}

/// The file list's menu, once Shift+F10 opened it.
fn menu_after_shift_f10(pointer: &mut Pointer, test: &TestWindow) -> MenuPopover {
    press_shift_f10(pointer);
    wait_until("Shift+F10 to open the menu", || {
        test.window.context_menu().is_visible()
    });
    wait_for_frames(&test.window, 2);
    test.window.context_menu()
}

/// Checks that `menu` points at the row of the item at `position`, in the
/// view's coordinates, and not at the parked pointer.
fn assert_points_at_item(test: &TestWindow, menu: &MenuPopover, position: u32, case: &str) {
    let view = test.window.folder_pane().input_widget();
    let row = test
        .window
        .folder_pane()
        .owners()
        .widget_at(position)
        .and_then(|row| row.compute_bounds(&view))
        .expect("the item is shown");
    let (_, pointing) = menu.pointing_to();
    let middle_y = f64::from(pointing.y()) + f64::from(pointing.height()) / 2.0;
    assert!(
        middle_y >= f64::from(row.y()) - 1.0 && middle_y <= f64::from(row.y() + row.height()) + 1.0,
        "{case}: the menu points at the item ({pointing:?} vs {row:?})"
    );
    assert!(
        f64::from(pointing.x()) < f64::from(view.width()) - 40.0,
        "{case}: not at the pointer in the corner ({pointing:?})"
    );
}

/// Closes `menu` with Escape and checks the keyboard went back to the
/// file list.
fn escape_closes(pointer: &mut Pointer, test: &TestWindow, menu: &MenuPopover, case: &str) {
    pointer.tap_key(gdk::Key::Escape);
    wait_until(&format!("{case}: Escape to close the menu"), || {
        !menu.is_visible()
    });
    wait_for(SETTLE);
    assert!(
        test.window.folder_pane().view_has_focus(),
        "{case}: the keyboard went back to the file list"
    );
}

/// With the Settings choices as they come, Shift+F10 opens Windows 11's
/// menu, with the strip and "Show more options", for one file, one
/// folder, several items and blank space, in Details, List, Tiles, Large
/// icons and the compact view, pointing at the item; Escape closes it and
/// gives the keyboard back.
///
/// parity: CMD-013, CMD-014
#[gtk::test]
fn shift_f10_opens_the_right_click_menu_in_every_view() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    for (view, compact) in [
        ("details", false),
        ("list", false),
        ("tiles", false),
        ("large", false),
        ("details", true),
    ] {
        test.activate("view", Some(view));
        if compact != test.context.settings_data().preferences.compact_density {
            test.activate("compact-density", None);
        }
        test.wait_for_listing(view);
        wait_for_frames(&test.window, 3);
        for (case, selected) in [
            ("one file", vec!["Notes 2.txt"]),
            ("one folder", vec!["Documents"]),
            ("several items", vec!["Documents", "Résumé.txt"]),
            ("blank space", Vec::new()),
        ] {
            let case = format!("{view}{}: {case}", if compact { " compact" } else { "" });
            select_names(&test, &selected);
            test.window.folder_pane().focus_view();
            park_pointer(&mut pointer, &test);
            let menu = menu_after_shift_f10(&mut pointer, &test);

            assert_eq!(menu.style(), MenuStyle::Compact, "{case}: Windows 11's menu");
            assert_eq!(
                menu.row_labels().last().map(String::as_str),
                Some("Show more options"),
                "{case}"
            );
            if !selected.is_empty() {
                assert!(!menu.strip_labels().is_empty(), "{case}: the strip of buttons");
                let first = test.position_of(selected[0]);
                assert_points_at_item(&test, &menu, first, &case);
            }
            escape_closes(&mut pointer, &test, &menu, &case);
        }
    }
    if test.context.settings_data().preferences.compact_density {
        test.activate("compact-density", None);
    }
    pointer.move_to(720, 500);
}

/// With "Windows 10 · Classic" chosen for the right-click menu, Shift+F10
/// opens that menu too, as the right-click and the Menu key do.
///
/// parity: CMD-013
#[gtk::test]
fn shift_f10_follows_the_right_click_menus_style() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.window.folder_pane().focus_view();
    park_pointer(&mut pointer, &test);
    let menu = menu_after_shift_f10(&mut pointer, &test);
    assert_eq!(menu.style(), MenuStyle::Classic);
    escape_closes(&mut pointer, &test, &menu, "classic chosen");
    choose_menu_style(&test, ContextMenu::Win11);
    pointer.move_to(720, 500);
}

/// The navigation pane's menu, shown after Shift+F10 on the row
/// `label`, with the parked pointer.
pub(super) fn sidebar_menu_after_shift_f10(
    pointer: &mut Pointer,
    test: &TestWindow,
    label: &str,
) -> MenuPopover {
    let sidebar = test.window.sidebar();
    let index = sidebar
        .labels()
        .iter()
        .position(|shown| shown == label)
        .unwrap_or_else(|| panic!("the sidebar shows {label}"));
    let row = i32::try_from(index)
        .ok()
        .and_then(|index| sidebar.list().row_at_index(index))
        .expect("every entry has a row");
    assert!(row.grab_focus(), "{label}: the row takes the keyboard");
    park_pointer(pointer, test);
    press_shift_f10(pointer);
    let shown = || {
        crate::test_support::harness::descendants::<MenuPopover>(sidebar)
            .into_iter()
            .find(WidgetExt::is_visible)
    };
    wait_until(&format!("{label}: Shift+F10 to open the row's menu"), || {
        shown().is_some()
    });
    wait_for_frames(&test.window, 2);
    let menu = shown().expect("the menu shows");
    let (_, pointing) = menu.pointing_to();
    let bounds = row.compute_bounds(sidebar).expect("the row is laid out");
    let middle_y = f64::from(pointing.y()) + f64::from(pointing.height()) / 2.0;
    assert!(
        middle_y >= f64::from(bounds.y()) - 1.0 && middle_y <= f64::from(bounds.y() + bounds.height()) + 1.0,
        "{label}: the menu points at the row ({pointing:?} vs {bounds:?})"
    );
    menu
}

/// Shift+F10 on a navigation pane row opens the row's right-click menu,
/// Windows 11's as it comes, at the row; Escape gives the keyboard back
/// to the row.
///
/// parity: CMD-013
#[gtk::test]
fn shift_f10_opens_a_navigation_pane_rows_right_click_menu() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    wait_for_frames(&test.window, 3);
    for label in ["This PC", "Local Disk"] {
        let menu = sidebar_menu_after_shift_f10(&mut pointer, &test, label);
        assert_eq!(menu.style(), MenuStyle::Compact, "{label}: Windows 11's menu");
        assert_eq!(
            menu.row_labels().last().map(String::as_str),
            Some("Show more options"),
            "{label}"
        );
        pointer.tap_key(gdk::Key::Escape);
        wait_until(&format!("{label}: Escape to close the menu"), || {
            !menu.is_visible()
        });
        wait_for(SETTLE);
        let list = test.window.sidebar().list().clone();
        let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
        assert!(
            focus.is_some_and(|focus| focus.is_ancestor(&list)),
            "{label}: the keyboard went back to the navigation pane"
        );
    }
    pointer.move_to(720, 500);
}

/// Shift+F10 on the tab in front opens the tab's menu under it, as the
/// Menu key does; the keyboard goes back to the tab after Escape.
///
/// parity: CMD-013
#[gtk::test]
fn shift_f10_on_a_tab_opens_its_menu() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let tab = test
        .window
        .tab_strip()
        .tab_list()
        .first_child()
        .expect("the window has a tab");
    assert!(tab.grab_focus(), "the tab takes the keyboard");
    park_pointer(&mut pointer, &test);
    press_shift_f10(&mut pointer);
    let menu = test.window.tab_strip().menu();
    wait_until("Shift+F10 to open the tab's menu", || menu.is_visible());
    assert_eq!(menu.row_labels()[0], "Move tab to new window");
    assert!(
        !test.window.context_menu().is_visible(),
        "the file list's menu stays shut"
    );
    pointer.tap_key(gdk::Key::Escape);
    wait_until("Escape to close the tab's menu", || !menu.is_visible());
    wait_for(SETTLE);
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&test.window),
        Some(tab),
        "the keyboard is back on the tab"
    );
    pointer.move_to(720, 500);
}
