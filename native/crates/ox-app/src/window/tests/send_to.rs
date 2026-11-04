// SPDX-License-Identifier: AGPL-3.0-only
//! Send to in a real window (CMD-038): the classic menu's submenu from the
//! keyboard, a copy to Documents through the transfer engine with Undo,
//! and Desktop (create shortcut). The home folder, its Desktop and
//! Documents lie in a temporary directory.

use std::cell::Cell;
use std::fs;
use std::rc::Rc;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::file_uri;
use ox_core::network::NetworkError;
use ox_core::ops::JournalDirection;
use ox_core::places::{FolderLocations, KnownFolder};
use ox_core::settings::ContextMenu;

use super::context_menus::{choose_menu_style, position_of};
use super::file_ops_support::{require_private_trash, select_names};
use super::network::message_box_texts;
use crate::test_support::harness::{wait_until, TestWindow};
use crate::window::menu_popover::MenuStyle;

/// A home with a Desktop, a Documents folder and a Work folder holding
/// `report.txt`.
struct Folders {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl Folders {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room for folders");
        let base = fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        for folder in ["Desktop", "Documents", "Work"] {
            fs::create_dir_all(home.join(folder)).expect("a fixture folder");
        }
        fs::create_dir_all(&config).expect("the config folder");
        fs::write(home.join("Work/report.txt"), b"Synthetic quarterly figures").expect("a file");
        Self {
            _root: root,
            home,
            config,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.home.join(relative)
    }

    /// A window on the Work folder with the classic menu open on
    /// report.txt, as Shift+F10 opens it, once the standard folders are
    /// read.
    fn open_menu_on_report(&self) -> TestWindow {
        let locations = FolderLocations::new(self.home.clone(), &self.config);
        let test = TestWindow::open_with_standard_folders(&file_uri(&self.path("Work")), locations, |_| {});
        choose_menu_style(&test, ContextMenu::Win10);
        wait_until("the standard folders", || {
            test.context
                .known_folders()
                .iter()
                .any(|place| place.known_folder == Some(KnownFolder::Documents))
        });
        select_names(&test, &["report.txt"]);
        test.window.right_click(Some(position_of(&test, "report.txt")));
        test
    }
}

/// Waits until the operation that changed the Undo label from `before`
/// has ended, so the next one may start.
fn wait_for_operation_after(test: &TestWindow, before: &str) {
    wait_until("the operation to be recorded and end", || {
        test.window.journal_label(JournalDirection::Undo) != before
            && test.window.imp().file_operations.borrow().is_idle()
    });
}

/// Send to is a named, focusable row of the classic menu; Right opens it
/// in place, listing Windows' destinations this computer has, and
/// Documents copies the file there through the transfer engine, keeping
/// it selected, which Undo reverses.
///
/// parity: CMD-038, OPS-029
#[gtk::test]
fn send_to_opens_from_the_keyboard_and_copies_to_documents_with_undo() {
    require_private_trash();
    let folders = Folders::new();
    let test = folders.open_menu_on_report();
    let menu = test.window.context_menu();
    assert_eq!(menu.style(), MenuStyle::Classic);
    let rows = menu.row_labels();
    let send_to = rows
        .iter()
        .position(|label| label == "Send to")
        .expect("Send to is listed");
    assert_eq!(rows[send_to + 1], "-");
    assert_eq!(rows[send_to + 2], "Cut", "Send to sits above Cut");

    let row = menu.row("Send to");
    assert!(row.is_sensitive());
    assert!(
        row.is_activatable() && row.is_focusable(),
        "the keyboard reaches it"
    );
    assert!(gtk::test_accessible_has_property(
        &row,
        gtk::AccessibleProperty::Label
    ));
    row.grab_focus();
    assert!(menu.press_in_list(gtk::gdk::Key::Right));
    wait_until("the Send to submenu", || {
        menu.row_labels()
            .contains(&"Compressed (zipped) folder".to_owned())
    });
    let entries: Vec<String> = menu
        .row_labels()
        .into_iter()
        .filter(|label| label != "-" && label != "Send to")
        .collect();
    // No Bluetooth adapter, Email portal or removable drive in the tests.
    assert_eq!(
        entries,
        [
            "Compressed (zipped) folder",
            "Desktop (create shortcut)",
            "Documents"
        ]
    );
    let documents = menu.row("Documents");
    assert!(documents.is_focusable());
    assert!(gtk::test_accessible_has_property(
        &documents,
        gtk::AccessibleProperty::Label
    ));
    let before = test.window.journal_label(JournalDirection::Undo);
    documents.emit_activate();

    let copy = folders.path("Documents/report.txt");
    wait_until("the copy in Documents", || copy.is_file());
    assert_eq!(fs::read(&copy).expect("the copy"), b"Synthetic quarterly figures");
    assert!(folders.path("Work/report.txt").is_file(), "the original stays");
    wait_for_operation_after(&test, &before);
    wait_until("report.txt to stay selected, as in Windows", || {
        test.selected_names() == ["report.txt"]
    });
    test.activate("undo", None);
    wait_until("Undo to remove the copy", || !copy.exists());
    assert!(folders.path("Work/report.txt").is_file());
}

/// Desktop (create shortcut) puts a link named as Windows names it on the
/// Desktop, numbers the next one, and never touches the file itself.
///
/// parity: CMD-038
#[gtk::test]
fn desktop_create_shortcut_makes_windows_named_links() {
    let folders = Folders::new();
    let test = folders.open_menu_on_report();
    let original = folders.path("Work/report.txt");

    let before = test.window.journal_label(JournalDirection::Undo);
    test.activate("send-to-desktop", None);
    let first = folders.path("Desktop/report.txt - Shortcut");
    wait_until("the shortcut", || first.symlink_metadata().is_ok());
    // One operation runs at a time: the next starts once this one ends.
    wait_for_operation_after(&test, &before);
    test.window.context_menu().popdown();
    test.activate("send-to-desktop", None);
    let second = folders.path("Desktop/report.txt - Shortcut (2)");
    wait_until("the second shortcut", || second.symlink_metadata().is_ok());

    assert_eq!(fs::read_link(&first).expect("a link"), original);
    assert_eq!(fs::read_link(&second).expect("a link"), original);
    assert_eq!(
        fs::read(&original).expect("the file"),
        b"Synthetic quarterly figures"
    );
}

/// A saved network location that is not connected is enabled, as in
/// Windows: choosing it connects through the window's usual mount, with
/// its sign-in, and copies only once that succeeds. The share is fictional
/// and on a host that does not exist, and the tests answer the mount.
///
/// parity: CMD-038, NET-004
#[gtk::test]
fn a_saved_share_that_is_not_connected_connects_first_then_copies() {
    let folders = Folders::new();
    let test = folders.open_menu_on_report();
    test.window.context_menu().popdown();
    test.save_share("smb://studio-nas.invalid/media", "Media (M:)");
    wait_until("the saved share", || {
        test.window
            .places()
            .saved_shares
            .iter()
            .any(|share| share.bookmark.label == "Media (M:)" && !share.is_connected)
    });
    let mounts = Rc::new(Cell::new(0));
    let counted = Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Err(NetworkError::NotAFolder)
    });
    let choose_share = || {
        test.window.right_click(Some(position_of(&test, "report.txt")));
        let menu = test.window.context_menu();
        assert!(menu.open_submenu_labelled("Send to"));
        let share = menu.row("Media (M:)");
        assert!(share.is_sensitive(), "enabled although not connected");
        share.emit_activate();
    };

    // A failed connection is reported, and nothing is copied.
    choose_share();
    wait_until("the failed connection", || {
        mounts.get() == 1 && test.window.shown_message_text() == "This location is not a folder."
    });

    // Once connected, the copy starts: here the fictional host cannot be
    // reached, which the copy's destination check reports.
    let counted = Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Ok(())
    });
    choose_share();
    let texts = message_box_texts("Could not check destination");
    assert_eq!(mounts.get(), 2, "connected once more, then copied");
    assert!(!texts.is_empty());
}
