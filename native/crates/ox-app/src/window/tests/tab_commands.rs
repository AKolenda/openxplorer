// SPDX-License-Identifier: AGPL-3.0-only
//! Dolphin's tab commands: tab numbers, Close other tabs, reopening closed
//! tabs, a double-click on a tab and Ctrl+Q (TAB-006, TAB-007, TAB-014 to
//! TAB-016, TAB-058).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::file_ops_support::{press_shortcut, press_shortcut_where_focused};
use super::support::{click_gesture, middle_of};
use crate::application::AppAction;
use crate::test_support::harness::{application, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuEntry;
use crate::window::session::TabId;
use crate::window::title_bar::open_windows_menu;

fn tab_ids(test: &TestWindow) -> Vec<TabId> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

/// Runs the window action `name` with `target`.
fn activate_with(test: &TestWindow, name: &str, target: &glib::Variant) {
    WidgetExt::activate_action(&test.window, &format!("win.{name}"), Some(target))
        .expect("the window has the action");
}

/// A window with tabs on `fixture`, its Documents folder and `fixture`
/// again, the last one in front.
fn three_tabs(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    for uri in [fixture.uri_of("Documents"), fixture.uri()] {
        test.window.add_tab(&uri).expect("valid folder");
        test.wait_for_listing("the new tab");
    }
    test
}

/// parity: TAB-006, TAB-007
#[gtk::test]
fn tab_numbers_and_ctrl_page_keys_show_tabs() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let [left, middle, right] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };

    activate_with(&test, "show-tab-number", &2_u32.to_variant());
    let second = test.active_tab();
    activate_with(&test, "show-tab-number", &0_u32.to_variant());
    let last = test.active_tab();
    activate_with(&test, "show-tab-number", &9_u32.to_variant());
    let past_the_end = test.active_tab();
    test.activate("next-tab", None);

    assert_eq!(second, Some(middle));
    assert_eq!(last, Some(right));
    assert_eq!(past_the_end, Some(right), "a missing tab number changes nothing");
    assert_eq!(test.active_tab(), Some(left), "Ctrl+Page Down wraps around");
}

/// A double-click on a tab behind opens a copy of it in front. The first
/// click shows the tab, which builds the tabs anew, so the strip counts the
/// clicks; it claims the second press, which keeps the title bar from
/// maximizing the window.
///
/// parity: TAB-014
#[gtk::test]
fn double_clicking_a_tab_opens_a_copy_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    let strip = test.window.tab_strip();
    let first_tab = || strip.tab_list().first_child().expect("the window has a tab");
    wait_until("the tabs to be laid out", || first_tab().width() > 0);
    let first = first_tab();
    let point = middle_of(&first, strip);

    let strip_click = click_gesture(strip, gtk::gdk::BUTTON_PRIMARY);
    strip_click.emit_by_name::<()>("pressed", &[&1_i32, &point.0, &point.1]);
    click_gesture(&first, gtk::gdk::BUTTON_PRIMARY)
        .emit_by_name::<()>("released", &[&1_i32, &point.0, &point.1]);
    wait_until("the tabs built anew to be laid out", || {
        let tab = first_tab();
        tab != first && tab.width() > 0
    });
    // GTK counts the press on the tabs built anew as a first press.
    strip_click.emit_by_name::<()>("pressed", &[&1_i32, &point.0, &point.1]);

    assert_eq!(test.window.tab_count(), 3);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(
        test.active_tab(),
        tab_ids(&test).get(1).copied(),
        "the copy is in front, next to its tab (TAB-017)"
    );
}

/// parity: TAB-015
#[gtk::test]
fn close_other_tabs_keeps_only_the_chosen_tab() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let middle = tab_ids(&test)[1];

    activate_with(&test, "close-other-tabs", &middle.to_variant());

    assert_eq!(tab_ids(&test), [middle]);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// Ctrl+Shift+T puts the last closed tab back where it was, with its
/// history; the windows menu lists the closed tabs, most recent first, and
/// reopens any of them. Ctrl+Shift+T is off while no tab has closed.
///
/// parity: TAB-016
#[gtk::test]
fn a_closed_tab_reopens_where_it_was_with_its_history() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let reopen_at_first = test.window.is_action_enabled("reopen-closed-tab");
    let [first, middle, last] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the last tab's subfolder");
    activate_with(&test, "close-tab-by-id", &first.to_variant());
    activate_with(&test, "close-tab-by-id", &last.to_variant());
    let menu = reopen_items(&test);

    activate_with(&test, "restore-closed-tab", &1_u32.to_variant());
    test.wait_for_listing("the older tab, from the menu");
    let restored = test.active_tab();
    test.activate("reopen-closed-tab", None);
    test.wait_for_listing("the most recent tab");
    let reopened = test.active_tab();

    let root = fixture
        .root()
        .file_name()
        .expect("a named folder")
        .to_string_lossy()
        .into_owned();
    assert!(!reopen_at_first, "nothing to reopen yet");
    assert_eq!(
        menu,
        [
            ("Reopen Documents".to_owned(), Some("Ctrl+Shift+T".to_owned())),
            (format!("Reopen {root}"), None),
        ]
    );
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert!(test.window.is_action_enabled("back"), "the history came back");
    assert_eq!(
        tab_ids(&test).into_iter().map(Some).collect::<Vec<_>>(),
        [restored, reopened, Some(middle)],
        "each back where it was"
    );
    assert_eq!(test.window.closed_tabs().len(), 0);
    assert!(!test.window.is_action_enabled("reopen-closed-tab"));
}

/// The "Reopen" items of the windows menu, with their shortcuts.
fn reopen_items(test: &TestWindow) -> Vec<(String, Option<String>)> {
    let entries = open_windows_menu(&test.window.imp().open_windows_button);
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Divider => None,
        })
        .filter(|item| item.label.starts_with("Reopen "))
        .map(|item| (item.label, item.shortcut.map(str::to_owned)))
        .collect()
}

/// The tab keys go through the window's capture-phase shortcuts: Ctrl+T
/// from the list opens a tab but is left to the search box when it has
/// focus; Alt+1, Ctrl+W and Ctrl+Shift+T work from the list.
///
/// parity: TAB-001, TAB-005, TAB-006, TAB-016
#[gtk::test]
fn the_tab_keys_reach_the_window_through_its_shortcuts() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let first_tab = tab_ids(&test)[0];

    press_shortcut(&test, gtk::gdk::Key::t, control);
    let after_ctrl_t = test.window.tab_count();
    test.window.search_box().entry().grab_focus();
    let search_took_key = !press_shortcut_where_focused(&test, gtk::gdk::Key::t, control);
    let from_search = test.window.tab_count();
    press_shortcut(&test, gtk::gdk::Key::_1, gtk::gdk::ModifierType::ALT_MASK);
    let first = test.active_tab();
    press_shortcut(&test, gtk::gdk::Key::w, control);
    let after_ctrl_w = test.window.tab_count();
    press_shortcut(
        &test,
        gtk::gdk::Key::t,
        control | gtk::gdk::ModifierType::SHIFT_MASK,
    );
    test.wait_for_listing("the reopened tab");

    assert_eq!((after_ctrl_t, from_search), (2, 2));
    assert!(search_took_key, "the window leaves Ctrl+T to the text field");
    assert_eq!(first, Some(first_tab), "Alt+1 shows the first tab");
    assert_eq!(after_ctrl_w, 1);
    assert_eq!(test.window.tab_count(), 2, "Ctrl+Shift+T reopened it");
}

/// parity: TAB-058
#[gtk::test]
fn ctrl_q_quits() {
    let keys = application().accels_for_action(&AppAction::Quit.detailed_name());

    assert_eq!(
        keys.iter().map(glib::GString::as_str).collect::<Vec<_>>(),
        ["<Control>q"]
    );
}

/// Open in new tabs opens every selected folder behind the current tab
/// and skips the files.
///
/// parity: TAB-027
#[gtk::test]
fn open_in_new_tabs_opens_each_selected_folder() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.root().join("Music")).expect("a new folder in the fixture");
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_all();

    WidgetExt::activate_action(&test.window, "win.open-selection-in-tabs", None)
        .expect("the window has the action");

    let uris: Vec<String> = {
        let session = test.window.imp().session.borrow();
        session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
    };
    assert_eq!(uris.len(), 3, "one tab per folder, none for the files");
    assert!(uris.contains(&fixture.uri_of("Documents")));
    assert!(uris.contains(&fixture.uri_of("Music")));
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}
