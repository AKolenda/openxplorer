// SPDX-License-Identifier: AGPL-3.0-only
//! Office documents in the Preview pane and the views (VIEW-070,
//! VIEW-057): Word, Excel and `PowerPoint` files and `OpenDocuments` shown
//! page by page as `LibreOffice` draws them in its sandbox, as Windows'
//! Preview pane shows them through Office, and their first page as the
//! item's thumbnail; without `LibreOffice`, "No preview available." under
//! the document's icon. Every document is written by the test itself
//! ([`office_documents`]). The tests with `LibreOffice` run where it,
//! poppler's `pdftoppm` and a working bubblewrap are installed.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::geometry::laid_out_with_details_pane;
use crate::test_support::harness::{settle, wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::office_documents::{self, Kind};
use crate::thumbnails::documents::{self, Converter};

/// The converter the tests use until it is dropped, when none is used
/// again.
struct UsedConverter;

impl UsedConverter {
    fn new(converter: Option<Arc<Converter>>) -> Self {
        documents::use_converter(converter);
        Self
    }
}

impl Drop for UsedConverter {
    fn drop(&mut self) {
        documents::use_converter(None);
    }
}

/// A window on `fixture` with the Preview pane open.
fn preview_window(fixture: &Fixture) -> TestWindow {
    let test = laid_out_with_details_pane(&fixture.uri());
    test.activate("preview-pane", None);
    wait_for_frames(&test.window, 2);
    test
}

/// Turns the views' previews off, so only the Preview pane converts
/// documents and its times are its own.
fn turn_view_previews_off(test: &TestWindow) {
    let mut options = test.context.settings_data().preferences.view_options;
    options.show_previews = false;
    let update = ox_core::settings::PreferencesUpdate {
        view_options: Some(options),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    test.context
        .update_preferences(update, |result| result.expect("saved"));
    wait_until("previews to be off", || {
        !test
            .context
            .settings_data()
            .preferences
            .view_options
            .show_previews
    });
}

/// The name of the test document of `kind` called `stem`.
fn name_of(stem: &str, kind: Kind) -> String {
    format!("{stem}.{}", kind.extension())
}

/// The names in `folder`, sorted.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// With `LibreOffice`, each kind of document shows its first page in the
/// Preview pane; a long one shows its pages one under the other, to
/// scroll through; selecting a document again shows it at once from the
/// cache; a damaged one says "No preview available." under its icon.
/// The time to the first page is measured for a small and a large
/// document, and nothing is ever written beside them.
///
/// parity: VIEW-070
#[gtk::test]
fn with_libreoffice_documents_show_their_pages_and_again_at_once() {
    let cache = tempfile::tempdir().expect("a cache");
    let Some(converter) = documents::test_office(cache.path()) else {
        return;
    };
    let _converter = UsedConverter::new(Some(converter));
    let fixture = Fixture::empty();
    for kind in Kind::ALL {
        office_documents::write(
            &fixture.path(&name_of("Quarterly plan", kind)),
            kind,
            &format!("Quarterly plan, {}", kind.extension()),
        );
    }
    office_documents::write_long(&fixture.path("Annual report.docx"), Kind::Docx, 600);
    office_documents::write_long(&fixture.path("Ledger.xlsx"), Kind::Xlsx, 3000);
    office_documents::write_long(&fixture.path("All hands.pptx"), Kind::Pptx, 120);
    office_documents::write_damaged(&fixture.path("Cut off.docx"));
    let before = names_in(fixture.root());
    let test = preview_window(&fixture);
    turn_view_previews_off(&test);
    let pane = test.window.imp().details_pane.get();
    let shows_pages = |count: usize| pane.preview_page_count() >= count;

    for kind in Kind::ALL {
        let name = name_of("Quarterly plan", kind);
        let start = Instant::now();
        test.select_named(&name);
        wait_until(&format!("{name}'s first page"), || shows_pages(1));
        eprintln!("first preview of the small {name}: {:?}", start.elapsed());
        if kind == Kind::Docx {
            crate::test_support::harness::capture_in_both_themes(&test.window, "document-preview-docx");
        }
    }
    for name in ["Annual report.docx", "Ledger.xlsx", "All hands.pptx"] {
        let start = Instant::now();
        test.select_named(name);
        wait_until(&format!("{name}'s first page"), || shows_pages(1));
        let first = start.elapsed();
        wait_until(&format!("{name}'s next pages"), || shows_pages(5));
        eprintln!(
            "first preview of the large {name}: {first:?}, five pages in {:?}",
            start.elapsed()
        );
        if name == "All hands.pptx" {
            crate::test_support::harness::capture_in_both_themes(&test.window, "document-preview-pptx");
        }
    }
    for kind in [Kind::Docx, Kind::Ods] {
        let name = name_of("Quarterly plan", kind);
        test.select_named("Annual report.docx");
        wait_until("the report", || shows_pages(1));
        let start = Instant::now();
        test.select_named(&name);
        wait_until(&format!("{name} again"), || shows_pages(1));
        let again = start.elapsed();
        eprintln!("{name} selected again: {again:?}");
        assert!(again < Duration::from_millis(400), "{name} again took {again:?}");
    }
    test.select_named("Cut off.docx");
    wait_until("the damaged document's message", || {
        pane.preview_message().as_deref() == Some("No preview available.")
            && pane.preview_message_icon().is_some()
    });
    assert_eq!(
        names_in(fixture.root()),
        before,
        "nothing was written beside them"
    );
}

/// In Large icons, a document's first page is its thumbnail, written into
/// the shared thumbnail cache as the Thumbnail Managing Standard asks;
/// without `LibreOffice` it keeps its icon and the cache gets nothing.
///
/// parity: VIEW-057
#[gtk::test]
fn large_icons_show_a_documents_first_page_from_the_shared_cache() {
    use ox_core::entry::{thumbnail_file, ThumbnailFlavor};

    let cache = tempfile::tempdir().expect("a cache");
    let fixture = Fixture::empty();
    for kind in Kind::ALL {
        office_documents::write(&fixture.path(&name_of("Budget", kind)), kind, "Budget");
    }
    let thumbnails = |flavor| {
        Kind::ALL
            .iter()
            .map(|kind| {
                thumbnail_file(
                    &glib::user_cache_dir(),
                    &fixture.uri_of(&name_of("Budget", *kind)),
                    flavor,
                )
            })
            .collect::<Vec<_>>()
    };
    {
        let _converter = UsedConverter::new(None);
        let test = TestWindow::open(&fixture.uri());
        test.activate("view", Some("large"));
        wait_for(Duration::from_secs(2));
        for flavor in [ThumbnailFlavor::Normal, ThumbnailFlavor::Large] {
            assert!(
                thumbnails(flavor).iter().all(|path| !path.exists()),
                "no thumbnail without LibreOffice"
            );
        }
    }
    let Some(converter) = documents::test_office(cache.path()) else {
        return;
    };
    let _converter = UsedConverter::new(Some(converter));
    // Another folder's URIs, as the first window's run remembers the
    // documents it could not draw.
    let fixture = Fixture::empty();
    for kind in Kind::ALL {
        office_documents::write(&fixture.path(&name_of("Budget", kind)), kind, "Budget");
    }
    let test = TestWindow::open(&fixture.uri());
    let start = Instant::now();
    test.activate("view", Some("large"));
    let made = || {
        [ThumbnailFlavor::Normal, ThumbnailFlavor::Large]
            .into_iter()
            .find(|flavor| {
                Kind::ALL.iter().all(|kind| {
                    thumbnail_file(
                        &glib::user_cache_dir(),
                        &fixture.uri_of(&name_of("Budget", *kind)),
                        *flavor,
                    )
                    .exists()
                })
            })
    };
    let deadline = Instant::now() + Duration::from_secs(40);
    while made().is_none() {
        assert!(Instant::now() < deadline, "the thumbnails were not made");
        wait_for(Duration::from_millis(50));
    }
    eprintln!("six thumbnails made in {:?}", start.elapsed());
    let flavor = made().expect("made");
    for kind in Kind::ALL {
        let uri = fixture.uri_of(&name_of("Budget", kind));
        let path = thumbnail_file(&glib::user_cache_dir(), &uri, flavor);
        let picture = gtk::gdk_pixbuf::Pixbuf::from_file(&path).expect("a PNG");
        assert_eq!(picture.option("tEXt::Thumb::URI").as_deref(), Some(uri.as_str()));
        assert!(picture.option("tEXt::Thumb::MTime").is_some());
        let edge = flavor.pixels();
        assert!(picture.width().max(picture.height()) <= edge, "{kind:?}");
        assert!(picture.width().max(picture.height()) >= edge / 2, "{kind:?}");
    }
    wait_for(Duration::from_millis(500));
    crate::test_support::harness::capture_in_both_themes(&test.window, "document-thumbnails-large-icons");
    drop(test);
    no_office_left_running();
}

/// In Large icons, while a workbook that takes half a minute holds the
/// conversion of thumbnails, the selected document still shows at once:
/// the Preview pane stops that conversion and goes first, rather than
/// waiting out its time limit.
///
/// parity: VIEW-070, VIEW-057
#[gtk::test]
fn the_selection_goes_before_a_slow_thumbnail() {
    let cache = tempfile::tempdir().expect("a cache");
    let Some(converter) = documents::test_office(cache.path()) else {
        return;
    };
    let _converter = UsedConverter::new(Some(converter));
    let fixture = Fixture::empty();
    office_documents::write_slow_spreadsheet(&fixture.path("00 Forecast.xlsx"), 600);
    office_documents::write(&fixture.path("01 Notes.odt"), Kind::Odt, "Notes");
    let test = preview_window(&fixture);
    test.activate("view", Some("large"));
    let pane = test.window.imp().details_pane.get();
    // The workbook's thumbnail is being made.
    wait_for(Duration::from_secs(2));

    let start = Instant::now();
    test.select_named("01 Notes.odt");
    while pane.preview_page_count() == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "the selection waited for the workbook"
        );
        wait_for(Duration::from_millis(20));
    }

    eprintln!(
        "the selection showed in {:?} beside a slow thumbnail",
        start.elapsed()
    );
    assert!(start.elapsed() < documents::TIMEOUT / 2);
    drop(test);
    no_office_left_running();
}

/// Waits until no `LibreOffice` runs for the user, which a closed
/// window's thumbnails must not leave behind for the tests after it.
fn no_office_left_running() {
    let start = Instant::now();
    while running_offices() > 0 {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "LibreOffice was left running"
        );
        wait_for(Duration::from_millis(50));
    }
}

/// The longest the main loop may spend on one event while the selection
/// moves: longer is a freeze the user sees.
const LONGEST_STEP: Duration = Duration::from_millis(250);

/// Runs the main loop for `duration`, and says the longest it spent on
/// one iteration.
fn run_and_time(duration: Duration) -> Duration {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + duration;
    let mut longest = Duration::ZERO;
    while Instant::now() < deadline {
        let start = Instant::now();
        context.iteration(false);
        longest = longest.max(start.elapsed());
        std::thread::sleep(Duration::from_millis(2));
    }
    longest
}

/// Writes the fifty documents the arrowing test goes through: every kind,
/// short and long, with a damaged one, one that needs a password, noise
/// under a document's name, a workbook that takes half a minute and a
/// macro every tenth.
fn write_fifty(fixture: &Fixture) -> Vec<String> {
    let mut names = Vec::new();
    for number in 0..50 {
        let kind = Kind::ALL[number % Kind::ALL.len()];
        let name = format!("Item {number:02}.{}", kind.extension());
        let path = fixture.path(&name);
        match number {
            7 => office_documents::write_damaged(&fixture.path(&format!("Item {number:02}.docx"))),
            13 => office_documents::write_password_protected(&path),
            19 => office_documents::write_noise(&path),
            25 => {
                let name = format!("Item {number:02}.xlsx");
                office_documents::write_slow_spreadsheet(&fixture.path(&name), 600);
                names.push(name);
                continue;
            }
            _ if number % 10 == 3 => {
                let name = format!("Item {number:02}.odt");
                office_documents::write_with_macro(&fixture.path(&name), "A form with a macro");
                names.push(name);
                continue;
            }
            _ if number % 9 == 4 => office_documents::write_long(&path, kind, 200),
            _ => office_documents::write(&path, kind, &format!("Item {number}")),
        }
        names.push(if number == 7 {
            format!("Item {number:02}.docx")
        } else {
            name
        });
    }
    names
}

/// The documents `LibreOffice` still runs for, by the processes of the
/// user's named `soffice.bin`.
fn running_offices() -> usize {
    use std::os::unix::fs::MetadataExt;

    let user = rustix::process::getuid().as_raw();
    fs::read_dir("/proc")
        .map(|listing| {
            listing
                .filter_map(Result::ok)
                .filter(|entry| entry.metadata().is_ok_and(|metadata| metadata.uid() == user))
                .filter(|entry| {
                    fs::read_to_string(entry.path().join("comm"))
                        .is_ok_and(|name| name.trim() == "soffice.bin")
                })
                .count()
        })
        .unwrap_or_default()
}

/// Real keys (the X server's XTEST) move the selection through fifty
/// mixed documents, quickly and then slowly, first with the Preview pane
/// open, then with the Details pane, then in Large icons while every
/// document's thumbnail is made: the window never freezes, the
/// conversion of a document passed is stopped, the one stopped on shows
/// its pages (or "No preview available." for a bad one), no `LibreOffice`
/// is left running, and nothing is written beside the documents.
///
/// parity: VIEW-070
#[gtk::test]
fn arrowing_through_fifty_documents_never_freezes_the_window() {
    let cache = tempfile::tempdir().expect("a cache");
    let Some(converter) = documents::test_office(cache.path()) else {
        return;
    };
    let _converter = UsedConverter::new(Some(converter));
    let fixture = Fixture::empty();
    let names = write_fifty(&fixture);
    assert_eq!(names.len(), 50);
    let before = names_in(fixture.root());
    // Before the window opens, which a new pointer would put away.
    let mut pointer = crate::test_support::pointer::Pointer::connect();
    let test = preview_window(&fixture);
    test.window.set_default_size(1280, 800);
    test.window.present();
    let pane = test.window.imp().details_pane.get();
    test.select_named(&test.names()[0]);
    test.window.folder_pane().focus_view();
    let (x, y) =
        crate::test_support::pointer::screen_point(&mut pointer, &test.window, (400.0, 300.0), (400, 300));
    // The keyboard follows the pointer on the private display.
    pointer.move_to(x, y);
    settle();
    let mut longest = Duration::ZERO;
    let position = || test.window.folder_model().selected_positions();

    // Quickly, as a held key repeats: no document is stopped on long
    // enough to be read.
    for _ in 0..20 {
        pointer.tap_key(gdk::Key::Down);
        longest = longest.max(run_and_time(Duration::from_millis(40)));
    }
    assert_eq!(position(), vec![20], "the keys moved the selection");
    // Slowly, stopping on each document as long as its conversion takes.
    for _ in 20..49 {
        pointer.tap_key(gdk::Key::Down);
        longest = longest.max(run_and_time(Duration::from_millis(450)));
    }
    assert_eq!(position(), vec![49]);
    let last = test.names()[49].clone();
    let shown = |pane: &crate::window::details_pane::DetailsPane| {
        pane.preview_page_count() > 0 || pane.preview_message().as_deref() == Some("No preview available.")
    };
    let start = Instant::now();
    while !shown(&pane) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{last} was never shown"
        );
        longest = longest.max(run_and_time(Duration::from_millis(50)));
    }
    eprintln!("Preview pane: the longest step was {longest:?}");
    assert!(longest < LONGEST_STEP, "the window froze for {longest:?}");

    test.activate("details-pane", None);
    wait_for_frames(&test.window, 2);
    assert!(pane.shows_properties());
    let mut longest = Duration::ZERO;
    for step in 0..49 {
        pointer.tap_key(gdk::Key::Up);
        let wait = if step < 25 { 40 } else { 200 };
        longest = longest.max(run_and_time(Duration::from_millis(wait)));
    }
    assert_eq!(position(), vec![0]);
    eprintln!("Details pane: the longest step was {longest:?}");
    assert!(longest < LONGEST_STEP, "the window froze for {longest:?}");

    // Large icons, where every document's thumbnail is made, while the
    // keys move the selection on with the Preview pane open again.
    test.activate("preview-pane", None);
    test.activate("view", Some("large"));
    test.window.folder_pane().focus_view();
    let mut longest = Duration::ZERO;
    for step in 0..30 {
        pointer.tap_key(gdk::Key::Right);
        let wait = if step % 3 == 0 { 600 } else { 60 };
        longest = longest.max(run_and_time(Duration::from_millis(wait)));
    }
    // The first key after the view changes may only bring the selection
    // into view.
    assert!(position().first().is_some_and(|&at| at >= 25), "{:?}", position());
    longest = longest.max(run_and_time(Duration::from_secs(3)));
    eprintln!("Large icons while thumbnails are made: the longest step was {longest:?}");
    assert!(longest < LONGEST_STEP, "the window froze for {longest:?}");
    pointer.move_to(720, 500);

    let start = Instant::now();
    while running_offices() > 0 {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "LibreOffice was left running"
        );
        wait_for(Duration::from_millis(100));
    }
    assert_eq!(
        names_in(fixture.root()),
        before,
        "nothing was written beside them"
    );
}
