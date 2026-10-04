// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree in the navigation pane (SIDE-028).

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{file_uri, same_location};
use ox_core::settings::{PreferencesUpdate, Settings};

use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// parity: SIDE-028
#[gtk::test]
fn the_folder_tree_opens_down_to_the_folder_shown_and_opens_the_folder_clicked() {
    let fixture = Fixture::standard();
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    fs::create_dir(fixture.path(".cache")).expect("hidden fixture folder");
    let test = TestWindow::open(&file_uri(&reports));
    // The fixture's temporary folder plays the home folder, so the tree
    // starts there and never lists the real disk.
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);

    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    let reports = file_uri(&reports);
    wait_until("the tree selects the folder shown", || {
        tree.selected_uri()
            .is_some_and(|uri| same_location(&uri, &reports))
    });

    assert!(tree.is_visible());
    assert_eq!(
        tree.outline(),
        ["Home", "  Example projects", "    Documents", "      Reports"],
        "only folders, no hidden one, each opened only on the way"
    );
    tree.activate_row(1);
    wait_until("the clicked folder opens", || {
        test.window
            .current_uri()
            .is_some_and(|uri| same_location(&uri, &fixture.uri()))
    });
}

/// Whether every row of the tree on screen shows its expand arrow
/// (`Some(true)`), none does (`Some(false)`), or the rows disagree.
fn arrows_shown(tree: &crate::window::folder_tree::FolderTree) -> Option<bool> {
    let expanders: Vec<gtk::TreeExpander> = descendants::<gtk::TreeExpander>(tree)
        .into_iter()
        .filter(|expander| expander.list_row().is_some())
        .collect();
    assert!(!expanders.is_empty(), "the tree has rows");
    // GTK draws an arrow (CSS name `expander`) only on rows that have
    // subfolders, so the property is what every row must agree on; the
    // drawn arrows must follow it.
    let shown = |expander: &gtk::TreeExpander| !expander.hides_expander();
    let drawn = descendants::<gtk::Widget>(tree)
        .into_iter()
        .filter(|widget| widget.css_name() == "expander" && widget.is_visible())
        .count();
    if expanders.iter().any(shown) != (drawn > 0) {
        return None;
    }
    if expanders.iter().all(shown) {
        Some(true)
    } else if expanders.iter().any(shown) {
        None
    } else {
        Some(false)
    }
}

/// Saves `hidden` as Settings or another window would, and lets the
/// window read it.
fn save_arrows_hidden(test: &TestWindow, hidden: bool) {
    let update = PreferencesUpdate {
        hide_folder_tree_arrows: Some(hidden),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
}

/// The tree's rows show their expand arrows by default; hiding them in
/// Settings takes them off every row at once, rows the tree builds later
/// too, and showing them brings them back.
///
/// parity: SIDE-032
#[gtk::test]
fn the_folder_trees_arrows_can_be_hidden_and_shown_again() {
    let fixture = Fixture::standard();
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    let test = TestWindow::open(&file_uri(&reports));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    let reports = file_uri(&reports);
    wait_until("the tree selects the folder shown", || {
        tree.selected_uri()
            .is_some_and(|uri| same_location(&uri, &reports))
    });
    assert_eq!(arrows_shown(tree), Some(true), "shown by default");

    save_arrows_hidden(&test, true);
    wait_until("the arrows to go", || arrows_shown(tree) == Some(false));
    assert!(tree.arrows_are_hidden());

    // Rows the tree builds afterwards come without arrows too.
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    wait_for_frames(&test.window, 3);
    assert_eq!(arrows_shown(tree), Some(false));

    save_arrows_hidden(&test, false);
    wait_until("the arrows to come back", || arrows_shown(tree) == Some(true));
}

/// Flipping the setting in the Settings tab and going back to the folder
/// tab, again and again, takes the drawn arrows off and brings them back
/// each time, without the tab showing another folder.
///
/// parity: SIDE-032
#[gtk::test]
fn flipping_the_setting_from_the_settings_tab_hides_and_brings_back_the_arrows() {
    let fixture = Fixture::standard();
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    let test = TestWindow::open(&file_uri(&reports));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    let reports = file_uri(&reports);
    wait_until("the tree selects the folder shown", || {
        tree.selected_uri()
            .is_some_and(|uri| same_location(&uri, &reports))
    });
    assert_eq!(arrows_shown(tree), Some(true));
    let folder = test.window.current_uri();

    for hidden in [true, false, true, false] {
        test.activate("settings", None);
        wait_for_frames(&test.window, 2);
        save_arrows_hidden(&test, hidden);
        wait_for_frames(&test.window, 2);
        test.activate("next-tab", None);
        wait_for_frames(&test.window, 3);
        assert_eq!(test.window.current_uri(), folder, "back on the same folder");
        assert_eq!(
            arrows_shown(tree),
            Some(!hidden),
            "after turning the setting {}",
            if hidden { "on" } else { "off" }
        );
    }
}
