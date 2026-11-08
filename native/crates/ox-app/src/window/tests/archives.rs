// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP archives in a real window: browsing, Extract all…, Extract here and
//! Compress to ZIP file.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, these also save
//! `native-archive-browser.png`, `native-extract-dialog.png` and
//! `native-extract-conflict-{light,dark}.png`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gtk::prelude::*;
use ox_core::archive::{CompressionRequest, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::settings::{PreferencesUpdate, ZipOpening};
use ox_core::transfer::Cancellation;

use super::item_dialogs::{press, texts};
use crate::archive_view::ArchiveBrowserView;
use crate::test_support::harness::{
    capture, capture_dialog, capture_popover, descendants, wait_for, wait_until, Fixture, TestWindow,
    ThemeGuard,
};

/// A standard fixture with `Bundle.zip`, which holds `Docs/a.txt` and
/// `readme.txt`.
pub(super) fn fixture_with_zip() -> Fixture {
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

/// A window on `fixture` that opens ZIPs in the "Compressed folder"
/// window, the choice ZIPs no longer open with by default.
pub(super) fn opening_zips_in_the_window(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    let choose = PreferencesUpdate {
        zip_opening: Some(ZipOpening::Window),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(choose, |result| result.expect("saved"));
    wait_until("the preference", || {
        test.context.settings_data().preferences.zip_opening == ZipOpening::Window
    });
    test
}

fn uri_in(folder: &Path, name: &str) -> String {
    file_uri(&folder.join(name))
}

/// The archive browser inside the dialog shown.
pub(super) fn archive_browser(test: &TestWindow) -> ArchiveBrowserView {
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
    let test = opening_zips_in_the_window(&fixture);
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
        "Opened a temporary copy. Changes are not saved back to the archive."
    );
    assert!(!fixture.path("readme.txt").exists(), "browsing extracts nothing");
}

/// A ZIP the browser refuses (corrupt, too large a directory, too many
/// members) says why in the browser's status line.
///
/// parity: ARC-005
#[gtk::test]
fn a_corrupt_zip_says_why_it_cannot_be_browsed() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Broken.zip"), b"not a zip").expect("fixture file");
    let test = opening_zips_in_the_window(&fixture);
    test.select_named("Broken.zip");

    test.activate("open", None);

    let browser = archive_browser(&test);
    wait_until("the refusal", || !browser.status_text().is_empty());
    assert_eq!(browser.status_text(), "File is not a zip file");
    assert!(browser.row_names().is_empty());
}

/// Opening a folder and going back before it is listed shows the top:
/// the Docs listing, answered after the top's, changes nothing.
///
/// parity: SAFE-013
#[gtk::test]
fn a_late_archive_listing_never_replaces_a_newer_one() {
    let fixture = fixture_with_zip();
    let test = opening_zips_in_the_window(&fixture);
    test.select_named("Bundle.zip");
    test.activate("open", None);
    let browser = archive_browser(&test);
    wait_until("the listing", || !browser.row_names().is_empty());
    let docs = browser.list_now("Docs/");
    assert_eq!(docs.entries.len(), 1, "the late answer has a row to show");

    let docs_listing = browser.show_folder_numbered("Docs/");
    browser.show_folder("");
    wait_until("the top again", || browser.row_names().len() == 2);
    browser.deliver_listing(docs_listing, docs);

    assert_eq!(browser.row_names(), ["Docs", "readme.txt"]);
}

/// Closing the Extract dialog cancels its check of the archive, and an
/// answer that arrives afterwards changes nothing in the closed dialog.
///
/// parity: SAFE-013
#[gtk::test]
fn closing_the_extract_dialog_drops_its_check() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.shown_dialog().expect("the Extract dialog");

    press(&frame, "Cancel");

    wait_for(Duration::from_millis(300));
    assert!(test.shown_dialog().is_none());
    assert!(
        texts(&frame)
            .iter()
            .any(|text| text == "Checking archive contents…"),
        "{:?}",
        texts(&frame)
    );
}

/// parity: ARC-009, ARC-011, ARC-012
#[gtk::test]
fn extract_all_unpacks_into_a_new_folder_and_shows_it() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");

    test.activate("extract-all", None);

    let frame = test.wait_for_dialog("the Extract dialog");
    assert_eq!(frame.title(), "Extract Bundle.zip");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    assert!(texts(&frame)
        .iter()
        .any(|text| text == "2 files · 1 folder · 12 bytes unpacked"));
    assert_eq!(
        frame.button_labels(),
        ["Cancel", "Extract"],
        "as short as Explorer's; the rest waits behind the (i) button"
    );
    let info = descendants::<gtk::MenuButton>(&frame)
        .into_iter()
        .find(|button| button.has_css_class("extract-info"))
        .expect("the (i) button");
    let bubble = info.popover().expect("its bubble");
    assert!(!bubble.is_visible(), "the counts and notes stay out of the way");
    assert!(descendants::<gtk::Button>(&bubble)
        .iter()
        .any(|button| button.label().as_deref() == Some("Open in archive manager")));
    info.popup();
    wait_until("the bubble", || bubble.is_visible());
    capture_popover(&test.window, &bubble, "native-extract-info.png");
    info.popdown();
    let fields = descendants::<gtk::Entry>(&frame);
    assert_eq!(fields.len(), 1, "one folder field, as in Explorer");
    assert_eq!(fields[0].text(), format!("{}/Bundle", fixture.root().display()));
    assert!(texts(&frame)
        .iter()
        .any(|text| text == "Files will be extracted to this folder"));
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
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// Marks every member of the ZIP at `path` as encrypted, in its local
/// header and in the central directory (general purpose flag bit 0).
pub(super) fn mark_encrypted(path: &Path) {
    let mut bytes = fs::read(path).expect("the ZIP");
    for at in 0..bytes.len().saturating_sub(10) {
        let flags = match &bytes[at..at + 4] {
            b"PK\x03\x04" => at + 6,
            b"PK\x01\x02" => at + 8,
            _ => continue,
        };
        bytes[flags] |= 1;
    }
    fs::write(path, bytes).expect("the ZIP rewritten");
}

/// A password-protected ZIP says so under the field at once, offers the
/// archive manager in the footer, and Extract repeats the reason.
///
/// parity: ARC-009
#[gtk::test]
fn a_password_zip_says_so_and_offers_the_archive_manager() {
    let fixture = fixture_with_zip();
    mark_encrypted(&fixture.path("Bundle.zip"));
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");

    wait_until("the refusal", || frame.error_text().contains("password"));
    assert_eq!(
        frame.error_text(),
        "This ZIP has a password, so OpenXplorer cannot extract it. Open it in the archive manager instead."
    );
    let fallback = descendants::<gtk::Button>(&frame)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Open in archive manager") && button.is_visible())
        .expect("the archive manager in the footer");
    press(&frame, "Extract");
    assert!(frame.error_text().contains("password"));
    assert_eq!(test.shown_dialog().as_ref(), Some(&frame));

    fallback.emit_clicked();
    wait_until("the launch", || !test.context.recorded_launches().is_empty());
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Bundle.zip")]);
}

/// parity: ARC-010, OPS-006
#[gtk::test]
fn extract_refuses_an_empty_folder_and_keeps_the_dialog_open() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    let fields = descendants::<gtk::Entry>(&frame);

    fields[0].set_text("  ");
    press(&frame, "Extract");

    assert_eq!(frame.error_text(), "Enter the folder to extract to.");
    assert_eq!(test.shown_dialog(), Some(frame));
}

/// The folder field as in Explorer: deleting the archive's name extracts
/// straight into the existing folder, and a file already there is
/// replaced only after asking; a missing folder is created with its
/// parents, and a lone folder of the same name is not nested.
///
/// parity: ARC-009, ARC-011, ARC-012
#[gtk::test]
fn extract_all_goes_straight_into_an_existing_folder_after_asking() {
    let fixture = fixture_with_zip();
    fs::write(fixture.path("readme.txt"), b"mine").expect("a file the archive also has");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    descendants::<gtk::Entry>(&frame)[0].set_text(&fixture.root().display().to_string());
    press(&frame, "Extract");

    super::file_ops_support::answer_conflicts(&test, super::file_ops_support::ConflictAnswer::Skip);

    wait_until("the extracted folder", || fixture.path("Docs/a.txt").exists());
    super::file_ops_support::assert_ended_without_a_message(&test);
    assert!(
        super::file_ops_support::dialog_over(&test).is_none(),
        "nothing reports the name Skip left alone, as in Windows"
    );
    assert_eq!(
        fs::read(fixture.path("readme.txt")).expect("kept"),
        b"mine",
        "skipped, not replaced"
    );
    // The private folder is removed once the move ends, which can come
    // just after the report on a busy machine.
    wait_until("the private folder to be removed", || {
        fs::read_dir(fixture.root())
            .expect("the folder")
            .filter_map(Result::ok)
            .all(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".openxplorer-extract-")
            })
    });
    let undo = test.window.lookup_action("undo").expect("the Undo action");
    assert!(
        !undo.is_enabled(),
        "Undo cannot move the files back into the removed private folder"
    );
    assert!(
        !fixture.path("Bundle").exists(),
        "no folder named after the archive"
    );
}

/// Cancel in the name-conflict question of an extraction into an existing
/// folder just stops, as in Windows Explorer: no message follows, the
/// file it asked about is unchanged and the private folder is removed.
/// As Windows asks at the end of the job, the items without a conflict
/// were already extracted, and stay.
///
/// parity: ARC-011, ARC-012
#[gtk::test]
fn cancelling_an_extractions_conflict_question_says_nothing() {
    let fixture = fixture_with_zip();
    fs::write(fixture.path("readme.txt"), b"mine").expect("a file the archive also has");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    descendants::<gtk::Entry>(&frame)[0].set_text(&fixture.root().display().to_string());
    press(&frame, "Extract");

    super::file_ops_support::cancel_conflicts(&test);
    wait_until("the private folder to be removed", || {
        fs::read_dir(fixture.root())
            .expect("the folder")
            .filter_map(Result::ok)
            .all(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".openxplorer-extract-")
            })
    });
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));

    assert!(super::file_ops_support::dialog_over(&test).is_none());
    assert_eq!(test.window.shown_message(), "", "nothing is said after Cancel");
    assert!(
        fixture.path("Docs/a.txt").is_file(),
        "the items without a conflict came first"
    );
    assert_eq!(fs::read(fixture.path("readme.txt")).expect("kept"), b"mine");
}

/// An extraction into an existing folder that takes in every item counts
/// the extracted files, as one into a new folder does.
///
/// parity: ARC-009, ARC-011, ARC-012
#[gtk::test]
fn extract_all_into_an_existing_folder_counts_its_files_when_nothing_is_left_out() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    descendants::<gtk::Entry>(&frame)[0].set_text(&fixture.path("Documents").display().to_string());
    press(&frame, "Extract");

    wait_until("the extracted files", || {
        fixture.path("Documents/readme.txt").exists()
    });
    assert!(fixture.path("Documents/Docs/a.txt").exists());
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// parity: ARC-009
#[gtk::test]
fn extract_all_creates_a_missing_folder_with_its_parents() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    let target = fixture.path("Unpacked/2026");
    descendants::<gtk::Entry>(&frame)[0].set_text(&target.display().to_string());
    press(&frame, "Extract");

    wait_until("the extracted folder", || {
        test.window.current_uri() == Some(file_uri(&target))
    });
    assert_eq!(
        fs::read(target.join("readme.txt")).expect("extracted"),
        b"read me"
    );
}

/// Enter in the folder field extracts, as Explorer's dialog does.
///
/// parity: ARC-009
#[gtk::test]
fn enter_in_the_folder_field_extracts() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });

    descendants::<gtk::Entry>(&frame)[0].emit_activate();

    let extracted = fixture.path("Bundle");
    wait_until("the extracted folder", || {
        test.window.current_uri() == Some(file_uri(&extracted))
    });
    assert!(test.shown_dialog().is_none());
    assert_eq!(
        fs::read(extracted.join("readme.txt")).expect("extracted"),
        b"read me"
    );
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
    super::file_ops_support::assert_ended_without_a_message(&test);
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

/// Extract all… checks the window before it looks at the destination,
/// so an operation such as Duplicate may claim the transfer panel in that
/// gap. The extraction then must not take the panel over: its Cancel
/// would stop the wrong work, and whichever ends first would hide the
/// panel and let the window close on the other with no warning.
///
/// parity: OPS-024
#[gtk::test]
fn extract_all_never_takes_the_panel_from_an_operation_started_meanwhile() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });

    press(&frame, "Extract");
    let running = test
        .window
        .begin_operation("Preparing copy…")
        .expect("the extraction has not claimed the window yet");
    wait_until("the extraction to end", || {
        test.shown_dialog().is_some() || fixture.path("Bundle/readme.txt").exists()
    });

    assert!(!fixture.path("Bundle").exists(), "nothing extracted beside it");
    let refusal = test.wait_for_dialog("why the extraction stopped");
    assert!(texts(&refusal)
        .iter()
        .any(|text| text == "Finish the current file operation before extracting."));
    assert!(
        test.window.transfer_panel().is_busy(),
        "the panel stays the operation's"
    );
    test.window.transfer_panel().cancel();
    assert!(
        running.cancel.is_cancelled(),
        "Cancel stops the operation that claimed the panel"
    );
    assert!(test.window.is_writing_files(), "closing still warns");
    test.window.end_operation();
}

/// Writes `Site.tar.gz` holding `Docs/a.txt` into `fixture`, with
/// Python's `tarfile`.
fn write_tar_gz(fixture: &Fixture) {
    let script = "import io, sys, tarfile\n\
                  with tarfile.open(sys.argv[1], 'w:gz') as archive:\n\
                  \x20   member = tarfile.TarInfo('Docs/a.txt'); member.size = 5\n\
                  \x20   archive.addfile(member, io.BytesIO(b'first'))\n";
    let status = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(fixture.path("Site.tar.gz"))
        .status()
        .expect("Python 3 writes the fixture archive");
    assert!(status.success());
}

/// A .tar.gz opens in the archive browser window when archives open in
/// a pop-up window, which calls it an archive, not a ZIP, and extracts
/// like a ZIP; with "Open archives as folders" off it opens in its
/// default application, unless it was handed to the app.
///
/// parity: ARC-022, ARC-024
#[gtk::test]
fn a_compressed_tar_is_browsed_and_extracted_unless_archives_open_elsewhere() {
    let fixture = Fixture::standard();
    write_tar_gz(&fixture);
    let test = opening_zips_in_the_window(&fixture);
    test.select_named("Site.tar.gz");

    test.activate("open", None);
    let browser = archive_browser(&test);
    wait_until("the listing", || browser.row_names() == ["Docs"]);
    let shown = texts(&test.shown_dialog().expect("the browser"));
    assert!(
        shown.iter().any(|text| text.starts_with("Read-only archive.")),
        "{shown:?}"
    );
    assert!(!shown.iter().any(|text| text.contains("ZIP")), "{shown:?}");
    test.shown_dialog().expect("the browser").close();
    test.activate("extract-here", None);
    // Its lone top-level folder is lifted out rather than nested (ARC-025).
    let extracted = fixture.path("Docs/a.txt");
    wait_until("the extracted file", || extracted.exists());
    assert_eq!(fs::read(&extracted).expect("extracted"), b"first");

    let turn_off = ox_core::settings::PreferencesUpdate {
        browse_archives: Some(false),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    test.context
        .update_preferences(turn_off, |result| result.expect("saved"));
    wait_until("the preference", || {
        !test.context.settings_data().preferences.browse_archives
    });
    test.select_named("Site.tar.gz");
    test.activate("open", None);
    wait_until("the default application", || {
        !test.context.recorded_launches().is_empty()
    });
    assert!(test.context.recorded_launches()[0].contains("Site.tar.gz"));

    // An archive handed to the app, which may be its default application,
    // is browsed whatever the setting, so it never launches itself again.
    test.window.open_locations(vec![fixture.uri_of("Site.tar.gz")]);
    let browser = archive_browser(&test);
    wait_until("the listing", || browser.row_names() == ["Docs"]);
    assert_eq!(test.context.recorded_launches().len(), 1);
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
    test.wait_for_listing("the folder listed again");
    wait_until("the ZIP in the listing", || {
        test.names().contains(&"Documents.zip".to_owned())
    });
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// Compress to… asks for the name and format, writes a .tar.xz, and
/// keeps the dialog open with the reason when the name is taken.
///
/// parity: ARC-023
#[gtk::test]
fn compress_to_asks_for_a_name_and_a_format() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let compress = |name: &str| {
        test.select_named("Notes 2.txt");
        test.activate("compress-to", None);
        let frame = test.wait_for_dialog("the Compress dialog");
        assert_eq!(descendants::<gtk::Entry>(&frame)[0].text(), "Notes 2");
        descendants::<gtk::Entry>(&frame)[0].set_text(name);
        descendants::<gtk::DropDown>(&frame)[0].set_selected(1);
        press(&frame, "Compress");
        frame
    };

    compress("Backup");
    let archive = fixture.path("Backup.tar.xz");
    wait_until("the new archive", || archive.exists());
    super::file_ops_support::assert_ended_without_a_message(&test);
    let frame = compress("Backup");
    wait_until("the refusal", || !frame.error_text().is_empty());
    assert_eq!(test.shown_dialog(), Some(frame));
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
    let test = opening_zips_in_the_window(&fixture);
    test.select_named("Bundle.zip");
    test.activate("open", None);
    let frame = test.wait_for_dialog("the archive browser");

    press(&frame, "Open in archive manager");

    wait_until("the launch", || !test.context.recorded_launches().is_empty());
    assert_eq!(test.context.recorded_launches(), [fixture.uri_of("Bundle.zip")]);
    assert!(test.shown_dialog().is_none());
}

/// Extracting onto files that already exist asks Replace or Skip under a
/// line that says what really happens: extracting from the archive into
/// the chosen folder, never a move out of the private folder the
/// extraction is built in.
///
/// parity: ARC-012, OPS-019
#[gtk::test]
fn extracting_onto_existing_files_names_the_archive() {
    let fixture = fixture_with_zip();
    fs::write(fixture.path("readme.txt"), b"mine").expect("a file the archive also has");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Bundle.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        texts(&frame).iter().any(|text| text.ends_with("unpacked"))
    });
    descendants::<gtk::Entry>(&frame)[0].set_text(&fixture.root().display().to_string());
    press(&frame, "Extract");

    let window = super::file_ops_support::asking_window(&test);
    let shown = texts(&window);
    let folder = fixture
        .root()
        .file_name()
        .expect("a named folder")
        .to_string_lossy()
        .into_owned();
    assert!(
        shown.contains(&format!("Extracting 2 items from Bundle.zip to {folder}")),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|text| text.contains(".openxplorer-extract-")),
        "{shown:?}"
    );
    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        capture_dialog(&window, &format!("native-extract-conflict-{theme}.png"));
    }
    super::file_ops_support::answer_conflicts(&test, super::file_ops_support::ConflictAnswer::Skip);
    wait_until("the extracted folder", || fixture.path("Docs/a.txt").exists());
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// Compress to › TAR File puts one file, several files or a folder into a
/// new plain .tar beside them, named after the first item, which then
/// opens in the tab like any archive.
///
/// parity: ARC-023, ARC-026
#[gtk::test]
fn compress_to_tar_puts_the_selection_into_a_new_tar_beside_it() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Second.txt"), b"second").expect("a second file");
    let test = TestWindow::open(&fixture.uri());
    let choose = PreferencesUpdate {
        zip_opening: Some(ZipOpening::Folder),
        ..PreferencesUpdate::default()
    };
    test.context
        .update_preferences(choose, |result| result.expect("saved"));
    let cases: [(&[&str], &str, &[&str]); 3] = [
        (&["Notes 2.txt"], "Notes 2.tar", &["Notes 2.txt"]),
        (
            &["Notes 2.txt", "Second.txt"],
            "Notes 2 (2).tar",
            &["Notes 2.txt", "Second.txt"],
        ),
        (&["Documents"], "Documents.tar", &["Documents"]),
    ];
    for (selected, archive, inside) in cases {
        test.wait_for_listing("the folder");
        super::file_ops_support::select_names(&test, selected);
        test.activate("compress-to-tar", None);
        wait_until("the new TAR", || fixture.path(archive).exists());
        super::file_ops_support::assert_ended_without_a_message(&test);
        let bytes = fs::read(fixture.path(archive)).expect("written");
        assert_eq!(bytes.get(257..262), Some(b"ustar".as_slice()), "{archive}");
        wait_until("the TAR in the listing", || {
            test.names().contains(&(*archive).to_owned())
        });

        let root = ox_core::location::ArchiveLocation::root(&fixture.uri_of(archive));
        test.select_named(archive);
        test.activate("open", None);
        wait_until("the TAR in the tab", || {
            test.window.current_uri() == Some(root.uri())
        });
        test.wait_for_listing("the TAR's listing");
        assert_eq!(test.names(), inside, "{archive}");
        test.activate("up", None);
        wait_until("out of the TAR", || {
            test.window.current_uri() == Some(fixture.uri())
        });
    }
}
