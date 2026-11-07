// SPDX-License-Identifier: AGPL-3.0-only
//! Settings' two clipboard switches turned off. Both are on by default, so
//! the `clipboard` and `clipboard_interop` tests cover them on; off, the
//! window behaves as Windows 11 File Explorer does:
//!
//! - "Paste a picture from the clipboard as a new file" off: a picture is
//!   no data, so Paste and Paste into folder are unavailable and Ctrl+V
//!   does nothing (CLIP-015, CLIP-016).
//! - "Also copy file paths as text when copying files" off: Copy and Cut
//!   publish only the file formats, so a text reader gets nothing
//!   (CLIP-004).

use std::fs;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{CUSTOM, GNOME, KDE_CUT, TEXT, URI_LIST};

use super::clipboard_interop::scripted_owner::{Script, ScriptedOwner};
use super::file_ops_support::{is_enabled, select_names};
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};

/// The first bytes of a PNG, which a paste writes as they are.
const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Makes a PNG picture the clipboard of `test`'s window, as another
/// application copying a screenshot does.
fn copy_a_picture(test: &TestWindow) {
    let picture = gdk::ContentProvider::for_bytes("image/png", &glib::Bytes::from_static(&PNG));
    test.window
        .clipboard()
        .set_content(Some(&picture))
        .expect("the test owns the clipboard");
}

/// The names of the `.png` files in `folder` of `fixture`.
fn pictures_in(fixture: &Fixture, folder: &str) -> Vec<String> {
    fs::read_dir(fixture.path(folder))
        .expect("a readable folder")
        .filter_map(|entry| entry.expect("a folder entry").file_name().into_string().ok())
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "png")
        })
        .collect()
}

/// With "Paste a picture from the clipboard as a new file" off, a picture
/// is no data: Paste and Paste into folder are unavailable and Ctrl+V
/// writes nothing and says nothing. Turning it on again enables Paste for
/// the same clipboard at once, without copying again.
///
/// parity: CLIP-015, CLIP-016
#[gtk::test]
fn a_picture_is_no_data_while_picture_paste_is_off() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.set_paste_pictures_as_files(false);
    select_names(&test, &["Documents"]);

    copy_a_picture(&test);
    wait_for(Duration::from_millis(200));

    assert!(!is_enabled(&test, "paste"), "Paste is unavailable");
    assert!(!is_enabled(&test, "paste-into"), "Paste into folder too");
    test.window.paste_from_keyboard();
    wait_for(Duration::from_millis(200));
    assert_eq!(test.window.shown_message(), "", "nothing is said");
    assert!(pictures_in(&fixture, "").is_empty(), "nothing is written");

    test.set_paste_pictures_as_files(true);

    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.window.paste_from_keyboard();
    wait_until("the pasted picture", || pictures_in(&fixture, "").len() == 1);
}

/// Turning "Paste a picture from the clipboard as a new file" off while a
/// picture is on the clipboard turns Paste off at once.
///
/// parity: CLIP-015
#[gtk::test]
fn turning_picture_paste_off_turns_paste_off_at_once() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    copy_a_picture(&test);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    test.set_paste_pictures_as_files(false);

    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
}

/// A picture read that started while picture paste was on and answers
/// after the setting was turned off is dropped: the read the change started
/// decides, so Paste stays off. The clipboard's owner never changed, so the
/// owner check alone would let the late answer through.
///
/// parity: CLIP-015
#[gtk::test]
fn a_picture_read_overtaken_by_turning_picture_paste_off_is_dropped() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let slow_owner = ScriptedOwner::new(
        "image/png",
        Script::Late {
            delay: Duration::from_millis(1500),
            payload: PNG.to_vec(),
        },
    );
    let progress = slow_owner.progress();
    test.window
        .clipboard()
        .set_content(Some(&slow_owner))
        .expect("the test owns the clipboard");
    wait_until("the window to read the picture", || progress.was_asked.get());

    test.set_paste_pictures_as_files(false);
    assert!(
        !progress.has_answered.get(),
        "the setting changed while the picture was still being read"
    );
    wait_until("the slow owner to answer", || progress.has_answered.get());
    // The overtaken read ends after the owner's answer reaches it.
    wait_for(Duration::from_millis(300));

    assert!(
        !is_enabled(&test, "paste"),
        "Paste follows the setting, which is off"
    );
    test.window.paste_from_keyboard();
    wait_for(Duration::from_millis(300));
    assert!(
        pictures_in(&fixture, "Documents").is_empty(),
        "nothing is written"
    );
}

/// With "Also copy file paths as text when copying files" off, Copy and
/// Cut publish the file formats only: a reader that takes text only, such
/// as a text editor, gets nothing, and pasting into a folder still copies
/// the files.
///
/// parity: CLIP-004
#[gtk::test]
fn a_copy_offers_no_text_while_paths_as_text_is_off() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.set_copy_paths_as_text(false);
    select_names(&test, &["Notes 2.txt"]);

    for command in ["cut", "copy"] {
        test.activate(command, None);
        let formats = test.window.clipboard().formats();
        for mime_type in [CUSTOM, GNOME, URI_LIST, KDE_CUT] {
            assert!(formats.contain_mime_type(mime_type), "{command}: {mime_type}");
        }
        assert!(!formats.contain_mime_type(TEXT), "{command}: no paths as text");
    }
    let clipboard = test.window.clipboard();
    let read = glib::MainContext::default().block_on(async { clipboard.read_text_future().await });
    assert!(!matches!(read, Ok(Some(_))), "a text reader gets nothing");

    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    wait_until("the copy", || fixture.path("Documents/Notes 2.txt").is_file());
}
