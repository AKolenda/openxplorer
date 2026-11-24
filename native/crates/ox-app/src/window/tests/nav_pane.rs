// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane against Windows 11's: where a pin from a menu goes,
//! what a drag of files does over the pinned folders, and the other
//! fixes of the navigation-pane pass.

use std::fs;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use ox_core::location::file_uri;
use ox_core::places::FolderLocations;

use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};

/// A home folder with the standard folders Quick access lists, and a
/// project folder to pin, in a temporary directory.
struct PinHome {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl PinHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room");
        let base = fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        for folder in [
            "Desktop",
            "Documents",
            "Downloads",
            "Music",
            "Pictures",
            "Videos",
            "Projects",
        ] {
            fs::create_dir_all(home.join(folder)).expect("a folder");
        }
        fs::create_dir_all(&config).expect("a folder");
        Self {
            _root: root,
            home,
            config,
        }
    }

    fn locations(&self) -> FolderLocations {
        FolderLocations::new(self.home.clone(), &self.config)
    }

    fn uri_of(&self, name: &str) -> String {
        file_uri(&self.home.join(name))
    }
}

/// The labels of the Quick access rows, in order.
fn pinned_labels(test: &TestWindow) -> Vec<String> {
    test.window
        .places()
        .quick_access
        .iter()
        .map(|place| place.label.clone())
        .collect()
}

/// As in Windows 11, a folder pinned from its menu or the command bar goes
/// at the end of the pinned folders, and a standard folder unpinned and
/// pinned again goes there too. It used to go first, above Desktop, since
/// the pin was saved without the order the pane showed.
///
/// parity: SIDE-005, SIDE-007
#[gtk::test]
fn a_folder_pinned_from_a_menu_goes_at_the_end_of_the_pins() {
    let home = PinHome::new();
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), home.locations(), |_| {});
    test.wait_for_listing("the home folder");
    let before = pinned_labels(&test);

    test.activate("pin-location", Some(&home.uri_of("Projects")));
    wait_until("the pin", || {
        pinned_labels(&test).contains(&"Projects".to_owned())
    });
    let after_pin = pinned_labels(&test);
    test.window.unpin(&home.uri_of("Documents"));
    wait_until("the unpin", || {
        !pinned_labels(&test).contains(&"Documents".to_owned())
    });
    test.activate("pin-location", Some(&home.uri_of("Documents")));
    wait_until("the pin again", || {
        pinned_labels(&test).contains(&"Documents".to_owned())
    });
    let after_repin = pinned_labels(&test);

    assert!(before.len() >= 6, "the standard folders are pinned: {before:?}");
    assert_eq!(
        after_pin.last().map(String::as_str),
        Some("Projects"),
        "{after_pin:?}"
    );
    assert_eq!(
        after_pin[..before.len()],
        before[..],
        "the others keep their order"
    );
    assert_eq!(
        after_repin.last().map(String::as_str),
        Some("Documents"),
        "{after_repin:?}"
    );
}

/// A real drag of a file over the top edge of a pinned folder, where a
/// folder would be pinned, goes into that folder: a file cannot be
/// pinned, and the drop used to show the pin line and then refuse with
/// "Could not pin", leaving the file where it was.
///
/// parity: DND-011, DND-014
#[gtk::test]
fn a_real_drag_of_a_file_onto_the_edge_of_a_pin_goes_into_the_pinned_folder() {
    use super::right_drag::{item_on_screen, HOLD, PROBE};
    use crate::test_support::pointer::{screen_point, Button, Pointer};

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Drop shelf")).expect("fixture subfolder");
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri_of("Drop shelf"));
    test.activate("pin-folder", None);
    let sidebar = test.window.sidebar();
    wait_until("the pin", || sidebar.labels().contains(&"Drop shelf".to_owned()));
    test.show(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let pins_before = pinned_labels(&test);

    let from = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    let edge = (24.0, sidebar.top_edge_of("Drop shelf") + 1.0);
    let onto = screen_point(&mut pointer, sidebar.list(), edge, PROBE);
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), onto, 12);
    wait_for(HOLD);
    let line = sidebar.drop_highlight_of("Drop shelf");
    pointer.release(Button::Primary);

    wait_until("the move into the pinned folder", || {
        fixture.path("Drop shelf/Notes 2.txt").is_file()
    });
    assert_eq!(line, Some("file-drop-active"), "the whole row is the folder");
    assert!(
        !fixture.path("Notes 2.txt").exists(),
        "a plain drag within a drive moves"
    );
    assert_eq!(pinned_labels(&test), pins_before, "nothing is pinned");
}

/// The screen point of the middle of the navigation pane's resize handle.
fn handle_on_screen(pointer: &mut crate::test_support::pointer::Pointer, test: &TestWindow) -> (i32, i32) {
    use super::right_drag::PROBE;
    let handle = crate::window::sidebar_resizer::paned_handle(test.window.workspace())
        .expect("the workspace has a handle");
    let bounds = handle.compute_bounds(&handle).expect("a shown handle has bounds");
    let middle = (f64::from(bounds.width() / 2.0), f64::from(bounds.height() / 2.0));
    crate::test_support::pointer::screen_point(pointer, &handle, middle, PROBE)
}

/// Dragging the navigation pane's edge with a real pointer saves the new
/// width, which a new window starts with, as Windows Explorer remembers
/// it. `GtkPaned` takes the press, which used to end the gesture that saves
/// it before the edge moved, so no drag was ever saved.
///
/// parity: SIDE-023
#[gtk::test]
fn a_real_drag_of_the_pane_edge_saves_its_width() {
    use super::right_drag::HOLD;
    use crate::test_support::pointer::{Button, Pointer};

    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let workspace = test.window.workspace();
    let start = workspace.position();

    let from = handle_on_screen(&mut pointer, &test);
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 60, from.1), 12);
    wait_for(HOLD);
    pointer.release(Button::Primary);
    let moved = workspace.position();

    assert!(
        moved > start + 40,
        "the edge follows the pointer: {start} to {moved}"
    );
    wait_until("the width to be saved", || {
        test.context.settings_data().preferences.sidebar_width == u32::try_from(moved).ok()
    });
}

/// A drive that cannot be ejected, because a program still uses it or
/// it is not a removable mount at all, leaves the tab on the folder it
/// showed, as Windows leaves its window there with "This device is
/// currently in use". The tab used to stay at Home, where it was moved so
/// that the window's own listing would not keep the drive busy.
///
/// parity: DEV-007, DEV-009
#[gtk::test]
fn a_drive_that_cannot_be_ejected_leaves_the_tab_where_it_was() {
    let fixture = Fixture::standard();
    let documents = fixture.uri();
    let test = TestWindow::open(&documents);

    test.window
        .remove_drive(&documents, crate::devices::Removal::Eject);
    let moved_away = test.window.current_uri();
    wait_until("the tab to be back on its folder", || {
        test.window.current_uri().as_deref() == Some(documents.as_str())
    });
    test.wait_for_listing("the folder again");

    assert_ne!(
        moved_away.as_deref(),
        Some(documents.as_str()),
        "the tab left the drive first"
    );
    assert_eq!(
        test.names(),
        ["Documents", "Notes 2.txt", "Notes 10.txt", "Résumé.txt"]
    );
}

/// As in Windows 11, where "Expand to current folder" is off at first,
/// the folder tree does not open its folders down to the folder shown;
/// once the option is on, it does.
///
/// parity: SIDE-028
#[gtk::test]
fn the_tree_expands_to_the_current_folder_only_with_the_option_on() {
    let fixture = Fixture::standard();
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir(&reports).expect("fixture subfolder");
    let test = TestWindow::open(&file_uri(&reports));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    wait_until("the tree", || !tree.outline().is_empty());
    wait_for(std::time::Duration::from_millis(500));
    let closed = tree.outline();
    let selected_closed = tree.selected_uri();

    test.activate("folder-tree-option", Some("expand-to-current"));
    let shown = file_uri(&reports);
    wait_until("the tree opens down to Reports", || {
        tree.selected_uri().is_some_and(|uri| uri == shown)
    });

    assert_eq!(closed, ["Home"], "no folder opened on the way");
    assert_eq!(selected_closed, None, "Reports is not in the tree yet");
    assert_eq!(
        tree.outline(),
        ["Home", "  Example projects", "    Documents", "      Reports"]
    );
}
