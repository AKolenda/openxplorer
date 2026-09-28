// SPDX-License-Identifier: AGPL-3.0-only
//! Screenshots of the window for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (passed through the isolated check
//! environment), these save `native-browsing-light.png`,
//! `native-browsing-narrow.png`, `native-browsing-dark.png`,
//! `native-tabs-3-light.png`, `native-tabs-3-dark.png`,
//! `native-this-pc.png`, `native-network.png`, `native-menu-new-*.png`,
//! `native-menu-context-*.png` and `native-states-*.png` there. Without
//! it, they only prove the window lays out in both themes and on both
//! pages.

use std::time::Duration;

use gtk::prelude::*;

use super::support::{app_menu, menu_button_with_class};
use crate::locations::Page;
use crate::test_support::harness::{
    capture, capture_popover, descendants, wait_for, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::widget_tree::children;

/// Longer than the skin's 83 ms colour transitions (ui-spec.md M01).
const TRANSITION_TIME: Duration = Duration::from_millis(150);

/// The theme keys, as `win.theme` takes them.
const THEMES: [&str; 2] = ["light", "dark"];

#[gtk::test]
fn the_window_is_captured_light_narrow_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.activate("theme", Some("light"));
    capture(&test.window, "native-browsing-light.png");
    test.window.set_default_size(1000, 720);
    capture(&test.window, "native-browsing-narrow.png");
    test.window.set_default_size(1320, 810);
    test.activate("theme", Some("dark"));
    capture(&test.window, "native-browsing-dark.png");
}

/// Three tabs with the middle one in front, as the reference
/// `current-*-tabs-3.png` shows them.
#[gtk::test]
fn three_tabs_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    let middle = test.active_tab().expect("a tab in front");
    test.window.add_tab(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    test.activate_tab(middle);
    test.wait_for_listing("the middle tab");
    test.activate("theme", Some("light"));
    capture(&test.window, "native-tabs-3-light.png");
    test.activate("theme", Some("dark"));
    capture(&test.window, "native-tabs-3-dark.png");
}

#[gtk::test]
fn the_landing_pages_are_captured() {
    let _theme = ThemeGuard::keep();
    let test = TestWindow::open(Page::ThisPc.uri());
    test.activate("theme", Some("light"));
    capture(&test.window, "native-this-pc.png");
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    capture(&test.window, "native-network.png");
}

/// The New menu and the file list's context menu, in both themes.
#[gtk::test]
fn the_menus_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let new_button = menu_button_with_class(&test, "new-command");
    let new_menu = app_menu(&new_button);
    let view = test.window.content().view_widget();
    let context_menu = descendants::<gtk::PopoverMenu>(&view)
        .into_iter()
        .next()
        .expect("the file list has a context menu");
    for theme in THEMES {
        test.activate("theme", Some(theme));
        new_button.popup();
        capture_popover(
            &test.window,
            new_menu.upcast_ref(),
            &format!("native-menu-new-{theme}.png"),
        );
        new_button.popdown();
        wait_until("the New menu to close", || !new_menu.is_mapped());
        context_menu.popup();
        capture_popover(
            &test.window,
            context_menu.upcast_ref(),
            &format!("native-menu-context-{theme}.png"),
        );
        context_menu.popdown();
        wait_until("the context menu to close", || !context_menu.is_mapped());
    }
}

/// Hover on a caption button, "+", an inactive tab and a file row, with
/// Sort pressed and a row selected, in both themes. (A list box manages its
/// rows' hover itself, so the sidebar cannot be hovered this way.)
#[gtk::test]
fn hover_and_pressed_states_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folder_tab = test.active_tab().expect("a tab in front");
    test.window.add_tab(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    test.activate_tab(folder_tab);
    test.wait_for_listing("the folder tab");
    test.window.folder_model().select_only(1);
    let sort = menu_button_with_class(&test, "sort-command");
    let sort_button = sort.first_child().expect("a menu button holds a button");
    sort_button.set_state_flags(gtk::StateFlags::ACTIVE, false);
    for theme in THEMES {
        test.activate("theme", Some(theme));
        // A new theme redraws the tabs and the sidebar with new widgets.
        for widget in hovered_widgets(&test) {
            widget.set_state_flags(gtk::StateFlags::PRELIGHT, false);
        }
        wait_for(TRANSITION_TIME);
        capture(&test.window, &format!("native-states-{theme}.png"));
    }
}

/// One widget of each kind the states capture hovers.
fn hovered_widgets(test: &TestWindow) -> Vec<gtk::Widget> {
    let buttons = descendants::<gtk::Button>(&test.window);
    let minimize = buttons.iter().find(|button| button.has_css_class("minimize"));
    let new_tab = buttons
        .iter()
        .find(|button| button.action_name().as_deref() == Some("win.new-tab"));
    let tab_list = test.window.chrome().tabs.tab_list();
    let inactive_tab = children(tab_list).find(|tab| !tab.has_css_class("active"));
    let file_row = descendants::<gtk::Widget>(&test.window.content().view_widget())
        .into_iter()
        .filter(|widget| widget.css_name() == "row")
        .nth(2);
    [
        minimize.map(|button| button.clone().upcast()),
        new_tab.map(|button| button.clone().upcast()),
        inactive_tab,
        file_row,
    ]
    .into_iter()
    .flatten()
    .collect()
}
