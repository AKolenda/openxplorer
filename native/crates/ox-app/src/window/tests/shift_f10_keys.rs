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

/// Saves "Shift+F10 opens the classic menu" as `on` and waits for the
/// window to read it.
fn choose_shift_f10_classic(test: &TestWindow, on: bool) {
    let update = ox_core::settings::PreferencesUpdate {
        shift_f10_classic: Some(on),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    ox_core::settings::Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.shift_f10_classic == on
    });
}

/// With "Shift+F10 opens the classic menu" on, Shift+F10 opens the
/// classic menu again, on the file list and on a navigation pane row,
/// pointing at the item, while the Menu key keeps the right-click menu;
/// turned off, Shift+F10 opens the right-click menu again.
///
/// parity: CMD-013, CMD-014
#[gtk::test]
fn the_setting_makes_shift_f10_open_the_classic_menu() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    choose_shift_f10_classic(&test, true);
    wait_for_frames(&test.window, 3);

    for (case, selected) in [
        ("one file", vec!["Notes 2.txt"]),
        ("one folder", vec!["Documents"]),
        ("several items", vec!["Documents", "Résumé.txt"]),
        ("blank space", Vec::new()),
    ] {
        select_names(&test, &selected);
        test.window.folder_pane().focus_view();
        park_pointer(&mut pointer, &test);
        let menu = menu_after_shift_f10(&mut pointer, &test);
        assert_eq!(menu.style(), MenuStyle::Classic, "{case}: the classic menu");
        if let Some(first) = selected.first() {
            assert_points_at_item(&test, &menu, test.position_of(first), case);
        }
        escape_closes(&mut pointer, &test, &menu, case);
    }
    let menu = sidebar_menu_after_shift_f10(&mut pointer, &test, "This PC");
    assert_eq!(
        menu.style(),
        MenuStyle::Classic,
        "a navigation pane row: the classic menu"
    );
    pointer.tap_key(gdk::Key::Escape);
    wait_until("Escape to close the row's menu", || !menu.is_visible());

    // The Menu key keeps the right-click menu.
    select_names(&test, &["Notes 2.txt"]);
    test.window.folder_pane().focus_view();
    park_pointer(&mut pointer, &test);
    pointer.tap_key(gdk::Key::Menu);
    wait_until("the Menu key to open the menu", || {
        test.window.context_menu().is_visible()
    });
    let menu = test.window.context_menu();
    assert_eq!(menu.style(), MenuStyle::Compact, "the Menu key is unchanged");
    escape_closes(&mut pointer, &test, &menu, "the Menu key");

    choose_shift_f10_classic(&test, false);
    let menu = menu_after_shift_f10(&mut pointer, &test);
    assert_eq!(
        menu.style(),
        MenuStyle::Compact,
        "off again: the right-click menu"
    );
    escape_closes(&mut pointer, &test, &menu, "off again");
    pointer.move_to(720, 500);
}

/// Shift+right-click still opens the classic menu, the setting off or on,
/// as in Windows 11.
///
/// parity: CMD-008
#[gtk::test]
fn shift_right_click_still_opens_the_classic_menu() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    wait_for_frames(&test.window, 3);
    for on in [false, true] {
        choose_shift_f10_classic(&test, on);
        let at = super::right_drag::item_on_screen(&mut pointer, &test, "Notes 2.txt");
        pointer.move_to(at.0, at.1);
        pointer.hold_key(gdk::Key::Shift_L);
        pointer.press(crate::test_support::pointer::Button::Secondary);
        pointer.release(crate::test_support::pointer::Button::Secondary);
        pointer.let_go_key(gdk::Key::Shift_L);
        wait_until("Shift+right-click to open the menu", || {
            test.window.context_menu().is_visible()
        });
        let menu = test.window.context_menu();
        assert_eq!(menu.style(), MenuStyle::Classic, "setting {on}: the classic menu");
        pointer.tap_key(gdk::Key::Escape);
        wait_until("Escape to close the menu", || !menu.is_visible());
        wait_for(SETTLE);
    }
    pointer.move_to(720, 500);
}

/// The key text the row labelled `label` of `menu` shows, if any.
fn shortcut_text_of(menu: &MenuPopover, label: &str) -> Option<String> {
    crate::test_support::harness::descendants::<gtk::Label>(&menu.row(label))
        .into_iter()
        .find(|text| text.has_css_class("shortcut"))
        .map(|text| text.text().to_string())
}

/// Windows 11's menu shows no key beside "Show more options" (checked on
/// a Windows 11 VM), and Shift+F10 opens this same menu as it comes, so
/// the row shows no key on the file list or the navigation pane. With
/// "Shift+F10 opens the classic menu" on, Shift+F10 is the key that opens
/// what the row opens, and the row shows "Shift+F10".
///
/// parity: CMD-008, CMD-013
#[gtk::test]
fn show_more_options_shows_shift_f10_only_when_it_opens_the_classic_menu() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    for on in [false, true] {
        choose_shift_f10_classic(&test, on);
        let expected = on.then(|| "Shift+F10".to_owned());

        test.window.right_click(Some(test.position_of("Notes 2.txt")));
        let menu = test.window.context_menu();
        wait_until("the item's menu", || menu.is_visible());
        assert_eq!(
            shortcut_text_of(&menu, "Show more options"),
            expected,
            "an item, setting {on}"
        );
        menu.popdown();

        test.window.right_click(None);
        let menu = test.window.context_menu();
        wait_until("the folder's menu", || menu.is_visible());
        assert_eq!(
            shortcut_text_of(&menu, "Show more options"),
            expected,
            "the folder, setting {on}"
        );
        menu.popdown();

        let menu = test.window.sidebar().right_click_row("This PC");
        menu.popup();
        wait_until("the row's menu", || menu.is_visible());
        assert_eq!(
            shortcut_text_of(&menu, "Show more options"),
            expected,
            "This PC's row, setting {on}"
        );
        menu.popdown();
    }
    choose_shift_f10_classic(&test, false);
}

/// Taps `key` with `modifier` held on the real keyboard, `times` times.
fn tap_with(pointer: &mut Pointer, modifier: gdk::Key, key: gdk::Key, times: usize) {
    pointer.hold_key(modifier);
    for _ in 0..times {
        pointer.tap_key(key);
        wait_for(std::time::Duration::from_millis(60));
    }
    pointer.let_go_key(modifier);
    wait_for(std::time::Duration::from_millis(100));
}

/// One way of selecting several items from the keyboard before the menu
/// opens, as Windows 11 File Explorer's selection keys do.
#[derive(Debug, Clone, Copy)]
enum KeyboardSelection {
    /// Only the item clicked.
    One,
    /// Shift+Down three times from the item clicked: the keyboard ends on
    /// the last item of the range.
    ShiftDown,
    /// Shift+Up three times: the keyboard ends on the first item.
    ShiftUp,
    /// Two items, then Ctrl+Down twice and Ctrl+Space, which selects the
    /// item the keyboard moved to.
    CtrlSpaceSelects,
    /// The same, then Ctrl+Space again, which takes the item back out of
    /// the selection while the keyboard stays on it.
    CtrlSpaceUnselects,
    /// Ctrl+A with the keyboard on an item in the middle.
    CtrlA,
}

/// Clicks, as the pointer would, the item at `position`, then makes
/// `selection` with the real keyboard.
fn select_from_the_keyboard(
    pointer: &mut Pointer,
    test: &TestWindow,
    position: u32,
    selection: KeyboardSelection,
) {
    let pane = test.window.folder_pane();
    pane.select_and_reveal(position);
    wait_until("the item to take the keyboard", || {
        pane.focused_position() == Some(position)
    });
    match selection {
        KeyboardSelection::One => {}
        KeyboardSelection::ShiftDown => tap_with(pointer, gdk::Key::Shift_L, gdk::Key::Down, 3),
        KeyboardSelection::ShiftUp => tap_with(pointer, gdk::Key::Shift_L, gdk::Key::Up, 3),
        KeyboardSelection::CtrlSpaceSelects | KeyboardSelection::CtrlSpaceUnselects => {
            tap_with(pointer, gdk::Key::Shift_L, gdk::Key::Down, 1);
            tap_with(pointer, gdk::Key::Control_L, gdk::Key::Down, 2);
            tap_with(pointer, gdk::Key::Control_L, gdk::Key::space, 1);
            if matches!(selection, KeyboardSelection::CtrlSpaceUnselects) {
                tap_with(pointer, gdk::Key::Control_L, gdk::Key::space, 1);
            }
        }
        KeyboardSelection::CtrlA => tap_with(pointer, gdk::Key::Control_L, gdk::Key::a, 1),
    }
    wait_for_frames(&test.window, 2);
}

/// Where the keyboard's menu should point: at the item with keyboard
/// focus when it is selected, as Windows 11 File Explorer opens it there,
/// else at the first selected item.
fn expected_menu_item(test: &TestWindow, case: &str) -> u32 {
    let pane = test.window.folder_pane();
    let model = pane.model();
    let selected = model.selected_positions();
    assert!(!selected.is_empty(), "{case}: something is selected");
    match pane.focused_position() {
        Some(focused) if selected.contains(&focused) => focused,
        _ => selected[0],
    }
}

/// Opens the menu with Shift+F10 or, with `menu_key`, the Menu key.
fn open_menu_from_the_keyboard(pointer: &mut Pointer, test: &TestWindow, menu_key: bool) -> MenuPopover {
    if menu_key {
        pointer.tap_key(gdk::Key::Menu);
        wait_until("the Menu key to open the menu", || {
            test.window.context_menu().is_visible()
        });
        wait_for_frames(&test.window, 2);
        test.window.context_menu()
    } else {
        menu_after_shift_f10(pointer, test)
    }
}

/// With several items selected, Shift+F10 and the Menu key open the menu
/// at the item the keyboard moved to last, as Windows 11 File Explorer
/// does (with extra_2.txt clicked and Shift+Down pressed three times, at
/// the fourth item), in Details, List, Tiles, Large icons and the compact
/// view; at the first selected item when the keyboard is on an item that
/// is not selected. The pointer is parked far away.
///
/// parity: CMD-014
#[gtk::test]
fn the_keyboards_menu_opens_at_the_focused_item_of_a_selection() {
    let fixture = Fixture::with_files(12);
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win11);
    let mut moved_on = false;
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
        for selection in [
            KeyboardSelection::One,
            KeyboardSelection::ShiftDown,
            KeyboardSelection::ShiftUp,
            KeyboardSelection::CtrlSpaceSelects,
            KeyboardSelection::CtrlSpaceUnselects,
            KeyboardSelection::CtrlA,
        ] {
            for menu_key in [false, true] {
                let key = if menu_key { "Menu key" } else { "Shift+F10" };
                let case = format!(
                    "{view}{}: {selection:?}, {key}",
                    if compact { " compact" } else { "" }
                );
                let start = if matches!(selection, KeyboardSelection::ShiftUp) {
                    5
                } else {
                    2
                };
                park_pointer(&mut pointer, &test);
                select_from_the_keyboard(&mut pointer, &test, start, selection);
                let expected = expected_menu_item(&test, &case);
                let first = test.window.folder_pane().model().first_selected();
                moved_on |= first != Some(expected);
                let menu = open_menu_from_the_keyboard(&mut pointer, &test, menu_key);
                assert!(!menu.strip_labels().is_empty(), "{case}: the item's menu");
                assert_points_at_item(&test, &menu, expected, &case);
                escape_closes(&mut pointer, &test, &menu, &case);
            }
        }
    }
    assert!(
        moved_on,
        "some cases have the keyboard on another item than the first"
    );
    if test.context.settings_data().preferences.compact_density {
        test.activate("compact-density", None);
    }
    pointer.move_to(720, 500);
}

/// The selection's focused item scrolled out of sight comes back into
/// view first, as a single item does, and the menu points at it there.
///
/// parity: CMD-014
#[gtk::test]
fn the_keyboards_menu_scrolls_to_the_focused_item_first() {
    let fixture = Fixture::with_files(150);
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
        for menu_key in [false, true] {
            let case = format!(
                "{view}{}: {}",
                if compact { " compact" } else { "" },
                if menu_key { "Menu key" } else { "Shift+F10" }
            );
            park_pointer(&mut pointer, &test);
            select_from_the_keyboard(&mut pointer, &test, 2, KeyboardSelection::ShiftDown);
            let expected = expected_menu_item(&test, &case);
            assert_ne!(
                Some(expected),
                test.window.folder_pane().model().first_selected(),
                "{case}"
            );
            // The list scrolled to its end, away from the selection.
            let view_widget = test.window.folder_pane().view_widget();
            let scroller = view_widget
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
                .expect("the view scrolls");
            // The compact list's columns scroll sideways.
            let vertical = scroller.vadjustment();
            let adjustment = if vertical.upper() > vertical.page_size() + 1.0 {
                vertical
            } else {
                scroller.hadjustment()
            };
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            wait_for_frames(&test.window, 4);
            // On screen when a press in the middle of its row or tile
            // would land on it: GTK keeps the focused row's widget, where
            // it was, when it scrolls away.
            let shown = |position: u32| {
                let pane = test.window.folder_pane();
                let view = pane.input_widget();
                pane.owners()
                    .widget_at(position)
                    .and_then(|row| row.compute_bounds(&view))
                    .and_then(|bounds| {
                        let center = bounds.center();
                        pane.owners()
                            .position_at(&view, f64::from(center.x()), f64::from(center.y()))
                    })
                    == Some(position)
            };
            assert!(!shown(expected), "{case}: the focused item is out of sight");

            let menu = open_menu_from_the_keyboard(&mut pointer, &test, menu_key);
            assert!(shown(expected), "{case}: the focused item came into view");
            assert_points_at_item(&test, &menu, expected, &case);
            escape_closes(&mut pointer, &test, &menu, &case);
        }
    }
    if test.context.settings_data().preferences.compact_density {
        test.activate("compact-density", None);
    }
    pointer.move_to(720, 500);
}
