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

/// The list of the folder tree's rows.
fn tree_list(test: &TestWindow) -> gtk::ListView {
    let tree: gtk::Widget = test.window.folder_tree().clone().upcast();
    let mut pending = vec![tree];
    while let Some(widget) = pending.pop() {
        if let Ok(list) = widget.clone().downcast::<gtk::ListView>() {
            return list;
        }
        pending.extend(std::iter::successors(
            widget.first_child(),
            gtk::Widget::next_sibling,
        ));
    }
    panic!("the folder tree has a list of rows");
}

/// The middle of the folder tree's row named `name`, in the list's
/// coordinates.
fn tree_row_middle(list: &gtk::ListView, name: &str) -> (f64, f64) {
    let mut pending: Vec<gtk::Widget> = vec![list.clone().upcast()];
    while let Some(widget) = pending.pop() {
        let is_name = widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == name && label.is_mapped());
        if is_name {
            let bounds = widget.compute_bounds(list).expect("a shown row has bounds");
            return (
                f64::from(bounds.x() + bounds.width() / 2.0),
                f64::from(bounds.y() + bounds.height() / 2.0),
            );
        }
        pending.extend(std::iter::successors(
            widget.first_child(),
            gtk::Widget::next_sibling,
        ));
    }
    panic!("the folder tree shows {name}");
}

/// A real drag of a file onto a folder of the folder tree moves it into
/// that folder, as a drop on a folder of Windows Explorer's navigation
/// pane does within a drive; the tree used to take no drops at all.
///
/// parity: DND-011, SIDE-028
#[gtk::test]
fn a_real_drag_onto_a_folder_of_the_folder_tree_goes_into_it() {
    use super::right_drag::{item_on_screen, HOLD, PROBE};
    use crate::test_support::pointer::{screen_point, Button, Pointer};

    let fixture = Fixture::standard();
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    fs::write(reports.join("Draft.txt"), "draft").expect("fixture file");
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&file_uri(&reports));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    let shown = file_uri(&reports);
    wait_until("the tree selects the folder shown", || {
        tree.selected_uri().is_some_and(|uri| same_location(&uri, &shown))
    });
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let list = tree_list(&test);

    let from = item_on_screen(&mut pointer, &test, "Draft.txt");
    let onto = screen_point(
        &mut pointer,
        &list,
        tree_row_middle(&list, "Example projects"),
        PROBE,
    );
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    crate::test_support::harness::wait_for(HOLD);
    pointer.release(Button::Primary);

    wait_until("the move into the tree's folder", || {
        fixture.path("Draft.txt").is_file()
    });
    assert!(
        !reports.join("Draft.txt").exists(),
        "a plain drag within a drive moves"
    );
}
