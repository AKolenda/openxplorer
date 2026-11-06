// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties window keeps Windows 11's width for every item: a long
//! name, location or value wraps or ends in "…" instead of widening it.

use std::fs;

use gtk::prelude::*;

use super::item_dialogs::general;
use crate::properties::PropertiesWindow;
use crate::test_support::harness::{
    capture_dialog, descendants, wait_for_frames, Fixture, TestWindow, ThemeGuard,
};

/// A fictional name far longer than the window is wide.
const LONG_NAME: &str =
    "Discover Effective Calming Techniques_ Deep Breathing and Quiet Evenings – Example.com_files";

/// Every tab a Properties window may have, by page name.
const TABS: [&str; 8] = [
    "general",
    "hardware",
    "sharing",
    "security",
    "location",
    "details",
    "versions",
    "customize",
];

/// The window's width once its General tab is filled and drawn.
fn shown_width(window: &PropertiesWindow) -> i32 {
    general(window);
    wait_for_frames(window, 4);
    window.width()
}

/// The shown widgets of `window` wider than it, for the failure message.
fn spilling_widgets(window: &PropertiesWindow) -> Vec<String> {
    let width = window.width();
    descendants::<gtk::Widget>(window)
        .into_iter()
        .filter(|widget| widget.is_mapped() && widget.width() > width)
        .map(|widget| {
            format!(
                "{} {:?} {}",
                widget.type_().name(),
                widget.css_classes(),
                widget.width()
            )
        })
        .collect()
}

/// Shows every tab `window` has and checks that each keeps `width` and
/// that nothing on it is wider than the window.
fn assert_every_tab_keeps(window: &PropertiesWindow, width: i32, what: &str) {
    for name in TABS {
        if window.tab_page(name).is_none() {
            continue;
        }
        window.select_tab(name);
        wait_for_frames(window, 4);
        assert_eq!(
            window.width(),
            width,
            "{what}: the {name} tab keeps the window's width"
        );
        let spilling = spilling_widgets(window);
        assert!(
            spilling.is_empty(),
            "{what}: on the {name} tab, wider than the window: {spilling:?}"
        );
    }
}

/// Opens Properties of the selection in `test`, titled `title`.
fn properties_of_selection(test: &TestWindow, title: &str) -> PropertiesWindow {
    test.activate("properties", None);
    test.wait_for_properties(title)
}

/// A long name, a long location and several items with long names give a
/// window as wide as a short name does, on every tab: Windows' fixed
/// width. The name box keeps the width and scrolls its text; the title
/// ends in "…".
///
/// parity: PROP-001
#[gtk::test]
fn a_long_name_or_location_never_widens_properties() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    fs::create_dir(fixture.path(LONG_NAME)).expect("a folder");
    fs::write(fixture.path(&format!("{LONG_NAME}.txt")), "fictional").expect("a file");
    let deep = [LONG_NAME, LONG_NAME, LONG_NAME].join("/");
    fs::create_dir_all(fixture.path(&format!("Deep/{deep}"))).expect("a deep folder");
    let test = TestWindow::open(&fixture.uri());

    test.select_named("Short");
    let short = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&short);
    let height = short.height();
    assert!(width > 0);
    let short_name = descendants::<gtk::Entry>(&short)
        .into_iter()
        .next()
        .expect("the name box");
    let name_width = short_name.width();
    assert_every_tab_keeps(&short, width, "a short name");
    short.close();

    test.select_named(LONG_NAME);
    let folder = properties_of_selection(&test, &format!("{LONG_NAME} Properties"));
    assert_eq!(shown_width(&folder), width, "a long folder name");
    assert_eq!(folder.height(), height, "nor makes it taller");
    let long_name = descendants::<gtk::Entry>(&folder)
        .into_iter()
        .next()
        .expect("the name box");
    assert_eq!(long_name.width(), name_width, "the name box keeps its width");
    assert_eq!(long_name.text(), LONG_NAME, "and holds the whole name");
    assert_every_tab_keeps(&folder, width, "a long folder name");
    folder.close();

    let file_name = format!("{LONG_NAME}.txt");
    test.select_named(&file_name);
    let file = properties_of_selection(&test, &format!("{file_name} Properties"));
    assert_eq!(shown_width(&file), width, "a long file name");
    assert_every_tab_keeps(&file, width, "a long file name");
    file.close();

    test.show(&fixture.uri_of(&format!("Deep/{}", [LONG_NAME, LONG_NAME].join("/"))));
    test.wait_for_listing("the deep folder");
    test.select_named(LONG_NAME);
    let nested = properties_of_selection(&test, &format!("{LONG_NAME} Properties"));
    assert_eq!(shown_width(&nested), width, "a long location");
    assert_eq!(nested.height(), height, "a long location keeps the height too");
    assert_every_tab_keeps(&nested, width, "a long location");
    nested.close();

    test.show(&fixture.uri());
    test.wait_for_listing("the fixture");
    super::file_ops_support::select_names(&test, &[LONG_NAME, &file_name]);
    // Windows titles them by their two names, in the listing's order.
    let is_both = |title: &str| {
        title == format!("{LONG_NAME}, {file_name} Properties")
            || title == format!("{file_name}, {LONG_NAME} Properties")
    };
    test.activate("properties", None);
    crate::test_support::harness::wait_until("Properties of the two items", || {
        test.window
            .properties_windows()
            .iter()
            .any(|window| is_both(&window.title_text()))
    });
    let several = test
        .window
        .properties_windows()
        .into_iter()
        .find(|window| is_both(&window.title_text()))
        .expect("Properties of the two items");
    wait_for_frames(&several, 6);
    assert_eq!(several.width(), width, "several items with long names");
    assert_every_tab_keeps(&several, width, "several items");
    several.close();
}

/// General with a short and a very long fictional name, light and dark,
/// for review with `OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`):
/// `properties-width-{short,long}-{light,dark}.png`.
///
/// parity: PROP-001
#[gtk::test]
fn properties_with_a_long_name_are_captured() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Holiday 2025")).expect("a folder");
    fs::create_dir(fixture.path(LONG_NAME)).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        for (name, label) in [("Holiday 2025", "short"), (LONG_NAME, "long")] {
            test.select_named(name);
            let window = properties_of_selection(&test, &format!("{name} Properties"));
            shown_width(&window);
            capture_dialog(&window, &format!("properties-width-{label}-{theme}.png"));
            window.close();
        }
    }
}
