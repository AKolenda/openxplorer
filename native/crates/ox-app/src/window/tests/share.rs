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

/// parity: CMD-037
#[gtk::test]
fn the_share_button_is_named_share_and_hidden_without_an_email_portal() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let button = share_button(&test);
    assert_eq!(button.tooltip_text().as_deref(), Some("Share"));
    let settings_only = SettingsPortal::serving(&[]);

    use_portal(&test, &settings_only.name());
    assert!(!button.is_visible(), "a portal without Email cannot share");
    use_portal(&test, ":1.4294967");
    assert!(!button.is_visible(), "nobody answers under that name");
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
    assert_eq!(test.window.shown_message().as_str(), "", "nothing to report");
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
    wait_until("the error", || !test.window.shown_message().is_empty());
    assert_eq!(
        test.window.shown_message().as_str(),
        "The email app could not be opened."
    );

    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let cancelled = EmailPortal::answering(1);
    use_portal(&test, &cancelled.name());
    select_names(&test, &["Notes 2.txt"]);
    share_button(&test).emit_clicked();
    wait_until("the portal to be asked", || !cancelled.calls().is_empty());
    wait_for_frames(&test.window, 3);
    assert_eq!(test.window.shown_message().as_str(), "", "the user chose to");
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
