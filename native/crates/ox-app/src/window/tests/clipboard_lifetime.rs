// SPDX-License-Identifier: AGPL-3.0-only
//! How long what Copy and Cut put on the clipboard stays there: past the
//! last window, until another application copies something (CLIP-010).
//!
//! Windows Explorer's process outlives its windows, so files copied in a
//! window that was then closed still paste in any application. Without a
//! clipboard manager that keeps file lists (Weston, GNOME Shell, or an X11
//! session without one), an X11 or Wayland clipboard empties when its
//! owner's process ends; these tests prove the application keeps running
//! while it serves the clipboard and stops as soon as another owner takes
//! it.

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{KDE_CUT, URI_LIST};

use super::file_ops_support::{is_enabled, select_names};
use std::time::Duration;

use crate::test_support::harness::{settle, wait_for, wait_until, Fixture, TestWindow};

/// Another client of the display claims the clipboard with `text`, as an
/// editor's Copy would.
///
/// It is a second connection to the test's X server, so the window's
/// clipboard sees a foreign owner, not a change of its own.
fn another_app_copies(text: &str) -> gdk::Display {
    let name = gdk::Display::default().expect("the test display").name();
    let other = gdk::Display::open(Some(&name)).expect("a second connection to the test display");
    other.clipboard().set_text(text);
    other
}

/// Closes `other` after the test's own connection takes the clipboard
/// back: GTK reports a read whose owner's connection closed under it as a
/// critical warning, which fails a test.
fn close_when_read(other: &gdk::Display) {
    let own = gdk::Display::default().expect("the test display").clipboard();
    own.set_text("Synthetic test text");
    wait_until("the test to own the clipboard again", || own.is_local());
    wait_for(Duration::from_millis(300));
    other.close();
}

/// parity: CLIP-010
#[gtk::test]
fn copied_files_outlive_the_last_window_until_another_app_copies() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    let clipboard = test.window.clipboard();
    wait_until("the window to serve its copy", || test.context.serves_clipboard());

    test.window.close();
    settle();

    assert!(!test.window.is_visible(), "the last window closed");
    assert!(
        test.context.serves_clipboard(),
        "the application keeps running to serve the copied files"
    );
    assert!(
        clipboard.is_local(),
        "the files are still this application's to give"
    );
    assert!(clipboard.formats().contain_mime_type(URI_LIST));

    let other = another_app_copies("Quarterly notes");
    wait_until("another owner to take the clipboard", || !clipboard.is_local());
    wait_until("the application to let go", || !test.context.serves_clipboard());
    close_when_read(&other);
}

/// Another client of the display offers `uris` as a cut, as KDE's
/// Klipper does when it puts its last entry back on a clipboard that was
/// emptied: a URI list with KDE's cut marker.
fn another_app_offers_a_cut(uris: &[String]) -> gdk::Display {
    let name = gdk::Display::default().expect("the test display").name();
    let other = gdk::Display::open(Some(&name)).expect("a second connection to the test display");
    let mut list = uris.join("\r\n");
    list.push_str("\r\n");
    let providers = [
        gdk::ContentProvider::for_bytes(URI_LIST, &glib::Bytes::from_owned(list.into_bytes())),
        gdk::ContentProvider::for_bytes(KDE_CUT, &glib::Bytes::from_static(b"1")),
    ];
    other
        .clipboard()
        .set_content(Some(&gdk::ContentProvider::new_union(&providers)))
        .expect("the other client claims the clipboard");
    other
}

/// KDE's Klipper puts the last clipboard entry back as soon as an owner
/// empties the clipboard, so the cut a paste has just moved and cleared
/// returns, naming items that are no longer there. It is no cut any more:
/// Paste stays disabled, as Windows leaves it after a cut is pasted, and a
/// second Ctrl+V asks nothing about the moved files. A cut of other items
/// from the same owner is a cut as before.
///
/// parity: CLIP-008, CLIP-016
#[gtk::test]
fn a_cut_a_clipboard_manager_puts_back_after_the_paste_is_no_cut() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 10.txt"]);
    test.activate("cut", None);
    let cut = vec![fixture.uri_of("Notes 10.txt")];
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    wait_until("the moved item", || {
        fixture.path("Documents/Notes 10.txt").is_file()
    });
    wait_until("the cut to be emptied", || !is_enabled(&test, "paste"));

    let manager = another_app_offers_a_cut(&cut);
    let clipboard = test.window.clipboard();
    wait_until("the manager to own the clipboard", || {
        clipboard.formats().contain_mime_type(KDE_CUT)
    });
    // Paste must not come back once the read is done.
    wait_for(Duration::from_millis(500));

    assert!(!is_enabled(&test, "paste"), "the moved cut is nothing to paste");
    close_when_read(&manager);

    let other = another_app_offers_a_cut(&[fixture.uri_of("Notes 2.txt")]);
    wait_until("a new cut to enable Paste", || is_enabled(&test, "paste"));
    close_when_read(&other);
}
