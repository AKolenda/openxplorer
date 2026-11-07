// SPDX-License-Identifier: AGPL-3.0-only
//! The Preview pane (VIEW-070): Windows 11's View > Preview pane, which
//! shows the selection's preview alone in the details pane's place, and
//! which the Details pane replaces, as in Windows.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::geometry::laid_out_with_details_pane;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuPopover;
use crate::window::window_action::WindowAction;

/// Whether `action`, a pane toggle, is on.
fn is_on(test: &TestWindow, action: WindowAction) -> bool {
    test.window
        .window_action_state(action)
        .and_then(|state| state.get::<bool>())
        .unwrap_or_default()
}

/// The labels the View menu marks now.
fn view_marks(test: &TestWindow) -> Vec<String> {
    let button = super::support::menu_button_with_class(test, "view-command");
    let menu = button
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("View's menu");
    menu.popup();
    wait_until("the View menu", || menu.is_visible());
    let marked = menu.checked_labels();
    menu.popdown();
    wait_until("the View menu to close", || !menu.is_visible());
    marked
}

/// View > Preview pane shows the preview alone, in the pane the Details
/// pane uses, and closes the Details pane; View > Details pane closes it
/// again. Each has the dot while it is open, and the choice is saved.
///
/// parity: VIEW-070
#[gtk::test]
fn the_preview_pane_and_the_details_pane_take_turns_in_one_pane() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    test.select_named("Notes 2.txt");
    let pane = test.window.imp().details_pane.get();
    assert!(pane.is_visible());
    assert!(pane.shows_properties(), "the Details pane lists properties");
    assert!(view_marks(&test).contains(&"Details pane".to_owned()));

    test.activate("preview-pane", None);

    wait_for_frames(&test.window, 2);
    assert!(is_on(&test, WindowAction::PreviewPane));
    assert!(!is_on(&test, WindowAction::DetailsPane));
    assert!(pane.is_visible(), "the same pane");
    assert!(pane.shows_preview(), "with the preview");
    assert!(!pane.shows_properties(), "and nothing else");
    assert_eq!(pane.title_text(), "Preview pane", "its accessible name");
    let marks = view_marks(&test);
    assert!(marks.contains(&"Preview pane".to_owned()), "{marks:?}");
    assert!(!marks.contains(&"Details pane".to_owned()), "{marks:?}");
    wait_until("the choice to be saved", || {
        let saved = test.context.settings_data().preferences;
        saved.preview_pane && !saved.show_details_pane
    });

    test.activate("details-pane", None);

    assert!(is_on(&test, WindowAction::DetailsPane));
    assert!(!is_on(&test, WindowAction::PreviewPane));
    assert!(pane.shows_properties());
    assert_eq!(pane.title_text(), "Details pane");

    test.activate("details-pane", None);
    assert!(!pane.is_visible(), "closing the open pane closes it");
    test.activate("preview-pane", None);
    test.activate("preview-pane", None);
    assert!(!pane.is_visible());
    assert!(!is_on(&test, WindowAction::DetailsPane) && !is_on(&test, WindowAction::PreviewPane));
}

/// A window opens with the pane the user chose last: the Preview pane
/// when it was saved open.
///
/// parity: VIEW-070
#[gtk::test]
fn a_new_window_opens_the_preview_pane_when_it_was_open() {
    let fixture = Fixture::standard();
    let first = TestWindow::open(&fixture.uri());
    first.activate("preview-pane", None);
    wait_until("the choice to be saved", || {
        first.context.settings_data().preferences.preview_pane
    });
    let second = first.open_beside(&fixture.uri());
    assert!(is_on(&second, WindowAction::PreviewPane));
    assert!(!is_on(&second, WindowAction::DetailsPane));
    assert!(!second.window.imp().details_pane.shows_properties());
}

/// A window on `fixture` with the Preview pane open.
fn preview_window(fixture: &Fixture) -> TestWindow {
    let test = laid_out_with_details_pane(&fixture.uri());
    test.activate("preview-pane", None);
    wait_for_frames(&test.window, 2);
    test
}

/// The Preview pane shows a text file's text, read-only and in a
/// fixed-width font, and a source file's too; a file larger than the
/// limit shows its beginning only, and a binary file, a ZIP, a folder
/// and several items say "No preview available.", as Windows 11's
/// Preview pane does. Nothing selected says "Select a file to preview.".
///
/// parity: VIEW-070
#[gtk::test]
fn the_preview_pane_shows_text_and_says_when_it_cannot() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").expect("source");
    let line = "2026-09-14 09:30:00 INFO request served\n";
    let log = line.repeat(3 * 1024 * 1024 / line.len());
    std::fs::write(fixture.path("server.log"), &log).expect("log");
    let mut binary = b"MZ\x90\0\x03\0\0\0".to_vec();
    binary.extend((0..4096_u32).map(|n| (n * 7 % 251) as u8));
    std::fs::write(fixture.path("setup.exe"), &binary).expect("binary");
    std::fs::write(fixture.path("Bundle.zip"), b"PK\x03\x04\x14\0\0\0\0\0").expect("zip");
    let test = preview_window(&fixture);
    let pane = test.window.imp().details_pane.get();

    wait_until("the hint with nothing selected", || {
        pane.preview_message().as_deref() == Some("Select a file to preview.")
    });

    test.select_named("Notes 2.txt");
    wait_until("the text", || {
        pane.preview_text().as_deref() == Some("Synthetic test data\n")
    });

    test.select_named("main.rs");
    wait_until("the source", || {
        pane.preview_text()
            .is_some_and(|text| text.starts_with("fn main() {"))
    });

    test.select_named("server.log");
    wait_until("the log's beginning", || pane.preview_text().is_some());
    let shown = pane.preview_text().expect("text");
    assert!(log.starts_with(&shown), "the beginning, unchanged");
    assert!(
        shown.len() <= 1024 * 1024 && shown.len() > 1000 * 1024,
        "{}",
        shown.len()
    );

    for name in ["setup.exe", "Bundle.zip", "Documents"] {
        test.select_named(name);
        wait_until(name, || {
            pane.preview_message().as_deref() == Some("No preview available.")
        });
    }

    test.window.folder_model().select_all();
    wait_until("several items", || {
        pane.preview_message().as_deref() == Some("No preview available.")
    });
}

/// A picture is shown whole, as large as the pane allows and with its
/// shape, without widening the pane; a quick run through several files
/// ends on the last one's contents.
///
/// parity: VIEW-070
#[gtk::test]
fn the_preview_pane_fits_a_picture_and_follows_the_last_selection() {
    let fixture = Fixture::with_files(12);
    let picture = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 2000, 1000)
        .expect("a picture");
    picture.fill(0x3366_99ff);
    picture.savev(fixture.path("Wide.png"), "png", &[]).expect("png");
    let test = preview_window(&fixture);
    let pane = test.window.imp().details_pane.get();
    let width = pane.width();

    test.select_named("Wide.png");
    wait_until("the picture", || pane.preview_picture_size().is_some());
    let (shown_width, shown_height) = pane.preview_picture_size().expect("a picture");
    assert_eq!(shown_width, shown_height * 2, "its shape");
    assert!(shown_width < 2000, "decoded to the pane's size, not its own");
    wait_for_frames(&test.window, 2);
    assert_eq!(pane.width(), width, "the pane keeps its width");

    for number in 0..12 {
        test.select_named(&format!("file {number:04}.txt"));
    }
    std::fs::write(fixture.path("file 0011.txt"), b"last\n").expect("last");
    test.select_named("Wide.png");
    test.select_named("file 0011.txt");
    wait_until("the last file's text", || {
        pane.preview_text().as_deref() == Some("last\n")
    });
}

/// Audio keeps the player the pane showed before, in the Details
/// column.
///
/// parity: VIEW-070
#[gtk::test]
fn the_preview_pane_keeps_the_player_for_audio() {
    let fixture = Fixture::empty();
    fixture.write("Song.mp3");
    let test = preview_window(&fixture);
    let pane = test.window.imp().details_pane.get();
    test.select_named("Song.mp3");
    wait_until("the column", || pane.shows_details_column());
    assert_eq!(pane.preview_message(), None);
}
