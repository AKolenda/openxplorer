// SPDX-License-Identifier: AGPL-3.0-only
//! Submenus driven by the real pointer and keyboard on check.py's private
//! display, as in Windows 11 File Explorer (checked on Windows 11 build
//! 26200): resting the pointer on a row with a submenu opens it beside the
//! menu without a click, resting on another row of the menu closes it,
//! moving across other rows on the way into the open submenu keeps it
//! open, a click opens it at once, Right opens it with the keyboard in it
//! and Left or Escape closes it and gives the keyboard back to its row.
//!
//! Every menu of the app with a submenu is a [`MenuPopover`], so these
//! tests check one menu of each kind: the command bar's, the file list's
//! right-click menus in both styles, the navigation pane's and a Home
//! tile's. Only real pointer motion goes through GTK's crossing and motion
//! events, popup grabs and placement the way a mouse does.

use std::fs;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use ox_core::settings::ContextMenu;

use super::command_bar_switch::{has_keyboard, menu_button, menu_of, row_of, Session};
use super::context_menus::{choose_menu_style, position_of};
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, Fixture};
use crate::window::menu_popover::{row_label, MenuEntry, MenuPopover, SUBMENU_DELAY};

/// Longer than the delay, with room for a slow machine to draw.
const PAST_THE_DELAY: Duration = Duration::from_millis(900);

/// Steps of a hand gliding the pointer from one row to another.
const GLIDE_STEPS: i32 = 6;

/// The labels of `menu`'s rows that open a submenu now.
fn submenu_rows(menu: &MenuPopover) -> Vec<String> {
    let rows = menu.row_list();
    menu.items()
        .into_iter()
        .filter(|item| !item.submenu.is_empty())
        .map(|item| item.label)
        .filter(|label| {
            std::iter::successors(rows.first_child(), WidgetExt::next_sibling)
                .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
                .any(|row| row.is_sensitive() && row_label(&row).as_deref() == Some(label))
        })
        .collect()
}

/// The first row of `menu` that can be chosen and opens no submenu.
fn plain_row(menu: &MenuPopover) -> String {
    let list = menu.row_list();
    let with_submenu: Vec<String> = menu
        .items()
        .into_iter()
        .filter(|item| !item.submenu.is_empty())
        .map(|item| item.label)
        .collect();
    std::iter::successors(list.first_child(), WidgetExt::next_sibling)
        .filter_map(|child| child.downcast::<gtk::ListBoxRow>().ok())
        .filter(|row| row.is_sensitive())
        .filter_map(|row| row_label(&row))
        .find(|label| !with_submenu.contains(label))
        .expect("the menu has a row without a submenu")
}

/// The labels the submenu of `label`'s row lists.
fn submenu_labels_of(menu: &MenuPopover, label: &str) -> Vec<String> {
    menu.items()
        .into_iter()
        .find(|item| item.label == label)
        .map(|item| {
            item.submenu
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Item(item) => Some(item.label),
                    MenuEntry::Divider => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the submenu of `label`'s row is open beside `menu` and shown.
fn shows_submenu_of(menu: &MenuPopover, label: &str) -> bool {
    menu.open_submenu_menu()
        .filter(WidgetExt::is_mapped)
        .is_some_and(|submenu| {
            let shown: Vec<String> = submenu.items().into_iter().map(|item| item.label).collect();
            shown == submenu_labels_of(menu, label)
        })
}

/// The screen point `fraction` across and half way down `label`'s row.
fn point_on_row(session: &Session, menu: &MenuPopover, label: &str, fraction: f64) -> (i32, i32) {
    let row = row_of(menu, label);
    session.on_screen(&row, (f64::from(row.width()) * fraction, f64::from(row.height()) / 2.0))
}

/// What the checks of the submenus found, for the log and the failures.
#[derive(Debug, Default)]
struct Findings {
    failures: Vec<String>,
}

impl Findings {
    /// Notes whether `what` held for the submenu of `row` in `menu`.
    fn note(&mut self, menu: &str, row: &str, what: &str, held: bool) {
        let verdict = if held { "PASS" } else { "FAIL" };
        eprintln!("{verdict}: {menu} > {row}: {what}");
        if !held {
            self.failures.push(format!("{menu} > {row}: {what}"));
        }
    }

    /// Fails the test with every check that did not hold.
    fn assert_none_failed(&self) {
        assert!(self.failures.is_empty(), "failed: {:#?}", self.failures);
    }
}

/// Checks every submenu of the open `menu`, called `name` in the log, with
/// the real pointer coming from `start` and the real keyboard: hover opens
/// it and hover on another row closes it, with the menu open throughout; a
/// click opens it before the hover delay could; Right opens it with the
/// keyboard in it, Left closes it with the keyboard back on its row, and so
/// does Escape, which leaves the menu open.
fn check_submenus(session: &mut Session, findings: &mut Findings, name: &str, menu: &MenuPopover, start: (i32, i32)) {
    let rows = submenu_rows(menu);
    assert!(!rows.is_empty(), "{name} has a submenu");
    let plain = plain_row(menu);
    let mut from = start;
    for label in rows {
        let row = row_of(menu, &label);
        // Hover opens it, after the delay.
        let on_row = point_on_row(session, menu, &label, 0.5);
        session.pointer.glide(from, on_row, GLIDE_STEPS);
        wait_for(PAST_THE_DELAY);
        let opened = shows_submenu_of(menu, &label);
        findings.note(name, &label, "hover opens", opened);
        // Hover on another row closes it; the menu stays.
        let on_plain = point_on_row(session, menu, &plain, 0.5);
        session.pointer.glide(on_row, on_plain, GLIDE_STEPS);
        wait_for(PAST_THE_DELAY);
        let closed = menu.open_submenu_menu().is_none();
        findings.note(name, &label, "hover on another row closes", opened && closed && menu.is_mapped());
        // A click opens it at once.
        let clicked = Instant::now();
        session.click_at(on_row);
        let at_once = (0..20).any(|_| {
            if shows_submenu_of(menu, &label) {
                return true;
            }
            wait_for(Duration::from_millis(15));
            false
        }) && clicked.elapsed() < SUBMENU_DELAY;
        findings.note(name, &label, "click opens at once", at_once);
        // Right opens it with the keyboard in it, Left closes it.
        row.grab_focus();
        if menu.open_submenu_menu().is_some() {
            session.pointer.tap_key(gdk::Key::Left);
        }
        wait_until("the submenu to close", || menu.open_submenu_menu().is_none());
        session.pointer.tap_key(gdk::Key::Right);
        wait_until("Right to open the submenu", || shows_submenu_of(menu, &label));
        let submenu = menu.open_submenu_menu().expect("the open submenu");
        wait_until("the keyboard in the submenu", || {
            has_keyboard(&session.test, &submenu)
        });
        session.pointer.tap_key(gdk::Key::Left);
        let left = (0..40).any(|_| {
            wait_for(Duration::from_millis(15));
            menu.open_submenu_menu().is_none() && has_keyboard(&session.test, &row)
        });
        findings.note(name, &label, "Right opens, Left closes to its row", left);
        // Escape closes only the submenu.
        session.pointer.tap_key(gdk::Key::Right);
        wait_until("Right to open the submenu again", || {
            shows_submenu_of(menu, &label)
        });
        let submenu = menu.open_submenu_menu().expect("the open submenu");
        wait_until("the keyboard in the submenu", || {
            has_keyboard(&session.test, &submenu)
        });
        session.pointer.tap_key(gdk::Key::Escape);
        let escaped = (0..40).any(|_| {
            wait_for(Duration::from_millis(15));
            menu.open_submenu_menu().is_none() && menu.is_mapped() && has_keyboard(&session.test, &row)
        });
        findings.note(name, &label, "Escape closes it to its row, the menu stays", escaped);
        from = on_row;
    }
}

/// The open menu of the folder pane, as a right-click opens it.
fn open_pane_menu(session: &Session) -> MenuPopover {
    let open = || {
        descendants::<MenuPopover>(session.test.window.folder_pane())
            .into_iter()
            .find(WidgetExt::is_mapped)
    };
    wait_until("the menu", || open().is_some());
    let menu = open().expect("an open menu");
    wait_for_frames(&menu, 3);
    menu
}

/// Where the pointer comes from into a menu opened by a right-click: its
/// top left corner, where the pointer was.
fn corner_of(session: &Session, menu: &MenuPopover) -> (i32, i32) {
    let first = menu.rows().into_iter().next().expect("a row");
    session.on_screen(&first, (2.0, 2.0))
}

/// The command bar's Sort and View menus: More, Group by and Show.
///
/// parity: CMD-005, VIEW-022
#[gtk::test]
fn the_command_bars_submenus_open_on_hover_click_and_keys() {
    let mut session = Session::open(None);
    let mut findings = Findings::default();
    for name in ["Sort", "View"] {
        session.open_by_click(name);
        let menu = menu_of(&session.test, name);
        let start = session.middle_of(&menu_button(&session.test, name));
        check_submenus(&mut session, &mut findings, name, &menu, start);
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    }
    findings.assert_none_failed();
}

/// The file list's right-click menus, compact and classic, on a file and
/// on blank space: Open with, Compress to, Send to, View, Sort by, Group
/// by and New.
///
/// parity: CMD-008, CMD-010
#[gtk::test]
fn the_right_click_menus_submenus_open_on_hover_click_and_keys() {
    let mut session = Session::open(None);
    let mut findings = Findings::default();
    for (style, item) in [
        (ContextMenu::Win11, Some("Notes 2.txt")),
        (ContextMenu::Win11, None),
        (ContextMenu::Win10, Some("Notes 2.txt")),
        (ContextMenu::Win10, None),
    ] {
        choose_menu_style(&session.test, style);
        let position = item.map(|name| position_of(&session.test, name));
        session.test.window.right_click(position);
        let menu = open_pane_menu(&session);
        let start = corner_of(&session, &menu);
        let name = format!("{style:?} menu of {}", item.unwrap_or("blank space"));
        check_submenus(&mut session, &mut findings, &name, &menu, start);
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    }
    findings.assert_none_failed();
}

/// A pinned folder's menu in the navigation pane (Compress to, New) and a
/// Quick access tile's menu on Home (Compress to).
///
/// parity: NAV-011, HOME-013
#[gtk::test]
fn the_place_menus_submenus_open_on_hover_click_and_keys() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let projects = fixture.uri_of("Projects");
    let mut session = Session::open_on(fixture, None);
    let mut findings = Findings::default();
    choose_menu_style(&session.test, ContextMenu::Win11);
    session.test.activate("pin-location", Some(&projects));
    let sidebar = session.test.window.sidebar();
    wait_until("the pin", || sidebar.labels().contains(&"Projects".to_owned()));
    wait_for_frames(&session.test.window, 3);
    let menu = sidebar.right_click_row("Projects");
    menu.popup();
    wait_until("the pin's menu", || menu.is_mapped());
    wait_for_frames(&menu, 3);
    let start = corner_of(&session, &menu);
    check_submenus(&mut session, &mut findings, "navigation pane pin", &menu, start);
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());

    super::home_page::open_quick_access_on_home(&session.test, true);
    session.test.activate("home", None);
    let body = session.test.window.folder_pane().landing().clone();
    wait_until("the Projects tile", || {
        crate::window::home_page::tile_names(&body).contains(&"Projects".to_owned())
    });
    let tile = crate::window::home_page::tile_named(&body, "Projects");
    tile.grab_focus();
    let menu = crate::window::home_page::item_menu(&body, "Projects", crate::window::context_menu::MENU_KEY);
    wait_until("the tile's menu", || menu.is_mapped());
    wait_for_frames(&menu, 3);
    let start = corner_of(&session, &menu);
    check_submenus(&mut session, &mut findings, "Home tile", &menu, start);
    menu.popdown();
    findings.assert_none_failed();
}

/// Moving the pointer slowly from a row's open submenu towards it, across
/// another row with a submenu of its own, keeps the open submenu: the
/// pointer heading for it does not count as resting on the rows it
/// crosses, however long it takes, as in Windows 11.
///
/// parity: CMD-010
#[gtk::test]
fn moving_slowly_towards_an_open_submenu_across_other_rows_keeps_it_open() {
    let mut session = Session::open(None);
    choose_menu_style(&session.test, ContextMenu::Win11);
    session.test.window.right_click(None);
    let menu = open_pane_menu(&session);
    // View, then Sort by below it: both open a submenu.
    let start = corner_of(&session, &menu);
    let on_view = point_on_row(&session, &menu, "View", 0.3);
    session.pointer.glide(start, on_view, GLIDE_STEPS);
    wait_until("View's submenu to open on hover", || {
        shows_submenu_of(&menu, "View")
    });
    let submenu = menu.open_submenu_menu().expect("View's submenu");
    wait_for_frames(&submenu, 3);
    // Down to the right across Sort by, into View's submenu below the
    // height of Sort by, slowly: longer on Sort by than the delay.
    let rows = submenu.rows();
    let inside = rows.get(3).unwrap_or_else(|| rows.last().expect("a row"));
    let target = session.on_screen(inside, (12.0, f64::from(inside.height()) / 2.0));
    let steps = 14;
    for step in 1..=steps {
        session.pointer.move_to(
            on_view.0 + (target.0 - on_view.0) * step / steps,
            on_view.1 + (target.1 - on_view.1) * step / steps,
        );
        wait_for(Duration::from_millis(70));
    }
    wait_for(PAST_THE_DELAY);
    assert!(
        shows_submenu_of(&menu, "View"),
        "View's submenu stays open while the pointer heads into it"
    );
    assert_eq!(menu.open_submenu_menu().as_ref(), Some(&submenu), "the same submenu");
    menu.popdown();
}

/// Moving the pointer away from an open submenu, down the menu, closes it
/// once the pointer rests on another row; leaving every menu then leaves
/// the menu open.
///
/// parity: CMD-010
#[gtk::test]
fn moving_away_from_an_open_submenu_closes_it_after_the_delay() {
    let mut session = Session::open(None);
    choose_menu_style(&session.test, ContextMenu::Win11);
    session.test.window.right_click(None);
    let menu = open_pane_menu(&session);
    let start = corner_of(&session, &menu);
    let on_view = point_on_row(&session, &menu, "View", 0.7);
    session.pointer.glide(start, on_view, GLIDE_STEPS);
    wait_until("View's submenu to open on hover", || {
        shows_submenu_of(&menu, "View")
    });
    // Down and to the left, away from the submenu, onto a row without one.
    let plain = plain_row(&menu);
    let away = point_on_row(&session, &menu, &plain, 0.2);
    session.pointer.glide(on_view, away, GLIDE_STEPS);
    wait_until("the submenu to close", || menu.open_submenu_menu().is_none());
    assert!(menu.is_mapped(), "the menu stays open");
    // Leaving every menu closes nothing more.
    let outside = session.on_screen(&session.test.window, (8.0, 8.0));
    session.pointer.glide(away, outside, GLIDE_STEPS);
    wait_for(PAST_THE_DELAY);
    assert!(menu.is_mapped(), "the pointer leaving the menu leaves it open");
    menu.popdown();
}

/// A submenu too wide to fit beside its menu on either side of the
/// monitor still opens on hover, over the menu, as Windows 11 places it,
/// instead of GTK squeezing it into the room left and closing it.
///
/// parity: CMD-010
#[gtk::test]
fn a_submenu_with_no_room_beside_its_menu_opens_over_it() {
    use crate::icons::Icon;
    use crate::window::menu_popover::MenuItem;
    use crate::window::window_action::WindowAction;

    let mut session = Session::open(None);
    // A submenu wider than the room on either side of a menu in the middle
    // of the monitor, as a menu's own is on a small or zoomed screen.
    let wide = gtk::CssProvider::new();
    wide.load_from_string("popover.ox-menu.submenu > contents { min-width: 900px; }");
    let display = gdk::Display::default().expect("the private display");
    gtk::style_context_add_provider_for_display(&display, &wide, gtk::STYLE_PROVIDER_PRIORITY_USER);
    let submenu = vec![
        MenuItem::new("Example item", Icon::Copy, WindowAction::CopyAddress).into(),
        MenuItem::new("Another example item", Icon::Copy, WindowAction::CopyAddress).into(),
    ];
    let menu = MenuPopover::new(vec![
        MenuItem::new("First", Icon::Copy, WindowAction::CopyAddress).into(),
        MenuItem::submenu("Example submenu", Icon::Folder, WindowAction::CopyAddress, submenu).into(),
    ]);
    let pane = session.test.window.folder_pane().clone();
    menu.set_parent(&pane);
    menu.set_pointing_to(Some(&gdk::Rectangle::new(pane.width() / 2, 40, 1, 1)));
    menu.popup();
    wait_until("the menu", || menu.is_mapped());
    wait_for_frames(&menu, 3);
    let first = point_on_row(&session, &menu, "First", 0.5);
    let on_row = point_on_row(&session, &menu, "Example submenu", 0.5);
    session.pointer.glide(first, on_row, GLIDE_STEPS);
    wait_for(PAST_THE_DELAY);
    let opened = shows_submenu_of(&menu, "Example submenu");
    let width = menu.open_submenu_menu().map_or(0, |submenu| submenu.width());
    menu.popdown();
    menu.unparent();
    gtk::style_context_remove_provider_for_display(&display, &wide);
    assert!(opened, "the submenu opens over its menu");
    assert!(width >= 900, "at its full width: {width}");
}

/// The pointer brought onto Show in one move as soon as View opens, and
/// resting there, opens Show's submenu. The window under the command
/// bar's menu, which holds no pointer grab of its own, is still drawing
/// the menu's opening, and GTK makes up a motion event for it at the
/// pointer as if the pointer had left the menu: it has not moved, and the
/// submenu still opens.
///
/// parity: CMD-005
#[gtk::test]
fn the_pointer_resting_on_show_as_view_opens_opens_its_submenu() {
    let mut session = Session::open(None);
    let mut failed = Vec::new();
    for attempt in 0..4 {
        session.click_button("View");
        let menu = menu_of(&session.test, "View");
        wait_until("View's menu to open", || menu.is_mapped());
        let on_show = point_on_row(&session, &menu, "Show", 0.5);
        session.pointer.move_to(on_show.0, on_show.1);
        wait_for(PAST_THE_DELAY);
        if !shows_submenu_of(&menu, "Show") {
            failed.push(attempt);
        }
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    }
    assert!(failed.is_empty(), "Show's submenu did not open on hover in attempts {failed:?}");
}
