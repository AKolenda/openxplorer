// SPDX-License-Identifier: AGPL-3.0-only
//! "Hide expand arrows" in a real window (SIDE-032): the three kinds of
//! arrow the skin hides are where it looks for them, and the window takes
//! up the choice at once, also when it is flipped in the Settings tab.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{file_uri, same_location};
use ox_core::settings::{PreferencesUpdate, Settings};

use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// Saves `hidden` as Settings or another window would, and lets the
/// window read it.
fn save_arrows_hidden(test: &TestWindow, hidden: bool) {
    let update = PreferencesUpdate {
        hide_expand_arrows: Some(hidden),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
}

/// Whether `widget` sits inside a widget with the CSS class `class`.
fn inside_class(widget: &gtk::Widget, class: &str) -> bool {
    let mut parent = widget.parent();
    while let Some(widget) = parent {
        if widget.has_css_class(class) {
            return true;
        }
        parent = widget.parent();
    }
    false
}

/// A window on Documents/Reports with the folder tree shown, so every
/// kind of arrow is on screen.
fn window_with_every_arrow(fixture: &Fixture) -> TestWindow {
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    fs::create_dir(reports.join("2026")).expect("a folder to expand");
    let test = TestWindow::open(&file_uri(&reports));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let reports = file_uri(&reports);
    wait_until("the tree selects the folder shown", || {
        test.window
            .folder_tree()
            .selected_uri()
            .is_some_and(|uri| same_location(&uri, &reports))
    });
    wait_for_frames(&test.window, 3);
    test
}

/// The arrows the skin hides are where its selectors look: the chevrons
/// of This PC and Network (`.sidebar .expand`), the folder tree's arrows
/// (`.folder-tree treeexpander expander`) and the file list's folder
/// arrows (`columnview.files .folder-expander`). Shown by default.
///
/// parity: SIDE-032
#[gtk::test]
fn every_kind_of_expand_arrow_is_where_the_skin_hides_it() {
    let fixture = Fixture::standard();
    let test = window_with_every_arrow(&fixture);
    assert!(!test.window.hides_expand_arrows(), "shown by default");
    let widgets = descendants::<gtk::Widget>(&test.window);

    let chevrons = widgets
        .iter()
        .filter(|widget| widget.has_css_class("expand") && inside_class(widget, "sidebar"))
        .count();
    assert_eq!(chevrons, 2, "This PC and Network");

    let tree_arrows = widgets
        .iter()
        .filter(|widget| {
            widget.css_name() == "expander"
                && widget
                    .parent()
                    .is_some_and(|parent| parent.css_name() == "treeexpander")
                && inside_class(widget, "folder-tree")
        })
        .count();
    assert!(tree_arrows > 0, "the folder tree draws arrows");

    let view = test.window.folder_pane().details().column_view();
    let list_arrows = descendants::<gtk::Widget>(view)
        .into_iter()
        .filter(|widget| widget.has_css_class("folder-expander") && widget.is_visible())
        .count();
    assert!(list_arrows > 0, "the file list draws folder arrows");
}

/// Flipping the setting in the Settings tab and going back to the folder
/// tab, again and again, hides and shows the arrows each time, the tab
/// still on its folder.
///
/// parity: SIDE-032
#[gtk::test]
fn flipping_the_setting_from_the_settings_tab_hides_and_shows_the_arrows() {
    let fixture = Fixture::standard();
    let test = window_with_every_arrow(&fixture);
    let folder = test.window.current_uri();

    for hidden in [true, false, true, false] {
        test.activate("settings", None);
        wait_for_frames(&test.window, 2);
        save_arrows_hidden(&test, hidden);
        test.activate("next-tab", None);
        wait_for_frames(&test.window, 2);
        assert_eq!(test.window.current_uri(), folder, "back on the same folder");
        assert_eq!(
            test.window.hides_expand_arrows(),
            hidden,
            "after turning the setting {}",
            if hidden { "on" } else { "off" }
        );
    }
}
