// SPDX-License-Identifier: AGPL-3.0-only
//! A 7z archive in a real window: it opens like a ZIP, following
//! Settings › ZIP & archives, files inside open as read-only copies, and
//! Extract all and Extract here unpack it, as Windows 11 handles 7z in
//! File Explorer (What's new in Windows 11, version 24H2).
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, this also saves
//! `native-7z-folder.png` and `native-7z-extract-dialog.png`.

use std::fs;

use gtk::gio;
use gtk::prelude::*;
use ox_core::archive::{CompressionRequest, ZipCompressor};
use ox_core::location::ArchiveLocation;
use ox_core::settings::{PreferencesUpdate, ZipOpening};
use ox_core::transfer::Cancellation;

use super::archives::{archive_browser, opening_zips_in_the_window};
use super::item_dialogs::{press, texts};
use crate::test_support::harness::{capture, descendants, wait_until, Fixture, TestWindow};

/// A standard fixture with `Bundle.7z` holding `Docs/a.txt` and
/// `readme.txt`, written by the app's own Compress to › 7z File.
fn fixture_with_7z() -> Fixture {
    let fixture = Fixture::standard();
    let sources = tempfile::tempdir().expect("a folder for the sources");
    fs::create_dir(sources.path().join("Docs")).expect("fixture folder");
    fs::write(sources.path().join("Docs/a.txt"), b"first").expect("fixture file");
    fs::write(sources.path().join("readme.txt"), b"read me").expect("fixture file");
    let uri_of = |name: &str| gio::File::for_path(sources.path().join(name)).uri().to_string();
    let request = CompressionRequest {
        uris: vec![uri_of("Docs"), uri_of("readme.txt")],
        destination_uri: fixture.uri(),
        archive_name: "Bundle.7z".to_owned(),
    };
    ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .expect("the fixture 7z is written");
    fixture
}

/// A window on `fixture` that opens ZIPs, and so 7z archives, like
/// folders.
fn opening_archives_as_folders(fixture: &Fixture) -> TestWindow {
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

/// Opens `Bundle.7z` from the folder and waits for its listing.
fn open_bundle(test: &TestWindow, fixture: &Fixture) -> ArchiveLocation {
    let root = ArchiveLocation::root(&fixture.uri_of("Bundle.7z"));
    test.wait_for_listing("the folder");
    test.select_named("Bundle.7z");
    test.activate("open", None);
    wait_until("the 7z in the tab", || {
        test.window.current_uri() == Some(root.uri())
    });
    test.wait_for_listing("the 7z's listing");
    root
}

/// The command bar's Extract all button.
fn extract_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class("extract-command"))
        .expect("the bar has Extract all")
}

/// Opening a 7z shows it in the tab like a folder: its items, a folder
/// opening in the tab, Up through it and out of it, and Extract all in
/// the bar.
///
/// parity: ARC-022, ARC-026
#[gtk::test]
fn a_7z_opens_in_the_tab_like_a_folder() {
    let fixture = fixture_with_7z();
    let test = opening_archives_as_folders(&fixture);
    let root = open_bundle(&test, &fixture);

    assert!(test.shown_dialog().is_none(), "no pop-up window");
    assert_eq!(test.names(), ["Docs", "readme.txt"]);
    assert!(extract_button(&test).is_visible());
    capture(&test.window, "native-7z-folder.png");

    test.select_named("Docs");
    test.activate("open", None);
    let docs = root.member("Docs/");
    wait_until("the folder inside", || {
        test.window.current_uri() == Some(docs.uri())
    });
    test.wait_for_listing("the folder's listing");
    assert_eq!(test.names(), ["a.txt"]);
    test.activate("up", None);
    wait_until("the 7z's root", || test.window.current_uri() == Some(root.uri()));
    test.activate("up", None);
    wait_until("out of the 7z", || {
        test.window.current_uri() == Some(fixture.uri())
    });
}

/// A file inside a 7z opens as a private copy, as one inside a ZIP.
///
/// parity: ARC-006, ARC-026
#[gtk::test]
fn a_file_inside_a_7z_opens_as_a_private_copy() {
    let fixture = fixture_with_7z();
    let test = opening_archives_as_folders(&fixture);
    open_bundle(&test, &fixture);

    test.select_named("readme.txt");
    test.activate("open", None);

    wait_until("the copy handed to its application", || {
        !test.context.recorded_launches().is_empty()
    });
    let launched = test.context.recorded_launches()[0].clone();
    let copy = gio::File::for_uri(&launched).path().expect("a local copy");
    assert_eq!(fs::read(&copy).expect("the copy"), b"read me");
    assert!(fs::metadata(&copy).expect("the copy").permissions().readonly());
    assert!(!fixture.path("readme.txt").exists(), "browsing extracts nothing");
}

/// With "In a pop-up window" a 7z opens in the archive window, and with
/// "Open archives as folders" off it opens in its default application,
/// as a ZIP does.
///
/// parity: ARC-022, ARC-024
#[gtk::test]
fn a_7z_follows_the_zip_settings() {
    let fixture = fixture_with_7z();
    let test = opening_zips_in_the_window(&fixture);
    test.select_named("Bundle.7z");
    test.activate("open", None);
    let browser = archive_browser(&test);
    wait_until("the listing", || browser.row_names() == ["Docs", "readme.txt"]);
    test.shown_dialog().expect("the archive window").close();
    super::file_ops_support::wait_for_no_dialog(&test);

    let turn_off = PreferencesUpdate {
        browse_archives: Some(false),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(turn_off, |result| result.expect("saved"));
    wait_until("the preference", || {
        !test.context.settings_data().preferences.browse_archives
    });
    test.select_named("Bundle.7z");
    test.activate("open", None);
    wait_until("the default application", || {
        !test.context.recorded_launches().is_empty()
    });
    assert!(test.context.recorded_launches()[0].contains("Bundle.7z"));
}

/// Extract all opens the Extract dialog for a 7z, with Windows' check
/// box, and unpacks it into a new folder that it then shows; Extract
/// here unpacks it beside the archive.
///
/// parity: ARC-009, ARC-024
#[gtk::test]
fn extract_all_and_extract_here_unpack_a_7z() {
    let fixture = fixture_with_7z();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    test.select_named("Bundle.7z");
    assert!(extract_button(&test).is_visible(), "Extract all in the bar");

    test.activate("extract-all", None);

    let frame = test.wait_for_dialog("the Extract dialog");
    assert_eq!(frame.title(), "Extract Bundle.7z");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    let show = descendants::<gtk::CheckButton>(&frame)
        .into_iter()
        .find_map(|check| check.label())
        .expect("the check box");
    assert_eq!(show, "Show extracted files when complete");
    let fields = descendants::<gtk::Entry>(&frame);
    assert_eq!(fields[0].text(), format!("{}/Bundle", fixture.root().display()));
    capture(&test.window, "native-7z-extract-dialog.png");
    press(&frame, "Extract");
    let extracted = fixture.path("Bundle");
    wait_until("the extracted folder shown", || {
        test.window.current_uri() == Some(gio::File::for_path(&extracted).uri().to_string())
    });
    assert_eq!(
        fs::read(extracted.join("Docs/a.txt")).expect("extracted"),
        b"first"
    );
    assert_eq!(
        fs::read(extracted.join("readme.txt")).expect("extracted"),
        b"read me"
    );

    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    test.select_named("Bundle.7z");
    test.activate("extract-here", None);
    let beside = fixture.path("Bundle (2)/readme.txt");
    wait_until("the files beside the 7z", || beside.exists());
    assert_eq!(fs::read(beside).expect("extracted"), b"read me");
}

/// A damaged 7z, or one whose names are encrypted, says why in the tab.
///
/// parity: ARC-016, ARC-017, ARC-026
#[gtk::test]
fn a_damaged_or_encrypted_7z_says_why_in_the_tab() {
    let fixture = fixture_with_7z();
    let bundle = fs::read(fixture.path("Bundle.7z")).expect("the 7z");
    fs::write(fixture.path("Broken.7z"), &bundle[..bundle.len() / 2]).expect("a cut 7z");
    fs::write(fixture.path("Secret.7z"), encrypted_names()).expect("an encrypted 7z");
    let test = opening_archives_as_folders(&fixture);
    test.wait_for_listing("the folder");

    for (name, words) in [("Broken.7z", "damaged"), ("Secret.7z", "encrypted")] {
        test.select_named(name);
        test.activate("open", None);
        let root = ArchiveLocation::root(&fixture.uri_of(name));
        wait_until(name, || test.window.current_uri() == Some(root.uri()));
        test.wait_for_listing("the refusal");
        let error = test.window.load_error().expect("the tab says why");
        assert!(error.contains(words), "{name}: {error}");
        test.activate("up", None);
        wait_until("back", || test.window.current_uri() == Some(fixture.uri()));
        test.wait_for_listing("the folder");
    }
}

/// A 7z whose header, and so every name, is encrypted with AES, as 7-Zip
/// writes it with "Encrypt file names".
fn encrypted_names() -> Vec<u8> {
    let packed = [0x5a_u8; 32];
    let header: Vec<u8> = [
        &[0x17, 0x06, 0x00, 0x01, 0x09, 32, 0x00][..],
        &[
            0x07, 0x0b, 0x01, 0x00, 0x01, 0x24, 0x06, 0xf1, 0x07, 0x01, 0x02, 0x13, 0x00,
        ],
        &[0x0c, 32, 0x00, 0x08, 0x00, 0x00],
    ]
    .concat();
    let mut start = Vec::new();
    start.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    start.extend_from_slice(&(header.len() as u64).to_le_bytes());
    start.extend_from_slice(&crc32(&header).to_le_bytes());
    let mut bytes = b"7z\xBC\xAF\x27\x1C\x00\x04".to_vec();
    bytes.extend_from_slice(&crc32(&start).to_le_bytes());
    bytes.extend_from_slice(&start);
    bytes.extend_from_slice(&packed);
    bytes.extend_from_slice(&header);
    bytes
}

/// The CRC-32 of `bytes`, as 7z headers record it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
