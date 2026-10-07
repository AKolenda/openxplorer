// SPDX-License-Identifier: AGPL-3.0-only
//! "Hide expand arrows" in a real window (SIDE-032): the navigation pane's
//! arrows draw nothing until keyboard focus is in the pane (or the pointer
//! over it), the file list keeps its own, and the window takes up the
//! choice at once, also when it is flipped in the Settings tab.

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

/// Whether `widget` draws anything: an arrow the skin hides draws
/// nothing, as GTK skips a widget whose CSS opacity is 0.
fn draws(widget: &gtk::Widget) -> bool {
    let picture = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    picture.snapshot(&snapshot, f64::from(widget.width()), f64::from(widget.height()));
    snapshot.to_node().is_some()
}

/// The navigation pane's arrows: the chevrons of This PC and Network and
/// the folder tree's.
fn navigation_arrows(test: &TestWindow) -> Vec<gtk::Widget> {
    descendants::<gtk::Widget>(test.window.sidebar())
        .into_iter()
        .filter(|widget| {
            let chevron = widget.has_css_class("side-expander");
            let tree_arrow = widget.css_name() == "expander"
                && widget
                    .parent()
                    .is_some_and(|parent| parent.css_name() == "treeexpander")
                && inside_class(widget, "folder-tree");
            chevron || tree_arrow
        })
        .filter(WidgetExt::is_mapped)
        .collect()
}

/// The file list's folder arrows.
fn file_list_arrows(test: &TestWindow) -> Vec<gtk::Widget> {
    let view = test.window.folder_pane().details().column_view();
    descendants::<gtk::Widget>(view)
        .into_iter()
        .filter(|widget| widget.has_css_class("folder-expander") && widget.is_mapped())
        .collect()
}

/// With the setting on, the navigation pane's arrows (This PC's and
/// Network's chevrons, the folder tree's) draw nothing while keyboard
/// focus is elsewhere and show once it is in the pane, as the pointer
/// over the pane shows them; the file list's folder arrows stay, as the
/// Expandable folders switch decides those. Off, every arrow shows.
///
/// parity: SIDE-032
#[gtk::test]
fn hidden_navigation_arrows_show_with_focus_in_the_pane() {
    let fixture = Fixture::standard();
    let test = window_with_every_arrow(&fixture);
    assert!(!test.window.hides_expand_arrows(), "shown by default");
    let arrows = navigation_arrows(&test);
    let chevrons = arrows
        .iter()
        .filter(|widget| widget.has_css_class("side-expander"))
        .count();
    assert_eq!(chevrons, 2, "This PC and Network");
    assert!(arrows.len() > chevrons, "the folder tree draws arrows");
    assert!(arrows.iter().all(draws), "shown by default");

    test.window.folder_pane().grab_focus();
    save_arrows_hidden(&test, true);
    wait_until("the navigation pane's arrows to hide", || {
        test.window.hides_expand_arrows() && !navigation_arrows(&test).iter().any(draws)
    });
    let list_arrows = file_list_arrows(&test);
    assert!(!list_arrows.is_empty(), "the file list draws folder arrows");
    assert!(list_arrows.iter().all(draws), "and keeps them");

    let row = test.window.sidebar().list().row_at_index(0).expect("a first row");
    row.grab_focus();
    wait_until("focus in the pane to show its arrows", || {
        navigation_arrows(&test).iter().all(draws)
    });
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
