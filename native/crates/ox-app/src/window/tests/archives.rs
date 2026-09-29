// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP archives in a real window: browsing, Extract all…, Extract here and
//! Compress to ZIP file.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, these also save
//! `native-archive-browser.png` and `native-extract-dialog.png`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gtk::prelude::*;
use ox_core::archive::{CompressionRequest, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

use super::item_dialogs::{press, texts};
use crate::archive_view::ArchiveBrowserView;
use crate::test_support::harness::{capture, descendants, wait_for, wait_until, Fixture, TestWindow};

/// A standard fixture with `Bundle.zip`, which holds `Docs/a.txt` and
/// `readme.txt`.
fn fixture_with_zip() -> Fixture {
    let fixture = Fixture::standard();
    let sources = tempfile::tempdir().expect("a folder for the sources");
    fs::create_dir(sources.path().join("Docs")).expect("fixture folder");
    fs::write(sources.path().join("Docs/a.txt"), b"first").expect("fixture file");
    fs::write(sources.path().join("readme.txt"), b"read me").expect("fixture file");
    let request = CompressionRequest {
        uris: vec![
            uri_in(sources.path(), "Docs"),
            uri_in(sources.path(), "readme.txt"),
        ],
        destination_uri: fixture.uri(),
        archive_name: "Bundle.zip".to_owned(),
    };
    ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .expect("the fixture ZIP is written");
    fixture
}

fn uri_in(folder: &Path, name: &str) -> String {
    file_uri(&folder.join(name))
}

/// The archive browser inside the dialog shown.
fn archive_browser(test: &TestWindow) -> ArchiveBrowserView {
    let frame = test.wait_for_dialog("the archive browser");
    descendants::<ArchiveBrowserView>(&frame)
        .into_iter()
        .next()
        .expect("the dialog holds the archive browser")
}

/// parity: ARC-002, ARC-003, ARC-006
#[gtk::test]
fn opening_a_zip_browses_it_and_opens_a_member_as_a_private_copy() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");

    test.activate("open", None);

    let frame = test.wait_for_dialog("the archive browser");
    assert_eq!(frame.title(), "Bundle.zip — Compressed folder");
    assert_eq!(
        frame.button_labels(),
        ["Extract all…", "Open in archive manager", "Close"]
    );
    let browser = archive_browser(&test);
    wait_until("the listing", || !browser.row_names().is_empty());
    assert_eq!(browser.row_names(), ["Docs", "readme.txt"]);
    capture(&test.window, "native-archive-browser.png");
    browser.activate_row("Docs");
    wait_until("the Docs folder", || browser.row_names() == ["a.txt"]);
    assert!(
        browser.path_text().ends_with(" › Docs/"),
        "{}",
        browser.path_text()
    );
    press(&frame, "Up");
    wait_until("the top again", || browser.row_names().len() == 2);
    browser.activate_row("readme.txt");
    wait_until("the private copy", || {
        !test.context.recorded_launches().is_empty()
    });
    let opened = test.context.recorded_launches()[0].clone();
    assert!(opened.contains("winspace-archive-previews"), "{opened}");
    assert_eq!(
        browser.status_text(),
        "Opened a temporary copy. Changes are not saved back to the ZIP."
    );
    assert!(!fixture.path("readme.txt").exists(), "browsing extracts nothing");
}

/// parity: ARC-009, ARC-011, ARC-012
#[gtk::test]
fn extract_all_unpacks_into_a_new_folder_and_shows_it() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");

    test.activate("extract-all", None);

    let frame = test.wait_for_dialog("the Extract dialog");
    assert_eq!(frame.title(), "Extract compressed folder");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    assert!(texts(&frame)
        .iter()
        .any(|text| text == "2 files · 1 folder · 12 bytes unpacked"));
    let expected_target = format!("Extract into: {}/Bundle", fixture.root().display());
    assert!(texts(&frame).contains(&expected_target), "{:?}", texts(&frame));
    capture(&test.window, "native-extract-dialog.png");
    press(&frame, "Extract");

    let extracted = fixture.path("Bundle");
    wait_until("the extracted folder", || {
        test.window.current_uri() == Some(file_uri(&extracted))
    });
    assert_eq!(
        fs::read(extracted.join("Docs/a.txt")).expect("an extracted file"),
        b"first"
    );
    assert_eq!(test.window.shown_message(), "Extracted 2 files into Bundle.");
}

/// parity: ARC-010
#[gtk::test]
fn extract_refuses_a_bad_name_and_keeps_the_dialog_open() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    let fields = descendants::<gtk::Entry>(&frame);

    fields[1].set_text("a/b");
    press(&frame, "Extract");

    assert!(!frame.error_text().is_empty());
    assert_eq!(test.shown_dialog(), Some(frame));
}

/// parity: ARC-025
#[gtk::test]
fn extract_here_uses_the_next_free_name() {
    let fixture = fixture_with_zip();
    fs::create_dir(fixture.path("Bundle")).expect("a folder with the archive's name");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");

    test.activate("extract-here", None);

    let extracted = fixture.path("Bundle (2)");
    wait_until("the extracted folder", || extracted.join("readme.txt").exists());
    assert_eq!(
        fs::read_dir(fixture.path("Bundle"))
            .expect("the old folder")
            .count(),
        0
    );
    wait_until("the toast", || {
        test.window.shown_message() == "Extracted 2 files into Bundle (2)."
    });
}

/// parity: OPS-024
#[gtk::test]
fn nothing_is_extracted_while_a_file_operation_runs() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    let running = test.window.begin_operation("Preparing copy…");

    test.activate("extract-here", None);
    wait_for(Duration::from_millis(200));

    assert!(running.is_some());
    assert!(!fixture.path("Bundle").exists(), "the extraction waits");
    test.window.end_operation();
}

/// parity: ARC-023
#[gtk::test]
fn compress_to_zip_puts_the_selection_into_a_new_zip_beside_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");

    test.activate("compress-to-zip", None);

    let archive = fixture.path("Documents.zip");
    wait_until("the new ZIP", || archive.exists());
    wait_until("the toast", || {
        test.window.shown_message() == "Compressed 1 items into Documents.zip."
    });
    test.wait_for_listing("the folder listed again");
    wait_until("the ZIP in the listing", || {
        test.names().contains(&"Documents.zip".to_owned())
    });
}

/// parity: ARC-009, ARC-023, ARC-025
#[gtk::test]
fn the_archive_commands_follow_the_selection() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    let is_enabled = |name: &str| {
        test.window
            .lookup_action(name)
            .is_some_and(|action| action.is_enabled())
    };

    test.select_named("Notes 2.txt");
    assert!(!is_enabled("extract-all"));
    assert!(is_enabled("compress-to-zip"));
    test.select_named("Bundle.zip");
    assert!(is_enabled("extract-all"));
    assert!(is_enabled("extract-here"));
}

/// parity: ARC-021
#[gtk::test]
fn open_in_archive_manager_hands_the_zip_to_the_desktop() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("open", None);
    let frame = test.wait_for_dialog("the archive browser");

    press(&frame, "Open in archive manager");

    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Bundle.zip")]);
    assert!(test.shown_dialog().is_none());
}
