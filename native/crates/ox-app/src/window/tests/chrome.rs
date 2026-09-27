// SPDX-License-Identifier: AGPL-3.0-only
//! The frame's geometry and controls, measured against the current app.
//!
//! The current app has a 42-pixel title bar whose active tab starts 9
//! pixels in and runs into the navigation row, the "+" right after the
//! last tab, 46-pixel caption buttons, and 34-pixel address and search
//! boxes (see [`super::geometry`] for where the numbers come from).

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out};
use crate::test_support::harness::{descendants, wait_for_frames, Fixture};

/// parity: TAB-010
#[gtk::test]
fn the_active_tab_starts_9_pixels_in_and_reaches_the_bottom_of_the_42_pixel_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let title_bar = test
        .window
        .titlebar()
        .expect("the window has the tab strip as its title bar");
    let (_, title_y, _, title_height) = bounds(&test, &title_bar);
    assert_eq!(
        (title_y, title_height),
        (0, 42),
        "the title bar is 42 pixels tall"
    );
    let tabs = test.window.chrome().tabs.tab_list();
    let first_tab = tabs.first_child().expect("one tab");
    assert_eq!(bounds(&test, &first_tab), (9, 7, 215, 35), "215 x 35 at (9, 7)");
    assert!(first_tab.has_css_class("active"));
}

#[gtk::test]
fn the_new_tab_button_follows_the_last_tab() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    wait_for_frames(&test.window, 3);
    let tabs = test.window.chrome().tabs.tab_list();
    let last_tab = tabs.last_child().expect("two tabs");
    let (tab_x, _, tab_width, _) = bounds(&test, &last_tab);
    assert_eq!(tab_x, 9 + 215 + 2, "tabs are 2 pixels apart");
    let (plus_x, plus_y, plus_width, plus_height) = bounds(&test, &button_for(&test, "win.new-tab"));
    assert_eq!(plus_x, tab_x + tab_width + 5, "5 pixels after the last tab");
    assert_eq!((plus_y, plus_width, plus_height), (8, 38, 34));
}

#[gtk::test]
fn the_caption_buttons_are_46_pixels_wide_and_as_tall_as_the_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let captions: Vec<gtk::Button> = descendants::<gtk::Button>(&test.window)
        .into_iter()
        .filter(|button| button.has_css_class("caption"))
        .collect();
    assert!(!captions.is_empty(), "the desktop's layout shows caption buttons");
    let window_width = test.window.width();
    for caption in &captions {
        let (_, y, width, height) = bounds(&test, caption);
        assert_eq!((y, width, height), (0, 46, 42), "{:?}", caption.tooltip_text());
    }
    let close = captions.iter().find(|button| button.has_css_class("close"));
    if let Some(close) = close {
        let (x, _, width, _) = bounds(&test, close);
        assert_eq!(x + width, window_width, "Close sits in the corner");
    }
}

#[gtk::test]
fn the_address_and_search_boxes_are_34_pixels_tall_14_pixels_below_the_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let chrome = test.window.chrome();
    let (address_x, address_y, _, address_height) = bounds(&test, &chrome.address.root);
    assert_eq!((address_x, address_y, address_height), (178, 56, 34));
    let (search_x, search_y, search_width, search_height) = bounds(&test, &chrome.search.root);
    assert_eq!((search_y, search_width, search_height), (56, 235, 34));
    assert_eq!(
        search_x + search_width,
        test.window.width() - 16,
        "the search box ends 16 pixels from the edge"
    );
}

#[gtk::test]
fn the_search_box_names_the_folder_it_searches() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri_of("Documents"));
    let placeholder = test.window.chrome().search.entry.placeholder_text();
    assert_eq!(placeholder.as_deref(), Some("Search Documents"));
}

#[gtk::test]
fn a_new_window_types_into_its_file_list() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focused widget");
    let view = test.window.content().view_widget();
    assert!(
        focus.is_ancestor(&view) || focus == view,
        "focus is in the file list, not {focus:?}"
    );
}
