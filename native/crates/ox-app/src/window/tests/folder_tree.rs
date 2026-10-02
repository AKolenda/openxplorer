// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree in the navigation pane (SIDE-028).

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{file_uri, same_location};

use crate::test_support::harness::{wait_until, Fixture, TestWindow};

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
