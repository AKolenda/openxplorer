// SPDX-License-Identifier: AGPL-3.0-only
//! Screenshots of the window for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (passed through the isolated check
//! environment), these save `native-browsing-light.png`,
//! `native-browsing-narrow.png`, `native-browsing-dark.png`,
//! `native-tabs-3-light.png`, `native-tabs-3-dark.png`,
//! `native-this-pc.png` and `native-network.png` there. Without it, they
//! only prove the window lays out in both themes and on both pages.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::locations::Page;
use crate::test_support::harness::{capture, Fixture, TestWindow, ThemeGuard};

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
    let middle = test.window.imp().session.borrow().active.expect("a tab in front");
    test.window.add_tab(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    WidgetExt::activate_action(&test.window, "win.select-tab", Some(&middle.to_variant()))
        .expect("the window has the action");
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
