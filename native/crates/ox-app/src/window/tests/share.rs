// SPDX-License-Identifier: AGPL-3.0-only
//! Share in the command bar, as Windows 11 has it between Rename and
//! Delete: it emails the selected files through the desktop's Email
//! portal, and is hidden where the desktop has none. A fake portal on a
//! second connection to the test's private session bus stands in for the
//! desktop's, under its unique name, so no real portal or email app is
//! reached.

use gtk::prelude::*;

use super::command_bar::command_bar_controls;
use super::file_ops_support::select_names;
use super::geometry::{button_for, laid_out};
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::portal::{EmailPortal, SettingsPortal};

/// Points `test`'s window at the portal called `portal` and waits until
/// it knows whether that portal can share.
fn use_portal(test: &TestWindow, portal: &str) {
    test.window.follow_share_service(portal);
    wait_until("the window to ask the portal", || {
        test.window.share_service_checked(portal)
    });
    wait_for_frames(&test.window, 2);
}

/// The Share button of `test`'s command bar.
fn share_button(test: &TestWindow) -> gtk::Button {
    button_for(test, "win.share")
}

/// Share shows, as in Windows 11, also where no Email portal can take
/// files: one without Email, nobody answering, or Email version 1, which
/// takes no attachments. A click then says why, and asks no portal.
///
/// parity: CMD-037
#[gtk::test]
fn share_shows_without_an_email_portal_and_says_why() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let button = share_button(&test);
    assert_eq!(button.tooltip_text().as_deref(), Some("Share"));
    let settings_only = SettingsPortal::serving(&[]);
    let first = EmailPortal::of_version(1, 0);
    for portal in [
        settings_only.name(),
        crate::window::share::NO_PORTAL.to_owned(),
        first.name(),
    ] {
        use_portal(&test, &portal);
        assert!(button.is_visible(), "{portal}: Share shows");
    }
    select_names(&test, &["Notes 2.txt"]);
    assert!(button.is_sensitive(), "a file can be shared");

    button.emit_clicked();
    let message = test.wait_for_dialog("the reason");
    assert_eq!(message.title(), "Share");
    assert!(first.calls().is_empty(), "version 1 is not asked");
}

/// parity: CMD-037
#[gtk::test]
fn with_an_email_portal_share_attaches_the_selected_files() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let portal = EmailPortal::answering(0);
    use_portal(&test, &portal.name());
    let button = share_button(&test);
    assert!(button.is_visible(), "the desktop can share by email");
    assert!(!button.is_sensitive(), "nothing is selected");
    // The neighbours by their actions, whatever Delete is labelled.
    let actions: Vec<Option<String>> = command_bar_controls(&test)
        .iter()
        .map(|control| {
            control
                .downcast_ref::<gtk::Button>()
                .and_then(ActionableExt::action_name)
                .map(|name| name.to_string())
        })
        .collect();
    let share = actions
        .iter()
        .position(|action| action.as_deref() == Some("win.share"))
        .expect("Share is shown");
    assert_eq!(
        actions[share - 1].as_deref(),
        Some("win.rename"),
        "after Rename, as in Windows 11"
    );
    assert_eq!(actions[share + 1].as_deref(), Some("win.trash"), "before Delete");

    select_names(&test, &["Notes 2.txt", "Résumé.txt"]);
    assert!(button.is_sensitive(), "files can be shared");
    button.emit_clicked();
    wait_until("the portal to be asked", || !portal.calls().is_empty());

    let calls = portal.calls();
    assert_eq!(calls.len(), 1, "one email");
    assert_eq!(
        calls[0].options,
        ["attachment_fds", "handle_token"],
        "only the files: no address, subject or body"
    );
    let mut attachments = calls[0].attachments.clone();
    attachments.sort();
    let mut expected = vec![fixture.path("Notes 2.txt"), fixture.path("Résumé.txt")];
    expected.sort();
    assert_eq!(attachments, expected);
    wait_for_frames(&test.window, 3);
    assert_eq!(
        test.window.shown_message().as_str(),
        "Opening your email app…",
        "only that the email app opens"
    );
    assert!(test.shown_dialog().is_none(), "nothing to report");
}

/// Windows cannot share a folder; nor can Share send an item of the
/// Recycle Bin or of a ZIP, which have no file for the email app.
///
/// parity: CMD-037
#[gtk::test]
fn share_needs_files_and_skips_folders() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let portal = EmailPortal::answering(0);
    use_portal(&test, &portal.name());
    let button = share_button(&test);

    select_names(&test, &["Documents"]);
    assert!(!button.is_sensitive(), "a folder alone");
    select_names(&test, &["Documents", "Notes 2.txt"]);
    assert!(!button.is_sensitive(), "a folder among files");
    select_names(&test, &["Notes 2.txt"]);
    assert!(button.is_sensitive(), "one file");
}

/// parity: CMD-037
#[gtk::test]
fn a_failed_email_shows_the_error_and_cancelling_shows_nothing() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let failing = EmailPortal::answering(2);
    use_portal(&test, &failing.name());
    select_names(&test, &["Notes 2.txt"]);

    share_button(&test).emit_clicked();
    // A failure is a message to read, in a dialog of its own.
    let dialog = test.wait_for_dialog("the error");
    let shown = super::item_dialogs::texts(&dialog);
    assert!(
        shown.contains(
            &"No email app is set up, so the email could not be written. Choose an email app in your \
              desktop's settings, then share again."
                .to_owned()
        ),
        "{shown:?}"
    );
    super::item_dialogs::press(&dialog, "OK");

    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let cancelled = EmailPortal::answering(1);
    use_portal(&test, &cancelled.name());
    select_names(&test, &["Notes 2.txt"]);
    share_button(&test).emit_clicked();
    wait_until("the portal to be asked", || !cancelled.calls().is_empty());
    wait_for_frames(&test.window, 3);
    assert!(test.shown_dialog().is_none(), "the user chose to");
}

/// One email takes at most 16 files, as one D-Bus message carries at
/// most 16 file descriptors; more are refused before the portal is asked.
///
/// parity: CMD-037
#[gtk::test]
fn more_files_than_an_email_takes_are_refused_with_a_reason() {
    let fixture = Fixture::with_files(17);
    let test = laid_out(&fixture.uri());
    let portal = EmailPortal::answering(0);
    use_portal(&test, &portal.name());
    test.window.folder_model().select_all();

    share_button(&test).emit_clicked();
    wait_until("the reason", || !test.window.shown_message().is_empty());
    assert_eq!(
        test.window.shown_message().as_str(),
        "Share can attach up to 16 files to one email."
    );
    assert!(portal.calls().is_empty(), "the portal is not asked");
}

/// GTK's portal backend writes each attachment's path into a `mailto:`
/// address unescaped, where a name like `report&bcc=…` would add a
/// recipient: such a file is refused by name before the portal is asked.
///
/// parity: CMD-037
#[gtk::test]
fn a_file_the_email_address_would_misread_is_refused_by_name() {
    let fixture = Fixture::standard();
    let name = "report&bcc=someone@example.invalid";
    std::fs::write(fixture.path(name), "report").expect("the file");
    let test = laid_out(&fixture.uri());
    let portal = EmailPortal::answering(0);
    use_portal(&test, &portal.name());
    select_names(&test, &[name]);

    share_button(&test).emit_clicked();
    // A refusal is a message to read, in a dialog of its own.
    let dialog = test.wait_for_dialog("the reason");
    let reason = format!(
        "“{name}” cannot be attached to an email because its path has &, #, ?, % or a \
         control character, which the email app could misread. Rename or move it, then share it again."
    );
    let shown = super::item_dialogs::texts(&dialog);
    assert!(shown.contains(&reason), "{shown:?}");
    assert!(
        test.window.shown_message().is_empty(),
        "nothing says the email app opens"
    );
    assert!(portal.calls().is_empty(), "the portal is not asked");
}

/// A compact window hides Share with Cut and Rename, and a wider one
/// shows it again.
///
/// parity: CMD-037, LOOK-022
#[gtk::test]
fn a_compact_window_hides_share() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let portal = EmailPortal::answering(0);
    use_portal(&test, &portal.name());
    let button = share_button(&test);

    test.window.set_default_size(672, 720);
    wait_until("the compact width", || test.window.width() <= 680);
    wait_for_frames(&test.window, 3);
    assert!(!button.is_visible(), "hidden in a compact window");
    test.window.set_default_size(1320, 720);
    wait_until("the wide width", || test.window.width() >= 1300);
    wait_for_frames(&test.window, 3);
    assert!(button.is_visible(), "back in a wide window");
}
