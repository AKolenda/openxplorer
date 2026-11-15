// SPDX-License-Identifier: AGPL-3.0-only
//! Password-protected ZIPs in a real window, as Windows 11 File Explorer
//! handles them: the ZIP is browsed without a password, and opening a
//! file, Extract all, Extract here and Copy ask "Password needed"; a wrong
//! password asks again with the reason, Cancel says nothing, and the
//! password that worked is not asked again while the window is open.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set, this also saves
//! `native-zip-password-{light,dark}.png` and
//! `native-zip-password-wrong-{light,dark}.png`.

use std::fs;

use gtk::prelude::*;
use ox_core::location::{file_uri, ArchiveLocation};

use super::file_ops_support::{dialog_over, open_dialog, wait_for_no_dialog};
use super::item_dialogs::press;
use crate::dialog::Dialog;
use crate::test_support::harness::{
    capture_in_both_themes, descendants, settle, wait_until, Fixture, TestWindow,
};

/// The password of `Harbor.zip`.
const PASSWORD: &str = "lighthouse";

/// `Harbor/`, `Harbor/tides.txt` (deflated) and `Harbor/buoys.txt`
/// (stored), both encrypted with "lighthouse" by Info-ZIP's `zip -P`.
const HARBOR_ZIP: &[&str] = &[
    "504b03040a000000000000603c5d00000000000000000000000007000000486172626f722f504b030414000900080000",
    "603c5daf04387b3a0000002f00000010000000486172626f722f74696465732e747874d8daa2a75707a61c3ae8c85451",
    "16b5a380e86c35a76e80ef5aee77d70513decb90524e7f0e62e6bbdbfe07d7e545e921841e74009873302e2a24504b07",
    "08af04387b3a0000002f000000504b03040a000900000000603c5d88babb53160000000a00000010000000486172626f",
    "722f62756f79732e74787431b01f189b41b3f495b677a07a9758107dc9280fc721504b070888babb53160000000a0000",
    "00504b01021e030a000000000000603c5d000000000000000000000000070000000000000000001000ed410000000048",
    "6172626f722f504b01021e0314000900080000603c5daf04387b3a0000002f000000100000000000000001000000a481",
    "25000000486172626f722f74696465732e747874504b01021e030a000900000000603c5d88babb53160000000a000000",
    "100000000000000001000000a4819d000000486172626f722f62756f79732e747874504b05060000000003000300b100",
    "0000f10000000000",
];

/// A standard fixture with the password-protected `Harbor.zip`.
fn fixture_with_password_zip() -> Fixture {
    let fixture = Fixture::standard();
    let text: String = HARBOR_ZIP.concat();
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|start| u8::from_str_radix(&text[start..start + 2], 16).expect("hex digits"))
        .collect();
    fs::write(fixture.path("Harbor.zip"), bytes).expect("fixture ZIP");
    fixture
}

/// Opens `Harbor.zip` and its `Harbor` folder in the tab.
fn open_harbor(test: &TestWindow, fixture: &Fixture) {
    test.select_named("Harbor.zip");
    test.activate("open", None);
    let folder = ArchiveLocation::root(&fixture.uri_of("Harbor.zip")).member("Harbor/");
    wait_until("the ZIP in the tab", || {
        test.window
            .current_uri()
            .is_some_and(|uri| uri.contains("Harbor.zip"))
    });
    test.wait_for_listing("the ZIP's listing");
    test.window.navigate(&folder.uri()).expect("the folder inside");
    test.wait_for_listing("the folder inside the ZIP");
}

/// The password question once it shows over `test`.
fn password_question(test: &TestWindow) -> Dialog {
    let dialog = open_dialog(test);
    assert_eq!(dialog.title_text(), "Password needed");
    dialog
}

/// Types `password` into `dialog`'s field and presses OK.
fn answer(dialog: &Dialog, password: &str) {
    let field = descendants::<gtk::PasswordEntry>(dialog)
        .into_iter()
        .next()
        .expect("a password field");
    field.set_text(password);
    dialog.press("OK");
}

/// Opening a file inside a password-protected ZIP asks for the password;
/// a wrong one asks again and says so; the right one opens the private
/// copy, and a second file opens without asking again.
///
/// parity: ARC-006, ARC-026
#[gtk::test]
fn opening_a_file_in_a_password_zip_asks_for_the_password() {
    let fixture = fixture_with_password_zip();
    let test = TestWindow::open(&fixture.uri());
    open_harbor(&test, &fixture);
    assert_eq!(
        test.names(),
        ["buoys.txt", "tides.txt"],
        "listed without a password"
    );

    test.select_named("tides.txt");
    test.activate("open", None);
    let dialog = password_question(&test);
    assert!(
        dialog
            .message_text()
            .contains("“Harbor.zip” is password protected"),
        "{}",
        dialog.message_text()
    );
    assert_eq!(dialog.button_labels(), ["Cancel", "OK"]);
    assert_eq!(dialog.error_text(), None);
    capture_in_both_themes(&dialog, "native-zip-password");
    answer(&dialog, "lighthouses");
    wait_for_no_dialog(&test);

    let again = password_question(&test);
    assert_eq!(
        again.error_text().as_deref(),
        Some("The password is incorrect. Try again.")
    );
    capture_in_both_themes(&again, "native-zip-password-wrong");
    assert!(test.context.recorded_launches().is_empty());
    answer(&again, PASSWORD);
    wait_until("the copy to open", || {
        !test.context.recorded_launches().is_empty()
    });
    let copy = gtk::gio::File::for_uri(&test.context.recorded_launches()[0])
        .path()
        .expect("a local copy");
    assert_eq!(
        fs::read_to_string(copy).expect("the copy"),
        "Tide table for the fictional Tidewater harbor.\n"
    );

    test.select_named("buoys.txt");
    test.activate("open", None);
    wait_until("the second copy to open", || {
        test.context.recorded_launches().len() == 2
    });
    assert!(dialog_over(&test).is_none(), "the password is not asked again");
}

/// Cancel in the password question stops without a message, as in
/// Windows Explorer, and opens nothing.
///
/// parity: ARC-006, ARC-026
#[gtk::test]
fn cancelling_the_password_question_says_nothing() {
    let fixture = fixture_with_password_zip();
    let test = TestWindow::open(&fixture.uri());
    open_harbor(&test, &fixture);

    test.select_named("tides.txt");
    test.activate("open", None);
    password_question(&test).press("Cancel");
    wait_for_no_dialog(&test);
    settle();

    assert_eq!(test.window.shown_message(), "");
    assert!(test.context.recorded_launches().is_empty());
}

/// Extract all checks a password-protected ZIP without a password, then
/// asks for it when Extract is pressed; a wrong one writes nothing.
///
/// parity: ARC-009, ARC-011, ARC-016
#[gtk::test]
fn extract_all_asks_for_the_password_and_writes_nothing_until_it_is_right() {
    let fixture = fixture_with_password_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Harbor.zip");
    test.activate("extract-all", None);
    let frame = test.wait_for_dialog("the Extract dialog");
    wait_until("the check", || {
        super::item_dialogs::texts(&frame)
            .iter()
            .any(|text| text.ends_with("unpacked"))
    });
    assert_eq!(frame.error_text(), "", "the check passes without a password");
    press(&frame, "Extract");

    answer(&password_question(&test), "harbor");
    wait_for_no_dialog(&test);
    let again = password_question(&test);
    assert!(
        !fixture.path("Harbor").exists(),
        "nothing written for a wrong password"
    );
    answer(&again, PASSWORD);

    let extracted = fixture.path("Harbor");
    wait_until("the extracted folder", || {
        test.window.current_uri() == Some(file_uri(&extracted))
    });
    assert_eq!(
        fs::read_to_string(extracted.join("buoys.txt")).expect("extracted"),
        "Buoy list\n"
    );
}

/// Extract here asks for the password too; Cancel leaves nothing beside
/// the ZIP and says nothing.
///
/// parity: ARC-025, ARC-016
#[gtk::test]
fn extract_here_asks_for_the_password_and_cancel_leaves_nothing() {
    let fixture = fixture_with_password_zip();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Harbor.zip");
    let before = test.names();

    test.activate("extract-here", None);
    password_question(&test).press("Cancel");
    wait_until("the operation to end", || !test.window.is_writing_files());
    settle();
    assert_eq!(test.names(), before);
    assert_eq!(test.window.shown_message(), "");

    test.select_named("Harbor.zip");
    test.activate("extract-here", None);
    answer(&password_question(&test), PASSWORD);
    let extracted = fixture.path("Harbor");
    wait_until("the extracted folder", || extracted.join("tides.txt").is_file());
}

/// Copy inside a password-protected ZIP asks for the password before the
/// copies go on the clipboard.
///
/// parity: ARC-026
#[gtk::test]
fn copying_out_of_a_password_zip_asks_for_the_password() {
    let fixture = fixture_with_password_zip();
    let test = TestWindow::open(&fixture.uri());
    open_harbor(&test, &fixture);
    test.select_named("buoys.txt");

    test.activate("copy", None);
    answer(&password_question(&test), PASSWORD);
    wait_until("the copy on the clipboard", || {
        !test.window.is_writing_files()
            && test.window.shown_message().is_empty()
            && clipboard_has_files(&test)
    });
}

/// True when the clipboard offers a file list.
fn clipboard_has_files(test: &TestWindow) -> bool {
    test.window
        .clipboard()
        .formats()
        .contain_mime_type("text/uri-list")
}
