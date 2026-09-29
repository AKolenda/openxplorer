// SPDX-License-Identifier: AGPL-3.0-only
//! Dolphin's tab commands: tab numbers, Close other tabs, reopening closed
//! tabs, a double-click on a tab and Ctrl+Q (TAB-006, TAB-007, TAB-014 to
//! TAB-016, TAB-058).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::application::AppAction;
use crate::test_support::harness::{application, Fixture, TestWindow};
use crate::window::session::TabId;

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

/// parity: TAB-014
#[gtk::test]
fn double_clicking_a_tab_opens_a_copy_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let tab = test
        .window
        .tab_strip()
        .tab_list()
        .first_child()
        .expect("the window has a tab");
    let click = tab
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .find(|click| click.button() == gtk::gdk::BUTTON_PRIMARY)
        .expect("a tab takes primary clicks");

    click.emit_by_name::<()>("released", &[&2_i32, &4.0_f64, &4.0_f64]);

    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_ne!(test.active_tab(), tab_ids(&test).first().copied());
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
/// history, and the windows menu lists the closed tabs.
///
/// parity: TAB-016
#[gtk::test]
fn a_closed_tab_reopens_where_it_was_with_its_history() {
    let fixture = Fixture::standard();
    let test = three_tabs(&fixture);
    let middle = tab_ids(&test)[1];
    test.activate_tab(middle);
    test.window.navigate(&fixture.uri()).expect("a folder");
    test.wait_for_listing("the parent folder");
    activate_with(&test, "close-tab-by-id", &middle.to_variant());
    let listed = test
        .window
        .closed_tabs()
        .iter()
        .map(|tab| tab.uri().to_owned())
        .collect::<Vec<_>>();

    test.activate("reopen-closed-tab", None);
    test.wait_for_listing("the reopened tab");

    assert_eq!(listed, [fixture.uri()]);
    assert_eq!(test.window.tab_count(), 3);
    assert_eq!(test.active_tab(), Some(tab_ids(&test)[1]), "back in the middle");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.window.is_action_enabled("back"), "the history came back");
    assert!(test.window.closed_tabs().is_empty());
    test.activate("reopen-closed-tab", None);
    assert_eq!(test.window.tab_count(), 3, "nothing more to reopen");
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
