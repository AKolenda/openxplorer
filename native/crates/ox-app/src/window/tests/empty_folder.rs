// SPDX-License-Identifier: AGPL-3.0-only
//! An empty folder in a real window takes the folder pane's right-click
//! menu, keys and drops, as an empty folder does in Windows Explorer, so
//! New and Paste work there. It keeps its list (see `empty_folder_look`);
//! the page other empty states show (an unavailable location, a search
//! without matches, an empty Recycle Bin) takes them too.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
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

/// Whether `test` shows an empty folder: its list, with no items and the
/// note that says so.
fn shows_empty_folder(test: &TestWindow) -> bool {
    let pane = test.window.folder_pane();
    !test.window.is_loading()
        && test.names().is_empty()
        && pane.page() == Some(PanePage::Listing)
        && pane.shows_empty_note()
}

/// Goes to the empty folder `uri` in `test`'s tab and returns the widget
/// that takes its input, the list, once it shows.
pub(in crate::window) fn go_to_empty_folder(test: &TestWindow, uri: &str) -> gtk::Widget {
    test.activate("go-to", Some(uri));
    wait_until("the empty folder", || shows_empty_folder(test));
    test.window.folder_pane().input_widget()
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
/// parity: CMD-011, CMD-013, NAV-004
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
/// parity: CMD-011, CMD-013
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

    wait_until("the empty folder", || shows_empty_folder(&test));
    let list = test.window.folder_pane().input_widget();
    assert!(focus_is_in(&test, &list), "focus stays on the list");
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

/// Space on Try again, under "This location is unavailable", reaches the
/// button, which tries again: the folder's own Space (select the current
/// item) does not take it first.
///
/// parity: VIEW-048
#[gtk::test]
fn space_on_try_again_reaches_the_button() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);
    let retry = descendants::<gtk::Button>(&page)
        .into_iter()
        .find(WidgetExt::is_visible)
        .expect("Try again shows");
    retry.grab_focus();
    let keys = controllers::<gtk::EventControllerKey>(&page)
        .into_iter()
        .next()
        .expect("the empty page takes the folder keys");

    for key in [gdk::Key::space, gdk::Key::Return] {
        let taken: bool = keys.emit_by_name(
            "key-pressed",
            &[&key.into_glib(), &0_u32, &gdk::ModifierType::empty()],
        );
        assert!(!taken, "{key:?} goes on to Try again");
    }
}

/// A window that opens on an empty folder gives its page the keyboard,
/// as a window opening on a listed folder gives the list it.
///
/// parity: CMD-011, NAV-004
#[gtk::test]
fn a_window_opening_on_an_empty_folder_gives_its_page_the_keyboard() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);

    let test = TestWindow::open(&empty);

    wait_until("the empty folder", || shows_empty_folder(&test));
    let list = test.window.folder_pane().input_widget();
    wait_until("focus on the list", || focus_is_in(&test, &list));
}

/// When the folder fills while its menu is open on the empty page, as when
/// another app copies a file in, closing the menu gives the keyboard to
/// the list that shows now, not to the hidden empty page.
///
/// parity: CMD-011, CMD-013
#[gtk::test]
fn closing_the_menu_after_the_folder_fills_gives_the_keyboard_to_the_list() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let page = go_to_empty_folder(&test, &empty);
    test.window.folder_pane().focus_view();
    test.window.open_context_menu_from_keyboard();
    let menu = menu_of(&page).expect("the empty page has the folder menu");
    wait_until("the menu", || menu.is_visible());

    std::fs::write(fixture.path("Empty/Arrived.txt"), "new").expect("the file is written");
    test.activate("refresh", None);
    wait_until("the list", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Listing)
    });
    menu.popdown();

    wait_until("the menu to close", || !menu.is_visible());
    let list = test.window.folder_pane().view_widget();
    assert!(
        focus_is_in(&test, &list),
        "focus is on the list, not {:?}",
        gtk::prelude::GtkWindowExt::focus(&test.window)
    );
}

/// "This location is unavailable" takes no drop, so its menu does not
/// offer New or Paste either: the folder could not be read.
///
/// parity: CMD-011
#[gtk::test]
fn an_unavailable_location_offers_no_new_or_paste() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    choose_menu_style(&test, ContextMenu::Win10);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);
    test.window.folder_pane().focus_view();

    test.window.open_context_menu_from_keyboard();
    let menu = menu_of(&page).expect("the page has the folder menu");
    wait_until("the menu", || menu.is_visible());

    assert!(!menu.row("New…").is_sensitive(), "New is off");
    assert!(!menu.row("Paste").is_sensitive(), "Paste is off");
}

/// A screen reader hears what "This location is unavailable" says, not
/// the list's type-to-select hint, as there is nothing to select. (An
/// empty folder keeps its list, described by its note.)
///
/// parity: CMD-011
#[gtk::test]
fn a_screen_reader_hears_what_the_empty_page_says() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);

    assert!(
        !gtk::test_accessible_has_property(&page, gtk::AccessibleProperty::Label),
        "no type-to-select label"
    );
    assert!(gtk::test_accessible_has_relation(
        &page,
        gtk::AccessibleRelation::LabelledBy
    ));
}

/// Try again with the keyboard on it, once the missing folder exists but
/// is empty, shows its empty list: Try again hides, and the keyboard
/// moves to the list, so the Menu key and Backspace still work.
///
/// parity: CMD-011, VIEW-048
#[gtk::test]
fn try_again_that_finds_an_empty_folder_gives_the_keyboard_to_its_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    let page = shown_empty_page(&test);
    let retry = descendants::<gtk::Button>(&page)
        .into_iter()
        .find(WidgetExt::is_visible)
        .expect("Try again shows");
    retry.grab_focus();
    std::fs::create_dir(fixture.path("Gone")).expect("the folder is made");

    retry.emit_clicked();

    wait_until("the empty folder", || shows_empty_folder(&test));
    let list = test.window.folder_pane().view_widget();
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("something has the keyboard");
    assert!(
        focus.is_drawable() && focus_is_in(&test, &list),
        "the list has the keyboard, not the hidden Try again: {focus:?}"
    );
}

/// A folder the user may write into but not read (mode 0333) cannot be
/// listed: "This location is unavailable". Gives its permissions back
/// when dropped, so the fixture can be removed.
struct Unreadable(std::path::PathBuf);

impl Unreadable {
    fn make(fixture: &Fixture) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let folder = fixture.path("Unreadable");
        std::fs::create_dir(&folder).expect("the folder is made");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o333))
            .expect("no read permission");
        Self(folder)
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// Goes to `uri`, which cannot be listed, and waits for its page.
fn go_to_unavailable(test: &TestWindow, uri: &str) {
    test.activate("go-to", Some(uri));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
}

/// Ctrl+V on "This location is unavailable" pastes nothing, as its menu
/// offers no Paste: the folder could not be read, even when it could be
/// written to.
///
/// parity: CMD-011
#[gtk::test]
fn ctrl_v_pastes_nothing_into_a_folder_that_could_not_be_read() {
    let fixture = Fixture::standard();
    let unreadable = Unreadable::make(&fixture);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    go_to_unavailable(&test, &fixture.uri_of("Unreadable"));

    press_shortcut(&test, gdk::Key::v, gdk::ModifierType::CONTROL_MASK);

    crate::test_support::harness::settle();
    wait_until("the window to be idle", || {
        test.window.imp().file_operations.borrow().is_idle()
    });
    assert!(!unreadable.0.join("Notes 2.txt").exists(), "nothing was pasted");
}

/// Try again on "This location is unavailable" keeps New and Paste off
/// while the folder is listed again, until a listing succeeds.
///
/// parity: CMD-011
#[gtk::test]
fn new_and_paste_stay_off_while_an_unavailable_folder_is_listed_again() {
    let fixture = Fixture::standard();
    let _unreadable = Unreadable::make(&fixture);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    go_to_unavailable(&test, &fixture.uri_of("Unreadable"));
    assert!(!super::file_ops_support::is_enabled(&test, "paste"));

    test.activate("refresh", None);
    let context = glib::MainContext::default();
    let mut enabled = 0;
    for _ in 0..200 {
        context.iteration(false);
        if super::file_ops_support::is_enabled(&test, "paste")
            || super::file_ops_support::is_enabled(&test, "new-folder")
        {
            enabled += 1;
        }
    }

    assert_eq!(enabled, 0, "New or Paste came on while listing again");
}

/// Typing on an empty page selects nothing, as there is nothing to
/// select: the letters do not start a type-to-select prefix, and
/// Backspace then goes back instead of erasing one.
///
/// parity: CMD-011, NAV-004
#[gtk::test]
fn typing_on_an_empty_page_starts_no_prefix_and_backspace_goes_back() {
    let fixture = Fixture::standard();
    let empty = make_empty_folder(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let page = go_to_empty_folder(&test, &empty);
    test.window.folder_pane().focus_view();
    let keys = controllers::<gtk::EventControllerKey>(&page)
        .into_iter()
        .next()
        .expect("the empty page takes the folder keys");
    let press = |key: gdk::Key| {
        let _: bool = keys.emit_by_name(
            "key-pressed",
            &[&key.into_glib(), &0_u32, &gdk::ModifierType::empty()],
        );
    };

    test.window.type_text("n");
    press(gdk::Key::BackSpace);

    wait_until("Back", || {
        test.window.current_uri().as_deref() == Some(fixture.uri().as_str())
    });
}

/// With "Tab key switches between split panes" on, Tab on "This location
/// is unavailable" moves through its message and Try again instead of
/// switching panes, so the keyboard can reach them.
///
/// parity: VIEW-048, VIEW-059
#[gtk::test]
fn tab_on_an_unavailable_page_moves_through_its_controls_in_split_view() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    ox_core::settings::Settings::open(test.settings_directory())
        .update_preferences(&ox_core::settings::PreferencesUpdate {
            tab_switches_split_panes: Some(true),
            ..ox_core::settings::PreferencesUpdate::default()
        })
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the choice", || {
        test.context.settings_data().preferences.tab_switches_split_panes
    });
    test.activate("split-view", None);
    go_to_unavailable(&test, &fixture.uri_of("Gone"));
    let page = shown_empty_page(&test);
    let side = test.window.active_side();
    test.window.folder_pane().focus_view();
    let keys = controllers::<gtk::EventControllerKey>(&page)
        .into_iter()
        .next()
        .expect("the page takes the folder keys");

    let taken: bool = keys.emit_by_name(
        "key-pressed",
        &[&gdk::Key::Tab.into_glib(), &0_u32, &gdk::ModifierType::empty()],
    );

    assert_eq!(test.window.active_side(), side, "the pane stays active");
    assert!(!taken, "Tab goes on to move the keyboard within the page");
}
