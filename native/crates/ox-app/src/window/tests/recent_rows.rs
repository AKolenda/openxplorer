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
use crate::test_support::harness::{wait_for, wait_until, Fixture, OpenedWindows, TestWindow};
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

/// A row of Recent files may be copied, not cut, renamed or deleted:
/// Cut, Rename and Delete are greyed out in the command bar and left out
/// of the compact menu's strip, which keeps Copy, and Ctrl+X does nothing
/// at all: no message, and the clipboard keeps what it held.
///
/// parity: SIDE-026
#[gtk::test]
fn a_recent_row_cannot_be_cut_renamed_or_deleted() {
    use super::context_menus::choose_menu_style;
    use crate::window::window_action::WindowAction;
    use ox_core::clipboard::ClipboardMode;
    use ox_core::settings::ContextMenu;

    let (_fixture, test, _) = selected_recent_file();
    choose_menu_style(&test, ContextMenu::Win11);
    select_names(&test, &["Budget.txt"]);

    for action in [WindowAction::Cut, WindowAction::Rename, WindowAction::Trash] {
        assert!(
            !test.window.is_action_enabled(action.name()),
            "{} greyed out",
            action.name()
        );
    }
    assert!(
        test.window.is_action_enabled(WindowAction::Copy.name()),
        "Copy stays"
    );

    test.window.right_click(Some(test.position_of("Budget.txt")));
    let menu = test.window.context_menu();
    let strip = menu.strip_labels();
    menu.popdown();
    assert!(strip.contains(&"Copy".to_owned()), "{strip:?}");
    for left_out in ["Cut", "Rename", "Delete"] {
        assert!(!strip.contains(&left_out.to_owned()), "{left_out} in {strip:?}");
    }

    // The display's clipboard is shared with earlier tests, so what it
    // holds before must be what it holds after.
    let before = copied_uris(&test);
    test.window.copy_selection(ClipboardMode::Cut);
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(test.window.shown_message_text(), "", "Ctrl+X says nothing");
    assert_eq!(copied_uris(&test), before, "nothing is cut");
}

/// A Recent files row as `GVfs` lists it: `recent:///<id>`, named `name`,
/// of `content_type`, typed `file_type` (a file, or a shortcut for some
/// remote items), standing for `target`.
fn recent_row(
    id: &str,
    name: &str,
    content_type: &str,
    target: &str,
    file_type: gtk::gio::FileType,
) -> ox_core::entry::Entry {
    let info = gtk::gio::FileInfo::new();
    info.set_file_type(file_type);
    info.set_display_name(name);
    info.set_content_type(content_type);
    info.set_attribute_string("standard::target-uri", target);
    let row = gtk::gio::File::for_uri(&format!("recent:///{id}"));
    ox_core::entry::entry_from_info(&row, &info)
}

/// Adds `rows` to the Recent files listing `test` shows, as `GVfs` would
/// list them; rows for files on a share or typed as shortcuts cannot be
/// made with the private session's own Recent backend.
fn list_recent_rows(test: &TestWindow, rows: Vec<ox_core::entry::Entry>) {
    let id = test.window.imp().session.borrow().active_id().expect("a tab");
    let store = test.window.tab_store(id).expect("the tab's items");
    let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    for row in rows {
        store.append(&crate::folder_view::item::FileItem::new(row));
    }
    wait_until("the added rows", || {
        names.iter().all(|name| test.names().contains(name))
    });
}

/// Counts the window's mounts, each answered "not a folder", so nothing
/// is ever reached; `example.invalid` reports itself not mounted.
fn count_mounts(test: &TestWindow) -> std::rc::Rc<std::cell::Cell<u32>> {
    let mounts = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = std::rc::Rc::clone(&mounts);
    test.window.network().answer_mounts_with(move || {
        counted.set(counted.get() + 1);
        Err(ox_core::network::NetworkError::NotAFolder)
    });
    mounts
}

/// The file a share's Recent row stands for, on a share that is not mounted.
const SHARE_FILE: &str = "smb://example.invalid/share/Plan.txt";

/// A Recent row that `GVfs` types as a shortcut is copied as the file it
/// stands for, as a file-typed one is: Copy is enabled and copies it.
///
/// parity: SIDE-026
#[gtk::test]
fn copy_on_a_shortcut_typed_recent_row_copies_the_file() {
    use crate::window::window_action::WindowAction;

    let (_fixture, test, file) = selected_recent_file();
    let shortcut = recent_row(
        "shortcutrow",
        "Budget (shortcut).txt",
        "text/plain",
        &file,
        gtk::gio::FileType::Shortcut,
    );
    list_recent_rows(&test, vec![shortcut]);
    select_names(&test, &["Budget (shortcut).txt"]);

    assert!(
        test.window.is_action_enabled(WindowAction::Copy.name()),
        "Copy is enabled"
    );
    test.activate("copy", None);
    wait_until("the copied file", || {
        copied_uris(&test).as_deref() == Some(file.as_str())
    });
}

/// Open with, from the menu's application or Choose another app, on a
/// Recent row whose file is on a share that is not mounted mounts the
/// share first, as opening the row does.
///
/// parity: SIDE-026, NET-004
#[gtk::test]
fn open_with_on_a_recent_file_on_an_unmounted_share_mounts_it_first() {
    let (_fixture, test, _) = selected_recent_file();
    let row = recent_row(
        "sharerow",
        "Plan.txt",
        "text/plain",
        SHARE_FILE,
        gtk::gio::FileType::Regular,
    );
    list_recent_rows(&test, vec![row]);
    select_names(&test, &["Plan.txt"]);
    let mounts = count_mounts(&test);
    let _tool = TextTool::install();

    test.activate("open-with-app", Some(TextTool::ID));
    wait_until("the mount before Open with", || mounts.get() == 1);

    test.activate("open-with", None);
    wait_until("the mount before Choose another app", || mounts.get() == 2);
}

/// Properties of a Recent row whose file is on a share that is not
/// mounted mounts the share first.
///
/// parity: SIDE-026, NET-004
#[gtk::test]
fn properties_of_a_recent_file_on_an_unmounted_share_mounts_it_first() {
    let (_fixture, test, _) = selected_recent_file();
    let row = recent_row(
        "sharerow",
        "Plan.txt",
        "text/plain",
        SHARE_FILE,
        gtk::gio::FileType::Regular,
    );
    list_recent_rows(&test, vec![row]);
    select_names(&test, &["Plan.txt"]);
    let mounts = count_mounts(&test);

    test.activate("properties", None);

    wait_until("the mount before Properties", || mounts.get() == 1);
}

/// Opening several Recent rows opens each as the file it stands for: a
/// desktop link to a folder opens that folder in a tab behind, as it does
/// from its own folder, and a file on a share that is not mounted mounts
/// the share first.
///
/// parity: SIDE-026, OPEN-009, NET-004
#[gtk::test]
fn opening_several_recent_rows_opens_each_as_its_file() {
    let (fixture, test, _) = selected_recent_file();
    let link = fixture.path("Projects link.desktop");
    let folder = fixture.uri_of("Documents");
    fs::write(
        &link,
        format!("[Desktop Entry]\nType=Link\nName=Projects\nURL={folder}\n"),
    )
    .expect("a link");
    let rows = vec![
        recent_row(
            "linkrow",
            "Projects link.desktop",
            "application/x-desktop",
            &fixture.uri_of("Projects link.desktop"),
            gtk::gio::FileType::Regular,
        ),
        recent_row(
            "sharerow",
            "Plan.txt",
            "text/plain",
            SHARE_FILE,
            gtk::gio::FileType::Regular,
        ),
    ];
    list_recent_rows(&test, rows);
    let mounts = count_mounts(&test);
    select_names(&test, &["Projects link.desktop", "Plan.txt"]);

    test.activate("open", None);

    wait_until("the link's folder in a tab", || tab_uris(&test).contains(&folder));
    wait_until("the mount before the share's file", || mounts.get() == 1);
    assert!(
        !test
            .context
            .recorded_launches()
            .iter()
            .any(|uri| uri.ends_with(".desktop")),
        "the link is followed, not launched: {:?}",
        test.context.recorded_launches()
    );
}

/// Properties of several Recent rows describes the files they stand for:
/// files in two folders are in "Multiple locations", not all in Recent.
///
/// parity: SIDE-026, PROP-002
#[gtk::test]
fn properties_of_several_recent_rows_are_those_of_their_files() {
    use super::item_dialogs::value_after;

    let (fixture, test, _) = selected_recent_file();
    add_to_desktop_history(&fixture.uri_of("Notes 2.txt"), "text/plain");
    test.window.refresh();
    wait_until("both recent files", || {
        test.names().contains(&"Notes 2.txt".to_owned())
    });
    select_names(&test, &["Budget.txt", "Notes 2.txt"]);

    test.activate("properties", None);

    wait_until("Properties", || !test.window.properties_windows().is_empty());
    let window = test.window.properties_windows().remove(0);
    wait_until("the General tab", || value_after(&window, "Location:").is_some());
    assert_eq!(
        value_after(&window, "Location:").as_deref(),
        Some("Multiple locations")
    );
    window.close();
}

/// Duplicate is greyed out for a Recent row, as Cut, Rename and Delete
/// are: the row is not a file to copy beside itself.
///
/// parity: SIDE-026
#[gtk::test]
fn duplicate_is_greyed_out_for_a_recent_row() {
    use crate::window::window_action::WindowAction;

    let (_fixture, test, _) = selected_recent_file();

    assert!(!test.window.is_action_enabled(WindowAction::Duplicate.name()));
}

/// Properties of several Recent rows whose files are on a share that is
/// not mounted mounts the share first, as Properties of one does; a
/// failed mount opens no Properties.
///
/// parity: SIDE-026, NET-004, PROP-002
#[gtk::test]
fn properties_of_several_recent_files_on_an_unmounted_share_mounts_it_first() {
    let (_fixture, test, _) = selected_recent_file();
    let rows = vec![
        recent_row(
            "sharerow1",
            "Plan.txt",
            "text/plain",
            SHARE_FILE,
            gtk::gio::FileType::Regular,
        ),
        recent_row(
            "sharerow2",
            "Notes.txt",
            "text/plain",
            "smb://example.invalid/share/Notes.txt",
            gtk::gio::FileType::Regular,
        ),
    ];
    list_recent_rows(&test, rows);
    select_names(&test, &["Plan.txt", "Notes.txt"]);
    let mounts = count_mounts(&test);

    test.activate("properties", None);

    wait_until("the mount before Properties", || mounts.get() >= 1);
    wait_until("the failed mount's message", || {
        test.window.shown_message().as_str() == "This location is not a folder."
    });
    assert!(
        test.window.properties_windows().is_empty(),
        "no Properties after a failed mount"
    );
}

/// Several Recent rows standing for folders open in tabs in the order of
/// the view, also when the first one is slower to read than the next.
///
/// parity: SIDE-026, TAB-027
#[gtk::test]
fn several_recent_folders_open_in_view_order_when_the_first_is_slow() {
    use crate::test_support::held;

    let (fixture, test, _) = selected_recent_file();
    fs::create_dir(fixture.path("Alpha")).expect("a folder");
    fs::create_dir(fixture.path("Beta")).expect("a folder");
    let (alpha, beta) = (fixture.uri_of("Alpha"), fixture.uri_of("Beta"));
    let folder = gtk::gio::FileType::Directory;
    let rows = vec![
        recent_row("alpharow", "Alpha", "inode/directory", &alpha, folder),
        recent_row("betarow", "Beta", "inode/directory", &beta, folder),
    ];
    list_recent_rows(&test, rows);
    select_names(&test, &["Alpha", "Beta"]);
    let slow = held::hold(&held::lookup_key(&alpha));

    test.activate("open", None);
    wait_for(std::time::Duration::from_millis(300));
    drop(slow);

    wait_until("both folders in tabs", || {
        let tabs = tab_uris(&test);
        tabs.contains(&alpha) && tabs.contains(&beta)
    });
    let tabs = tab_uris(&test);
    let place = |uri: &String| tabs.iter().position(|tab| tab == uri);
    assert!(place(&alpha) < place(&beta), "view order: {tabs:?}");
}

/// The Details pane describes the file a Recent row stands for: its File
/// location is that file's folder, not Recent files.
///
/// parity: SIDE-026, PROP-009
#[gtk::test]
fn the_details_pane_shows_a_recent_file_location() {
    let (fixture, test, _) = selected_recent_file();
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    let pane = test.window.details_pane();
    select_names(&test, &["Budget.txt"]);
    let documents = fixture.path("Documents");
    let folder = documents.to_str().expect("a UTF-8 fixture path");

    wait_until("the file's folder in the pane", || {
        pane.shown_properties()
            .iter()
            .any(|row| row.name == "File location" && row.value == folder)
    });
}
