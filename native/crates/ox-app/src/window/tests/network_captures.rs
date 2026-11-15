// SPDX-License-Identifier: AGPL-3.0-only
//! Pictures of the network surfaces that changed, for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (and `GDK_SCALE=2` for 2x), these save
//! `network-progress-share-*.png`, `network-sign-in-retry-*.png` and
//! `network-undo-share-copy-*.png` there, in the light and dark themes.
//! Without it, they check what each surface says in both themes.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use gtk::prelude::*;
use ox_core::transfer::{ByteProgress, ItemCount, Progress, ProgressScope};

use super::file_ops_support::{
    is_enabled, open_dialog, require_private_trash, select_names, wait_for_no_dialog,
};
use crate::test_support::harness::{capture_dialog, wait_for, wait_until, Fixture, TestWindow, ThemeGuard};
use crate::window::transfer_panel::{Action, Verb};

/// The theme keys, as `win.theme` takes them.
const THEMES: [&str; 2] = ["light", "dark"];

/// A 300 MB file copied from the top folder of a share: the progress
/// window names the share as its tab does and moves its percentage with
/// the bytes, with the time remaining in More details.
#[gtk::test]
fn a_large_copy_from_a_share_is_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let documents = fixture.uri_of("Documents");
    let size: u64 = 300 * 1024 * 1024;
    for theme in THEMES {
        test.activate("theme", Some(theme));
        let context = test
            .window
            .begin_operation("Copying…")
            .expect("no other operation runs");
        let action = Action::of(
            Verb::Copying,
            &["smb://nas/media/Holiday video.mkv".to_owned()],
            Some(&documents),
        );
        test.window.describe_operation(&context.cancel, action);
        let panel = test.window.transfer_panel();
        for written in [0, 40 * 1024 * 1024, 120 * 1024 * 1024] {
            panel.show_progress(&Progress {
                label: "Holiday video.mkv".to_owned(),
                fraction: 0.0,
                scope: ProgressScope::File,
                bytes: Some(ByteProgress {
                    file_written: written,
                    file_size: size,
                    batch_written: written,
                    batch_size: Some(size),
                    file_started: written == 0,
                }),
                items: Some(ItemCount { done: 0, total: 1 }),
                name: Some("Holiday video.mkv".to_owned()),
            });
            wait_for(std::time::Duration::from_millis(600));
        }
        let header = panel.header_text();
        let percent = panel.percent_text();
        let progress = test.context.progress_window().expect("the operation's window");
        progress.set_visible(true);
        // "More details" is one choice for every action, kept between them.
        if !panel.shows_details_now() {
            progress.press_details();
        }
        capture_dialog(&progress, &format!("network-progress-share-{theme}.png"));
        test.window.end_operation();

        assert_eq!(header, "Copying 1 item from media to Documents", "{theme}");
        assert_eq!(percent, "40% complete", "{theme}");
    }
}

/// The sign-in dialog after the server refused "alex": it says so and
/// keeps the typed user name.
#[gtk::test]
fn a_refused_sign_in_is_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let test = TestWindow::open(crate::locations::Page::ThisPc.uri());
    let flags = gtk::gio::AskPasswordFlags::NEED_USERNAME
        | gtk::gio::AskPasswordFlags::NEED_PASSWORD
        | gtk::gio::AskPasswordFlags::SAVING_SUPPORTED
        | gtk::gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;
    for theme in THEMES {
        test.activate("theme", Some(theme));
        let prompts = test.window.network().prompts().clone();
        let operation = prompts.create("smb://studio-nas/projects").expect("an SMB share");
        let sign_in = test.window.network().sign_in().clone();
        operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
        wait_until("the sign-in dialog", || sign_in.shown_dialog().is_some());
        let first = sign_in.shown_dialog().expect("the sign-in dialog");
        first.type_account("alex", "not the password");
        first.press_connect();
        wait_until("the answer", || sign_in.shown_dialog().is_none());
        operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
        wait_until("the dialog again", || sign_in.shown_dialog().is_some());
        let again = sign_in.shown_dialog().expect("the sign-in dialog");
        let user = again.username_text();
        capture_dialog(
            again.upcast_ref::<gtk::Window>(),
            &format!("network-sign-in-retry-{theme}.png"),
        );
        prompts.finish(&operation, ox_core::network::MountOutcome::Failed);
        wait_until("the dialog to close", || sign_in.shown_dialog().is_none());

        assert_eq!(user, "alex", "{theme}");
    }
}

/// Undo of a copy that cannot go to the Recycle Bin (a share's, here a
/// folder that became read-only) asks Delete's question about it.
#[gtk::test]
fn undoing_a_copy_without_a_recycle_bin_is_captured_light_and_dark() {
    require_private_trash();
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    let copy = fixture.path("Documents/Notes 2.txt");
    wait_until("the copy", || copy.is_file());
    super::file_ops_support::assert_ended_without_a_message(&test);
    let documents = fixture.path("Documents");
    fs::set_permissions(&documents, fs::Permissions::from_mode(0o555)).expect("chmod Documents");
    for theme in THEMES {
        test.activate("theme", Some(theme));
        test.activate("undo", None);
        let question = open_dialog(&test);
        let message = question.message_text();
        capture_dialog(
            question.upcast_ref::<gtk::Window>(),
            &format!("network-undo-share-copy-{theme}.png"),
        );
        question.press("No");
        wait_for_no_dialog(&test);

        assert_eq!(
            message, "Are you sure you want to permanently delete this file?",
            "{theme}"
        );
    }
    fs::set_permissions(&documents, fs::Permissions::from_mode(0o755)).expect("chmod Documents back");
    assert!(copy.is_file(), "No keeps the copy");
}
