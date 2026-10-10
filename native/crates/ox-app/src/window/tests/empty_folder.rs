// SPDX-License-Identifier: AGPL-3.0-only
//! An empty folder in a real window: its "This folder is empty" page takes
//! the folder pane's right-click menu, keys and drops, as an empty folder
//! does in Windows Explorer, so New and Paste work there.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::ContextMenu;

use super::context_menus::choose_menu_style;
use super::file_ops_support::{press_shortcut, select_names};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::PanePage;
use crate::window::menu_popover::MenuPopover;

/// Makes the empty folder "Empty" in `fixture`.
fn make_empty_folder(fixture: &Fixture) -> String {
    std::fs::create_dir(fixture.path("Empty")).expect("the empty folder is made");
    fixture.uri_of("Empty")
}

/// Goes to `uri` in `test`'s tab and returns the empty page's area, the
/// whole of the pane, once it shows.
pub(in crate::window) fn go_to_empty_folder(test: &TestWindow, uri: &str) -> gtk::Widget {
    test.activate("go-to", Some(uri));
    wait_until("the empty page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    shown_empty_page(test)
}

/// The area of the empty page that shows now.
pub(in crate::window) fn shown_empty_page(test: &TestWindow) -> gtk::Widget {
    let state = descendants::<gtk::Box>(&test.window)
        .into_iter()
        .find(|page| page.has_css_class("empty-state") && page.is_mapped())
        .expect("the empty page shows");
    state.parent().expect("the empty page's area")
}

/// The context menu of `page`, if it has one.
fn menu_of(page: &gtk::Widget) -> Option<MenuPopover> {
    descendants::<MenuPopover>(page).into_iter().next()
}

/// The controllers of `widget` of type `T`.
fn controllers<T: IsA<glib::Object>>(widget: &gtk::Widget) -> Vec<T> {
    widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<T>().ok())
        .collect()
}

/// A right-click on an empty folder opens the folder menu there, and
/// Paste in it pastes the copied file into the folder.
///
/// parity: CMD-011
#[gtk::test]
fn right_clicking_an_empty_folder_opens_its_menu_and_paste_works() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    let page = go_to_empty_folder(&test, &empty);

    let right_click = controllers::<gtk::GestureClick>(&page)
        .into_iter()
        .find(|gesture| gesture.button() == gdk::BUTTON_SECONDARY)
        .expect("the empty page takes a right-click");
    right_click.emit_by_name::<()>("pressed", &[&1_i32, &20.0_f64, &20.0_f64]);
    let menu = menu_of(&page).expect("the empty page has the folder menu");
    wait_until("the menu", || menu.is_visible());

    assert!(menu.row("New…").is_sensitive());
    assert!(menu.row("Paste").is_sensitive());
    menu.row("Paste").emit_activate();
    wait_until("the pasted copy", || fixture.path("Empty/Notes 2.txt").is_file());
}

/// The keyboard reaches an empty folder: focus follows from the list to
/// its page, the Menu key opens the folder menu there, and Backspace goes
/// back.
///
/// parity: CMD-011, NAV-004
#[gtk::test]
fn an_empty_folder_takes_the_folder_keys() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();
    let page = go_to_empty_folder(&test, &empty);

    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("something has focus");
    assert!(
        focus == page || focus.is_ancestor(&page),
        "focus is on the empty page, not {focus:?}"
    );
    test.window.open_context_menu_from_keyboard();
    let menu = menu_of(&page).expect("the empty page has the folder menu");
    wait_until("the menu", || menu.is_visible());
    menu.popdown();

    let keys = controllers::<gtk::EventControllerKey>(&page)
        .into_iter()
        .next()
        .expect("the empty page takes the folder keys");
    let _: bool = keys.emit_by_name(
        "key-pressed",
        &[
            &gdk::Key::BackSpace.into_glib(),
            &0_u32,
            &gdk::ModifierType::empty(),
        ],
    );
    wait_until("Back", || {
        test.window.current_uri().as_deref() == Some(fixture.uri().as_str())
    });
}

/// Ctrl+V pastes into an empty folder.
///
/// parity: CMD-011
#[gtk::test]
fn ctrl_v_pastes_into_an_empty_folder() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    go_to_empty_folder(&test, &empty);

    press_shortcut(&test, gdk::Key::v, gdk::ModifierType::CONTROL_MASK);

    wait_until("the pasted copy", || fixture.path("Empty/Notes 2.txt").is_file());
}

/// Whether keyboard focus is on `widget` or inside it.
fn focus_is_in(test: &TestWindow, widget: &gtk::Widget) -> bool {
    gtk::prelude::GtkWindowExt::focus(&test.window)
        .is_some_and(|focus| &focus == widget || focus.is_ancestor(widget))
}

/// Closing the folder menu on an empty folder gives the keyboard back to
/// its page, as closing it on the list gives it back to the list.
///
/// parity: CMD-011
#[gtk::test]
fn closing_the_menu_gives_the_keyboard_back_to_the_empty_page() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let page = go_to_empty_folder(&test, &empty);
    test.window.folder_pane().focus_view();
    test.window.open_context_menu_from_keyboard();
    let menu = menu_of(&page).expect("the empty page has the folder menu");
    wait_until("the menu", || menu.is_visible());

    menu.popdown();

    wait_until("the menu to close", || !menu.is_visible());
    assert!(focus_is_in(&test, &page), "focus is back on the empty page");
}

/// Pasting the first item into an empty folder shows the list, and the
/// keyboard moves there with it.
///
/// parity: CMD-011
#[gtk::test]
fn the_keyboard_follows_an_empty_folder_to_its_first_item() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    go_to_empty_folder(&test, &empty);

    press_shortcut(&test, gdk::Key::v, gdk::ModifierType::CONTROL_MASK);

    wait_until("the pasted copy to be listed", || test.names() == ["Notes 2.txt"]);
    let list = test.window.folder_pane().view_widget();
    wait_until("focus on the list", || focus_is_in(&test, &list));
}

/// When a folder's last item goes, the empty page shows and the keyboard
/// moves to it from the list.
///
/// parity: CMD-011
#[gtk::test]
fn the_keyboard_follows_a_folder_that_becomes_empty() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("One")).expect("the folder is made");
    std::fs::write(fixture.path("One/Last.txt"), "last").expect("the file is written");
    let test = TestWindow::open(&fixture.uri_of("One"));
    test.window.folder_pane().focus_view();
    let list = test.window.folder_pane().view_widget();
    assert!(focus_is_in(&test, &list));

    std::fs::remove_file(fixture.path("One/Last.txt")).expect("the file goes");
    test.activate("refresh", None);

    wait_until("the empty page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);
    assert!(focus_is_in(&test, &page), "focus is on the empty page");
}

/// The message under "This location is unavailable" can be selected and
/// copied: a click on it keeps it focused instead of moving the keyboard
/// to the page.
///
/// parity: VIEW-048
#[gtk::test]
fn a_click_on_the_unavailable_message_keeps_its_text_selectable() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);
    let message = descendants::<gtk::Label>(&page)
        .into_iter()
        .find(gtk::Label::is_selectable)
        .expect("the message is selectable");
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    message.grab_focus();
    message.select_region(0, 4);
    let bounds = message.compute_bounds(&page).expect("the message is laid out");
    let press = controllers::<gtk::GestureClick>(&page)
        .into_iter()
        .find(|gesture| gesture.button() == gdk::BUTTON_PRIMARY)
        .expect("the empty page takes clicks");

    press.emit_by_name::<()>(
        "pressed",
        &[&1_i32, &f64::from(bounds.x() + 2.0), &f64::from(bounds.y() + 2.0)],
    );

    assert!(message.has_focus(), "the message keeps the keyboard");
    assert_eq!(message.selection_bounds(), Some((0, 4)));
}

/// In split view each pane's empty page takes the folder menu for its own
/// folder.
///
/// parity: CMD-011, VIEW-059
#[gtk::test]
fn each_pane_of_a_split_tab_has_its_own_empty_page_menu() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    let page = go_to_empty_folder(&test, &empty);
    let shown_in = test.window.side_holding(&page).expect("the page is in a pane");

    assert_eq!(shown_in, test.window.active_side());
    let right_click = controllers::<gtk::GestureClick>(&page)
        .into_iter()
        .find(|gesture| gesture.button() == gdk::BUTTON_SECONDARY)
        .expect("the empty page takes a right-click");
    right_click.emit_by_name::<()>("pressed", &[&1_i32, &20.0_f64, &20.0_f64]);
    let menu = menu_of(&page).expect("the page has the folder menu");
    wait_until("the menu", || menu.is_visible());
    assert!(menu.row("New…").is_sensitive());
}

/// An empty Recycle Bin shows "Recycle Bin is empty" and its page opens
/// the Recycle Bin's own folder menu, with no Paste or New.
///
/// parity: OPS-040, CMD-011
#[gtk::test]
fn an_empty_recycle_bin_opens_its_own_menu() {
    super::file_ops_support::require_private_trash();
    let trash = glib::user_data_dir().join("Trash");
    let _ = std::fs::remove_dir_all(&trash);
    let test = TestWindow::open(ox_core::location::TRASH_URI);
    choose_menu_style(&test, ContextMenu::Win10);
    wait_until("the empty Recycle Bin", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);

    test.window.open_context_menu_from_keyboard();
    let menu = menu_of(&page).expect("the page has the folder menu");
    wait_until("the menu", || menu.is_visible());
    let labels = menu.row_labels();
    let sensitive = |label: &str| labels.iter().any(|shown| shown == label) && menu.row(label).is_sensitive();

    assert!(
        !sensitive("Paste"),
        "nothing is pasted into the Recycle Bin: {labels:?}"
    );
    assert!(!sensitive("New…"), "nor made there: {labels:?}");
}
