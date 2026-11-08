// SPDX-License-Identifier: AGPL-3.0-only
//! A row of Recent files stands for a file elsewhere: `GVfs` names it by a
//! hash under `recent:///` and gives the file only as its target. Opening
//! the row, Open with, Copy, Copy as path, a drag and Properties all act
//! on that file, as they do from the file's own folder, and a file that is
//! gone says "Item Not Found". Recent locations lists the folders
//! themselves, which open in place, in a new tab and in a new window.
//! Files are recorded instead of started (`AppContext::record_launches`).

use std::fs;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::integration::FOLDER_CONTENT_TYPE;
use ox_core::location::{RECENT_LOCATIONS_URI, RECENT_URI};

use crate::app_context::add_to_desktop_history;
use crate::test_support::harness::{wait_until, Fixture, OpenedWindows, TestWindow};
use crate::window::copy_path::CopiedPath;

use super::file_ops_support::{open_dialog, require_private_trash, select_names};

/// A window showing Recent files, where the file `Documents/<name>` of a
/// new fixture, just added to the desktop's history, is listed.
fn recent_files_listing(name: &str) -> (Fixture, TestWindow) {
    require_private_trash();
    let schemes = gtk::gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "recent"),
        "this test needs GVfs with its Recent backend; found {schemes:?}"
    );
    let fixture = Fixture::standard();
    let relative = format!("Documents/{name}");
    fs::write(fixture.path(&relative), b"Synthetic report\n").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    add_to_desktop_history(&fixture.uri_of(&relative), "text/plain");
    test.activate("go-to", Some(RECENT_URI));
    wait_until("Recent files", || {
        test.window.current_uri().as_deref() == Some(RECENT_URI) && !test.window.is_loading()
    });
    wait_until("the recent file", || test.names().contains(&name.to_owned()));
    (fixture, test)
}

/// The URIs a drag's content offers as a file list.
fn offered_uris(content: &gdk::ContentProvider) -> Vec<String> {
    let value = content
        .value(gdk::FileList::static_type())
        .expect("a drag offers a file list");
    let files = value.get::<gdk::FileList>().expect("the value is a file list");
    files.files().iter().map(|file| file.uri().to_string()).collect()
}

/// A text editor the Open with list offers for plain text, installed in
/// the private data folder the tests run with (native/tools/check.py)
/// until it is dropped; it runs `true`.
struct TextTool(std::path::PathBuf);

impl TextTool {
    const ID: &'static str = "io.example.RecentTextTool.desktop";

    fn install() -> Self {
        let data = gtk::glib::user_data_dir();
        assert!(
            data.starts_with(std::env::temp_dir()),
            "tests add applications only to a private data folder"
        );
        let folder = data.join("applications");
        fs::create_dir_all(&folder).expect("the data folder is writable");
        let path = folder.join(Self::ID);
        let entry = "[Desktop Entry]\nType=Application\nName=Text tool\nExec=true %U\nMimeType=text/plain;\n";
        fs::write(&path, entry).expect("the data folder is writable");
        wait_until("GIO to list the application", || {
            crate::integration::installed_application(Self::ID).is_some()
        });
        Self(path)
    }
}

impl Drop for TextTool {
    fn drop(&mut self) {
        if fs::remove_file(&self.0).is_ok() {
            wait_until("GIO to forget the application", || {
                crate::integration::installed_application(Self::ID).is_none()
            });
        }
    }
}

/// The URIs of `test`'s tabs, left to right.
fn tab_uris(test: &TestWindow) -> Vec<String> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.uri().to_owned()).collect()
}

/// The URI list the window's clipboard holds, one address per line.
fn copied_uris(test: &TestWindow) -> Option<String> {
    let clipboard = test.window.clipboard();
    let bytes = gtk::glib::MainContext::default().block_on(async {
        let (stream, _) = clipboard
            .read_future(&["text/uri-list"], gtk::glib::Priority::DEFAULT)
            .await
            .ok()?;
        stream
            .read_bytes_future(4096, gtk::glib::Priority::DEFAULT)
            .await
            .ok()
    })?;
    Some(String::from_utf8_lossy(&bytes).trim_end().to_owned())
}

/// Double-click and Enter (which both open the item at its position) and
/// the menu's Open open the file a Recent row stands for, in its default
/// application, as from its folder; the row's own `recent:` address was
/// refused ("Only local paths, smb:// locations and connected devices
/// are supported in this build.").
///
/// parity: SIDE-026
#[gtk::test]
fn a_recent_file_opens_the_file_it_stands_for() {
    let (fixture, test) = recent_files_listing("Report.txt");
    let file = fixture.uri_of("Documents/Report.txt");

    test.window.activate_from_view(test.position_of("Report.txt"));
    wait_until("the file to open", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), std::slice::from_ref(&file));

    select_names(&test, &["Report.txt"]);
    test.window.right_click(Some(test.position_of("Report.txt")));
    test.window.context_menu().click_row("Open");
    wait_until("the file to open again", || {
        test.context.recorded_launches().len() == 2
    });
    assert_eq!(test.context.recorded_launches(), [file.clone(), file]);
    assert_eq!(
        test.window.current_uri().as_deref(),
        Some(RECENT_URI),
        "a file opens without leaving Recent files"
    );
}

/// A Recent row whose file is no longer where it was says so in the
/// app's "Item Not Found" words, and opens nothing.
///
/// parity: SIDE-026
#[gtk::test]
fn a_recent_file_that_is_gone_says_item_not_found() {
    let (fixture, test) = recent_files_listing("Old.txt");
    let position = test.position_of("Old.txt");
    // The folder moves, so the file is gone from where Recent names it.
    fs::rename(fixture.path("Documents"), fixture.path("Moved")).expect("the folder can move");

    test.window.activate_item(position);

    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Item Not Found");
    let message = dialog.message_text();
    assert!(message.contains("Could not find this item"), "{message}");
    assert!(!message.contains("supported in this build"), "{message}");
    assert!(test.context.recorded_launches().is_empty(), "nothing opened");
    dialog.press("OK");
}

/// A window showing Recent files with `Documents/Budget.txt` selected,
/// and that file's address.
fn selected_recent_file() -> (Fixture, TestWindow, String) {
    let (fixture, test) = recent_files_listing("Budget.txt");
    let file = fixture.uri_of("Documents/Budget.txt");
    select_names(&test, &["Budget.txt"]);
    (fixture, test, file)
}

/// Copy as path on a Recent row copies the path of the file it stands
/// for, as from its folder.
///
/// parity: SIDE-026
#[gtk::test]
fn copy_as_path_on_a_recent_row_copies_the_file_path() {
    let (fixture, test, _) = selected_recent_file();
    let path = fixture.path("Documents/Budget.txt");

    assert_eq!(
        test.window.path_to_copy(),
        CopiedPath::Address(format!("\"{}\"", path.display()))
    );
}

/// Copy on a Recent row puts the file it stands for on the clipboard.
///
/// parity: SIDE-026
#[gtk::test]
fn copy_on_a_recent_row_copies_the_file() {
    let (_fixture, test, file) = selected_recent_file();

    test.activate("copy", None);

    wait_until("the copied file", || copied_uris(&test).is_some());
    assert_eq!(copied_uris(&test).as_deref(), Some(file.as_str()));
}

/// A drag of a Recent row carries the file it stands for.
///
/// parity: SIDE-026
#[gtk::test]
fn a_recent_row_drags_the_file() {
    let (_fixture, test, file) = selected_recent_file();

    let content = test
        .window
        .drag_content_for(test.position_of("Budget.txt"))
        .expect("a Recent file can be dragged");

    assert_eq!(offered_uris(&content), std::slice::from_ref(&file));
}

/// Properties of a Recent row are the file's.
///
/// parity: SIDE-026
#[gtk::test]
fn properties_of_a_recent_row_are_the_file_properties() {
    let (_fixture, test, file) = selected_recent_file();

    test.activate("properties", None);

    wait_until("Properties", || !test.window.properties_windows().is_empty());
    let properties = test.window.properties_windows().remove(0);
    assert_eq!(properties.widget_name(), file.as_str());
    properties.close();
}

/// Open with on a Recent row opens the file it stands for in the chosen
/// application.
///
/// parity: SIDE-026
#[gtk::test]
fn open_with_on_a_recent_row_opens_the_file() {
    let (_fixture, test, _) = selected_recent_file();
    let _tool = TextTool::install();

    test.activate("open-with-app", Some(TextTool::ID));

    wait_until("Open with to answer", || {
        !test.window.shown_message_text().is_empty()
    });
    assert_eq!(test.window.shown_message_text(), "Opened with Text tool");
}

/// A Recent locations row is the folder itself: it opens in place, and
/// its menu's Open in new tab and Open in new window open that folder.
///
/// parity: SIDE-026
#[gtk::test]
fn a_recent_location_opens_in_place_in_a_new_tab_and_in_a_new_window() {
    require_private_trash();
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let folder = fixture.uri_of("Projects");
    let test = TestWindow::open(&fixture.uri());
    add_to_desktop_history(&folder, FOLDER_CONTENT_TYPE);
    let show_recent_locations = || {
        test.activate("go-to", Some(RECENT_LOCATIONS_URI));
        wait_until("Recent locations", || {
            test.window.current_uri().as_deref() == Some(RECENT_LOCATIONS_URI) && !test.window.is_loading()
        });
        wait_until("the recent folder", || {
            test.names().contains(&"Projects".to_owned())
        });
    };

    show_recent_locations();
    test.window.activate_from_view(test.position_of("Projects"));
    wait_until("the folder in place", || {
        test.window.current_uri().as_deref() == Some(folder.as_str())
    });
    test.wait_for_listing("the folder");

    show_recent_locations();
    select_names(&test, &["Projects"]);
    test.window.right_click(Some(test.position_of("Projects")));
    test.window.context_menu().click_row("Open in new tab");
    wait_until("a new tab", || test.window.tab_count() == 2);
    let tabs = tab_uris(&test);
    assert!(tabs.contains(&folder), "in a new tab: {tabs:?}");

    show_recent_locations();
    test.window.right_click(Some(test.position_of("Projects")));
    test.window.context_menu().click_row("Open in new window");
    let opened = OpenedWindows::only(&[&test.window]);
    wait_until("the new window's folder", || {
        opened.window().current_uri().as_deref() == Some(folder.as_str())
    });
}

/// Open with on a Recent row whose file is gone, an application from the
/// menu or Choose another app, says "Item Not Found", as opening it does.
///
/// parity: SIDE-026
#[gtk::test]
fn open_with_on_a_recent_file_that_is_gone_says_item_not_found() {
    let (fixture, test, _) = selected_recent_file();
    let _tool = TextTool::install();
    fs::rename(fixture.path("Documents"), fixture.path("Moved")).expect("the folder can move");

    for (action, target) in [("open-with-app", Some(TextTool::ID)), ("open-with", None)] {
        test.activate(action, target);

        let dialog = open_dialog(&test);
        assert_eq!(dialog.title_text(), "Item Not Found", "{action}");
        let message = dialog.message_text();
        assert!(
            message.contains("Could not find this item"),
            "{action}: {message}"
        );
        dialog.press("OK");
        super::file_ops_support::wait_for_no_dialog(&test);
    }
}
