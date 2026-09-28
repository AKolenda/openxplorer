// SPDX-License-Identifier: AGPL-3.0-only
//! The frame's geometry and controls, measured against the current app.
//!
//! The current app has a 42-pixel title bar whose active tab starts 9
//! pixels in and runs into the navigation row, the "+" right after the
//! last tab, 46-pixel caption buttons, and 34-pixel address and search
//! boxes (see [`super::geometry`] for where the numbers come from).
//!
//! The frame's templates (`resources/ui/`) leave two things to Rust: the
//! window actions of their buttons and their glyphs, which name bundled
//! icons. The last tests prove that no control is left without either.

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out, Bounds};
use super::support::{app_menu, menu_button_with_class};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, Fixture, TestWindow};

/// parity: TAB-010
#[gtk::test]
fn the_active_tab_starts_9_pixels_in_and_reaches_the_bottom_of_the_42_pixel_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let title_bar = test
        .window
        .titlebar()
        .expect("the window has the tab strip as its title bar");
    let title = bounds(&test, &title_bar);
    assert_eq!(
        (title.y, title.height),
        (0, 42),
        "the title bar is 42 pixels tall"
    );
    let tabs = test.window.tab_strip().tab_list();
    let first_tab = tabs.first_child().expect("one tab");
    assert_eq!(
        bounds(&test, &first_tab),
        Bounds::new(9, 7, 215, 35),
        "215 x 35 at (9, 7)"
    );
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
    let tabs = test.window.tab_strip().tab_list();
    let last_tab = tabs.last_child().expect("two tabs");
    let tab = bounds(&test, &last_tab);
    assert_eq!(tab.x, 9 + 215 + 2, "tabs are 2 pixels apart");
    let plus = bounds(&test, &button_for(&test, "win.new-tab"));
    assert_eq!(plus.x, tab.right() + 5, "5 pixels after the last tab");
    assert_eq!((plus.y, plus.width, plus.height), (8, 38, 34));
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
        let place = bounds(&test, caption);
        assert_eq!(
            (place.y, place.width, place.height),
            (0, 46, 42),
            "{:?}",
            caption.tooltip_text()
        );
    }
    let close = captions.iter().find(|button| button.has_css_class("close"));
    if let Some(close) = close {
        let place = bounds(&test, close);
        assert_eq!(place.right(), window_width, "Close sits in the corner");
    }
}

#[gtk::test]
fn the_address_and_search_boxes_are_34_pixels_tall_14_pixels_below_the_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let address = bounds(&test, test.window.address_bar());
    assert_eq!((address.x, address.y, address.height), (178, 56, 34));
    let search = bounds(&test, test.window.search_box());
    assert_eq!((search.y, search.width, search.height), (56, 235, 34));
    assert_eq!(
        search.right(),
        test.window.width() - 16,
        "the search box ends 16 pixels from the edge"
    );
}

#[gtk::test]
fn the_search_box_names_the_folder_it_searches() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri_of("Documents"));
    let placeholder = test.window.search_box().entry().placeholder_text();
    assert_eq!(placeholder.as_deref(), Some("Search Documents"));
}

#[gtk::test]
fn the_open_windows_menu_lists_every_window_then_new_window_and_quit() {
    let fixture = Fixture::standard();
    let first = laid_out(&fixture.uri());
    let second = first.open_beside(&fixture.uri_of("Documents"));
    let button = menu_button_with_class(&first, "windows-button");
    button.popup();
    wait_for_frames(&first.window, 2);
    let menu = app_menu(&button);
    let labels = menu.row_labels();
    let checked = menu.checked_labels();
    button.popdown();
    let title_of = |test: &TestWindow| test.window.title().map(|title| title.to_string());
    let first_title = title_of(&first).expect("a titled window");
    let second_title = title_of(&second).expect("a titled window");
    assert!(
        labels.contains(&first_title) && labels.contains(&second_title),
        "{labels:?}"
    );
    assert_eq!(
        &labels[labels.len() - 3..],
        ["-", "New window", "Quit OpenXplorer"]
    );
    assert_eq!(checked, [first_title], "this window is checked");
    second.window.close();
}

#[gtk::test]
fn a_disabled_menu_item_names_the_milestone_that_brings_it() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more_button = menu_button_with_class(&test, "more-command");
    let more = app_menu(&more_button);
    more_button.popup();
    let license = more.row("License & source");
    let new_menu = app_menu(&menu_button_with_class(&test, "new-command"));
    let folder = new_menu
        .rows()
        .into_iter()
        .next()
        .expect("New lists Folder first");
    assert!(!license.is_sensitive());
    assert_eq!(
        license.tooltip_text().unwrap_or_default().as_str(),
        "License & source\nNot in the native preview yet: arrives with packaging and updates."
    );
    assert_eq!(
        folder.tooltip_text().unwrap_or_default().as_str(),
        "Folder",
        "a ported command names no milestone"
    );
    more_button.popdown();
}

/// A new window on a landing page has no file list to focus; GTK's first
/// focusable widget was a crumb, which drew the address bar's editing
/// line.
#[gtk::test]
fn a_new_window_on_a_landing_page_leaves_the_address_bar_alone() {
    let test = laid_out(Page::ThisPc.uri());
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
    let address = test.window.address_bar();
    let in_address = focus.is_some_and(|widget| widget.is_ancestor(address));
    assert!(!in_address, "the address bar does not hold focus");
}

#[gtk::test]
fn a_new_window_types_into_its_file_list() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focused widget");
    let view = test.window.folder_pane().view_widget();
    assert!(
        focus.is_ancestor(&view) || focus == view,
        "focus is in the file list, not {focus:?}"
    );
}

/// Every window action a control of the frame names, its menus included,
/// without the `win.` prefix.
fn window_actions_named(test: &TestWindow) -> Vec<String> {
    let widgets = descendants::<gtk::Widget>(&test.window);
    let controls = widgets
        .iter()
        .filter_map(|widget| widget.dynamic_cast_ref::<gtk::Actionable>());
    let detailed_names = controls.filter_map(ActionableExt::action_name);
    detailed_names
        .filter_map(|name| name.strip_prefix("win.").map(str::to_owned))
        .collect()
}

#[gtk::test]
fn every_control_of_the_frame_runs_an_action_the_window_has() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let named = window_actions_named(&test);
    let from_templates = [
        "new-tab",
        "location",
        "details-pane",
        "view",
        "check-updates",
        "settings",
    ];
    for action in from_templates {
        assert!(
            named.iter().any(|name| name == action),
            "a control runs win.{action}"
        );
    }
    let missing: Vec<&String> = named
        .iter()
        .filter(|name| test.window.lookup_action(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "controls name actions the window lacks: {missing:?}"
    );
}

/// Every image the frame can show is a bundled icon, GTK's own included:
/// the search box's clear button shows the bundled close glyph, not the
/// desktop theme's. Only the loading spinner, which is not an image, comes
/// from the theme (see `empty_page.rs`).
///
/// parity: LOOK-015
#[gtk::test]
fn every_image_of_the_frame_shows_a_bundled_icon() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let images: Vec<gtk::Image> = descendants::<gtk::Image>(&test.window)
        .into_iter()
        .filter(|image| image.is_visible() && image.storage_type() != gtk::ImageType::Empty)
        .collect();
    let glyph_count = images.iter().filter(|image| image.has_css_class("glyph")).count();
    assert!(glyph_count > 0, "the frame shows glyphs");
    let theme = gtk::IconTheme::for_display(&WidgetExt::display(&test.window));
    let missing: Vec<String> = images
        .iter()
        .map(|image| image.icon_name().map(String::from).unwrap_or_default())
        .filter(|name| !(name.starts_with("ox-") && theme.has_icon(name)))
        .collect();
    assert!(missing.is_empty(), "images without a bundled icon: {missing:?}");
    let clear = test
        .window
        .search_box()
        .entry()
        .last_child()
        .and_downcast::<gtk::Image>();
    let clear_icon = clear.and_then(|image| image.icon_name());
    assert_eq!(clear_icon.as_deref(), Some("ox-dismiss-16-symbolic"));
}
