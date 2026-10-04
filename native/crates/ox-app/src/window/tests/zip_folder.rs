// SPDX-License-Identifier: AGPL-3.0-only
//! A ZIP opened like a folder in a real window (ARC-026): browsing it in
//! the tab, its read-only commands, Extract all in the bar, copying and
//! dragging items out, and typing a path through it.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, this also saves
//! `native-zip-folder.png`.

use std::fs;

use gtk::prelude::*;
use gtk::{gdk, gio};
use ox_core::location::ArchiveLocation;
use ox_core::settings::{PreferencesUpdate, ZipOpening};

use super::archives::fixture_with_zip;
use super::file_ops_support::is_enabled;
use super::item_dialogs::texts;
use crate::test_support::harness::{capture, descendants, wait_until, Fixture, TestWindow};

/// A window on `fixture` that opens ZIPs like folders.
fn opening_zips_as_folders(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    let choose = PreferencesUpdate {
        zip_opening: Some(ZipOpening::Folder),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(choose, |result| result.expect("saved"));
    wait_until("the preference", || {
        test.context.settings_data().preferences.zip_opening == ZipOpening::Folder
    });
    test
}

/// Opens `Bundle.zip` from the folder and waits for its listing.
fn open_bundle(test: &TestWindow, fixture: &Fixture) -> ArchiveLocation {
    let root = ArchiveLocation::root(&fixture.uri_of("Bundle.zip"));
    test.select_named("Bundle.zip");
    test.activate("open", None);
    wait_until("the ZIP in the tab", || {
        test.window.current_uri() == Some(root.uri())
    });
    test.wait_for_listing("the ZIP's listing");
    root
}

/// The command bar's Extract all button.
fn extract_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class("extract-command"))
        .expect("the bar has Extract all")
}

/// Opening a ZIP shows it in the tab like a folder: its items, folders
/// opening in the tab, Up through it and out of it, Extract all in the
/// bar, and nothing that would change it.
///
/// parity: ARC-026
#[gtk::test]
fn a_zip_opens_in_the_tab_like_a_folder() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    assert!(!extract_button(&test).is_visible(), "only inside a ZIP");
    let root = open_bundle(&test, &fixture);

    assert!(test.shown_dialog().is_none(), "no pop-up window");
    assert_eq!(test.names(), ["Docs", "readme.txt"]);
    assert!(extract_button(&test).is_visible());
    assert!(is_enabled(&test, "extract-all"));
    assert!(!is_enabled(&test, "new-folder"), "a ZIP is read-only");
    test.select_named("readme.txt");
    assert!(is_enabled(&test, "copy"));
    for changing in ["cut", "rename", "trash", "delete"] {
        assert!(!is_enabled(&test, changing), "{changing}");
    }
    capture(&test.window, "native-zip-folder.png");

    test.select_named("Docs");
    test.activate("open", None);
    let docs = root.member("Docs/");
    wait_until("the folder inside", || {
        test.window.current_uri() == Some(docs.uri())
    });
    test.wait_for_listing("the folder's listing");
    assert_eq!(test.names(), ["a.txt"]);
    test.activate("up", None);
    wait_until("the ZIP's root", || test.window.current_uri() == Some(root.uri()));
    test.activate("up", None);
    wait_until("out of the ZIP", || {
        test.window.current_uri() == Some(fixture.uri())
    });
    test.wait_for_listing("the folder");
    test.select_named("Notes 2.txt");
    assert!(!extract_button(&test).is_visible(), "no ZIP selected or open");
}

/// Selecting a ZIP shows Extract all in the bar, as Explorer does, with
/// either way of opening ZIPs; selecting anything else hides it.
///
/// parity: ARC-026
#[gtk::test]
fn selecting_a_zip_shows_extract_all_in_the_bar() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    assert!(!extract_button(&test).is_visible());

    test.select_named("Bundle.zip");
    assert!(extract_button(&test).is_visible());
    test.select_named("Notes 2.txt");
    assert!(!extract_button(&test).is_visible());
    test.select_named("Bundle.zip");
    extract_button(&test).emit_clicked();

    let frame = test.wait_for_dialog("the Extract dialog");
    assert_eq!(frame.title(), "Extract Bundle.zip");
}

/// Extract all in the bar opens the Extract dialog for the ZIP shown.
///
/// parity: ARC-026
#[gtk::test]
fn extract_all_in_the_bar_extracts_the_zip_shown() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);

    extract_button(&test).emit_clicked();

    let frame = test.wait_for_dialog("the Extract dialog");
    let field = descendants::<gtk::Entry>(&frame).remove(0);
    assert_eq!(field.text(), format!("{}/Bundle", fixture.root().display()));
    assert!(texts(&frame)
        .iter()
        .any(|text| text == "Files will be extracted to this folder"));
}

/// Copy extracts the selected items and puts the copies on the
/// clipboard; Paste in a real folder copies them there. Cut is refused.
///
/// parity: ARC-026
#[gtk::test]
fn copied_items_paste_as_real_files() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);
    test.select_named("readme.txt");

    // Ctrl+X runs Cut even while the command is off, to say why.
    test.window.copy_selection(ox_core::clipboard::ClipboardMode::Cut);
    assert!(test.window.shown_message().contains("read-only"));
    test.activate("copy", None);
    wait_until("the copies on the clipboard", || {
        test.window.shown_message().contains("copied")
    });

    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    test.activate("paste", None);
    let pasted = fixture.path("Documents/readme.txt");
    wait_until("the pasted file", || pasted.exists());
    assert_eq!(fs::read(&pasted).expect("pasted"), b"read me");
}

/// A drag of items inside a ZIP offers a file list that is made only when
/// the drop asks for it, and names real copies.
///
/// parity: ARC-026
#[gtk::test]
fn a_drag_offers_copies_made_when_the_drop_asks() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);
    let position = test.position_of("Docs");

    let before = copy_folders();
    let content: gdk::ContentProvider = test.window.drag_content_for(position).expect("a drag");
    assert!(content.formats().contain_mime_type("text/uri-list"));
    assert_eq!(
        copy_folders(),
        before,
        "nothing is extracted before the drop asks"
    );
    let every_modifier = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK;
    assert_eq!(
        crate::window::file_drag::drag_actions(&content, every_modifier),
        gdk::DragAction::COPY,
        "never a link to a copy that lasts a day, nor the menu that offers one"
    );

    // Two requests at once, as a drop target that reads on hover and on
    // drop may make: one extraction answers both.
    let streams = [
        gio::MemoryOutputStream::new_resizable(),
        gio::MemoryOutputStream::new_resizable(),
    ];
    let finished = std::rc::Rc::new(std::cell::Cell::new(0));
    for stream in &streams {
        let done = std::rc::Rc::clone(&finished);
        let request = content.write_mime_type_future("text/uri-list", stream, gtk::glib::Priority::DEFAULT);
        gtk::glib::spawn_future_local(async move {
            request.await.expect("the file list");
            done.set(done.get() + 1);
        });
    }
    wait_until("the copies", || finished.get() == 2);
    let lists: Vec<String> = streams
        .iter()
        .map(|stream| String::from_utf8(stream.steal_as_bytes().to_vec()).expect("text"))
        .collect();
    assert_eq!(lists[0], lists[1], "the same copies");
    let uri = lists[0].lines().next().expect("one item").trim().to_owned();
    let copy = gio::File::for_uri(&uri).path().expect("a local copy");
    assert!(copy.ends_with("Docs"));
    assert_eq!(fs::read(copy.join("a.txt")).expect("copied"), b"first");
    assert!(
        crate::window::zip_copies::is_zip_copy(&uri),
        "a copy, never pinned"
    );
    assert_eq!(copy_folders().len(), before.len() + 1, "one extraction");
}

/// A drop of items dragged out of a ZIP in this app takes the copies
/// straight from the drag, and refuses to link to them.
///
/// parity: ARC-026
#[gtk::test]
fn a_drop_in_the_app_copies_out_of_a_zip_and_never_links() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);
    let content = test
        .window
        .drag_content_for(test.position_of("readme.txt"))
        .expect("a drag")
        .downcast::<crate::window::zip_copies::ZipDragContent>()
        .expect("the drag of items inside a ZIP");
    let copies = std::rc::Rc::new(std::cell::RefCell::new(None));
    let made = std::rc::Rc::clone(&copies);
    gtk::glib::spawn_future_local(async move {
        made.replace(Some(content.copies().await));
    });
    wait_until("the copies", || copies.borrow().is_some());
    let copies = copies.take().expect("made").expect("the copies");
    test.window.end_file_drag();
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");

    let linked = test
        .window
        .drop_files(&copies, None, crate::window::file_drop::DropAction::Link);
    assert!(!linked);
    assert_eq!(
        test.window.shown_message(),
        "Items from a ZIP cannot be linked. Copy or move them instead."
    );
    assert!(test
        .window
        .drop_files(&copies, None, crate::window::file_drop::DropAction::Copy));
    let dropped = fixture.path("Documents/readme.txt");
    wait_until("the copied file", || dropped.exists());
    assert!(!dropped.is_symlink());
    assert_eq!(fs::read(&dropped).expect("copied"), b"read me");
}

/// The names of the folders of copies taken out of ZIPs so far.
fn copy_folders() -> std::collections::BTreeSet<String> {
    fs::read_dir(ox_core::archive::copies_root())
        .map(|children| {
            children
                .filter_map(Result::ok)
                .map(|child| child.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("copy-"))
                .collect()
        })
        .unwrap_or_default()
}

/// A ZIP whose name GIO and the canonical form escape differently opens
/// too.
///
/// parity: ARC-026
#[gtk::test]
fn a_zip_named_with_brackets_opens_like_a_folder() {
    let fixture = fixture_with_zip();
    fs::rename(fixture.path("Bundle.zip"), fixture.path("Bundle (1).zip")).expect("renamed");
    let test = opening_zips_as_folders(&fixture);
    test.wait_for_listing("the folder");
    test.select_named("Bundle (1).zip");
    test.activate("open", None);
    wait_until("the ZIP in the tab", || {
        test.window
            .current_uri()
            .is_some_and(|uri| ox_core::location::is_archive_location(&uri))
    });
    test.wait_for_listing("the ZIP's listing");
    assert_eq!(test.names(), ["Docs", "readme.txt"]);
}

/// A path typed through the ZIP to a file opens its folder with the file
/// selected.
///
/// parity: ARC-026
#[gtk::test]
fn a_path_to_a_file_inside_selects_it() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    let typed = format!("{}/Bundle.zip/Docs/a.txt", fixture.root().display());

    test.window.navigate(&typed).expect("a path through the ZIP");

    let docs = ArchiveLocation::root(&fixture.uri_of("Bundle.zip")).member("Docs/");
    wait_until("the file's folder", || {
        test.window.current_uri() == Some(docs.uri())
    });
    test.wait_for_listing("the folder's listing");
    wait_until("the file selected", || test.selected_names() == ["a.txt"]);
}

/// The address bar shows a path through the ZIP, and typing one opens
/// the folder inside it.
///
/// parity: ARC-026
#[gtk::test]
fn a_path_through_the_zip_opens_the_folder_inside() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    let typed = format!("{}/Bundle.zip/Docs", fixture.root().display());

    test.window.submit_address(&typed);

    let docs = ArchiveLocation::root(&fixture.uri_of("Bundle.zip")).member("Docs/");
    wait_until("the folder inside", || {
        test.window.current_uri() == Some(docs.uri())
    });
    test.wait_for_listing("the folder's listing");
    assert_eq!(test.names(), ["a.txt"]);
}

/// A folder the ZIP does not have is not found, as a missing folder on
/// disk is, rather than listed as an empty folder.
///
/// parity: ARC-026
#[gtk::test]
fn a_folder_the_zip_does_not_have_is_not_found() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    let typed = format!("{}/Bundle.zip/Nope", fixture.root().display());

    test.window.navigate(&typed).expect("a path through the ZIP");

    let missing = ArchiveLocation::root(&fixture.uri_of("Bundle.zip")).member("Nope/");
    assert_eq!(test.window.current_uri(), Some(missing.uri()));
    test.wait_for_listing("the listing");
    assert_eq!(
        test.window.load_error(),
        Some(ox_core::entry::EntryError::NotFound("Nope".to_owned()).to_string())
    );
}

/// A ZIP listing that leaves out unsafe names says so in the message
/// line, as the Compressed folder window says it under its list.
///
/// parity: ARC-004, ARC-026
#[gtk::test]
fn hidden_members_are_announced_in_the_message_line() {
    let fixture = fixture_with_zip();
    let sources = tempfile::tempdir().expect("a folder for the sources");
    let unsafe_name = sources.path().join("back\\slash.txt");
    fs::write(&unsafe_name, b"hidden").expect("a source file");
    let request = ox_core::archive::CompressionRequest {
        uris: vec![ox_core::location::file_uri(&unsafe_name)],
        destination_uri: fixture.uri(),
        archive_name: "Unsafe.zip".to_owned(),
    };
    ox_core::archive::ZipCompressor::new()
        .compress(&request, &ox_core::transfer::Cancellation::new())
        .expect("the ZIP is written");
    let test = opening_zips_as_folders(&fixture);

    let root = ArchiveLocation::root(&fixture.uri_of("Unsafe.zip"));
    test.window.navigate(&root.uri()).expect("the ZIP");
    test.wait_for_listing("the ZIP's listing");

    assert!(test.names().is_empty());
    assert_eq!(test.window.shown_message(), "1 unsafe names or links are hidden.");
}

/// A ZIP another app hands to this one is browsed even with "Open
/// archives as folders" off, so it opens like a folder when ZIPs do.
///
/// parity: ARC-026
#[gtk::test]
fn a_zip_handed_in_opens_like_a_folder_when_zips_do() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    let browse_off = PreferencesUpdate {
        browse_archives: Some(false),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(browse_off, |result| result.expect("saved"));
    wait_until("the preference", || {
        !test.context.settings_data().preferences.browse_archives
    });

    test.window.open_locations(vec![fixture.uri_of("Bundle.zip")]);

    let root = ArchiveLocation::root(&fixture.uri_of("Bundle.zip"));
    wait_until("the ZIP in the tab", || {
        test.window.current_uri() == Some(root.uri())
    });
}

/// "In a pop-up window", the default, keeps the Compressed folder window.
///
/// parity: ARC-026
#[gtk::test]
fn the_pop_up_window_stays_the_default() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(
        test.context.settings_data().preferences.zip_opening,
        ZipOpening::Window
    );
    test.select_named("Bundle.zip");
    test.activate("open", None);
    test.wait_for_dialog("the Compressed folder window");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// With "In a pop-up window" a path typed through a ZIP stays a path,
/// which names no folder: the tab stays where it was.
///
/// parity: ARC-026
#[gtk::test]
fn a_path_through_a_zip_stays_a_path_with_the_pop_up_window() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    let typed = format!("{}/Bundle.zip/Docs", fixture.root().display());

    test.window.submit_address(&typed);

    wait_until("the refusal", || !test.window.shown_message().is_empty());
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// Inside a ZIP the menus offer only what applies to a read-only folder,
/// as Explorer's do.
///
/// parity: ARC-026
#[gtk::test]
fn the_menus_inside_a_zip_are_short() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);
    let position = test.position_of("readme.txt");
    test.window.right_click(Some(position));
    assert_eq!(
        test.window.context_menu().row_labels(),
        ["Open", "-", "Copy", "Copy path", "-", "Extract all…"]
    );
    test.window.context_menu().popdown();
    test.window.right_click(None);
    assert_eq!(
        test.window.context_menu().row_labels(),
        ["Extract all…", "Refresh"]
    );
}

/// A file inside a ZIP opened with a double-click goes to its application
/// as a private copy, which is removed once the application has had time
/// to read it: the copies are in the runtime folder, in memory until
/// logout, and every double-click left one behind before.
///
/// parity: ARC-026
#[gtk::test]
fn a_copy_opened_from_a_zip_is_removed_later() {
    let fixture = fixture_with_zip();
    let test = opening_zips_as_folders(&fixture);
    open_bundle(&test, &fixture);
    crate::window::zip_folder::keep_copies_for_tests(Some(std::time::Duration::from_millis(300)));

    test.select_named("readme.txt");
    test.activate("open", None);
    wait_until("the copy to be handed to its application", || {
        !test.context.recorded_launches().is_empty()
    });
    let launched = test
        .context
        .recorded_launches()
        .last()
        .cloned()
        .expect("a launch");
    let copy = gio::File::for_uri(&launched).path().expect("a local copy");
    let folder = copy.parent().expect("its private folder").to_path_buf();
    let removed = || !folder.exists();
    assert!(copy.is_file(), "the copy is there while its application opens it");

    wait_until("the copy to be removed", removed);
    crate::window::zip_folder::keep_copies_for_tests(None);
}
