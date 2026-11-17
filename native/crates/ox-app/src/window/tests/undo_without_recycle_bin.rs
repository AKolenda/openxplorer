// SPDX-License-Identifier: AGPL-3.0-only
//! Undo of a copy into a folder without a Recycle Bin, as on a share:
//! Undo asks Delete's own question there instead of failing (OPS-029).

use ox_core::ops::JournalDirection;

use super::file_ops_support::{
    is_enabled, open_dialog, require_private_trash, select_names, wait_for_no_dialog,
};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// A copy GIO says cannot go to the Recycle Bin (on a share, always;
/// here reported so for the test) is not sent to a Recycle Bin
/// that cannot take it: Undo asks, as Delete does there, "Are you sure
/// you want to permanently delete this file?", and No keeps the copy
/// and the Undo step.
///
/// parity: OPS-029, OPS-016
#[gtk::test]
fn undoing_a_copy_without_a_recycle_bin_asks_to_delete_it_permanently() {
    require_private_trash();
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
    crate::window::file_ops::report_no_recycle_bin(&fixture.uri_of("Documents/Notes 2.txt"));

    test.activate("undo", None);
    let question = open_dialog(&test);
    let (title, message, texts) = (question.title_text(), question.message_text(), question.texts());
    question.press("No");
    wait_for_no_dialog(&test);

    assert_eq!(title, "Delete File");
    assert_eq!(message, "Are you sure you want to permanently delete this file?");
    assert!(texts.iter().any(|text| text == "Notes 2.txt"), "{texts:?}");
    assert!(copy.is_file(), "No keeps the copy");
    assert_eq!(test.window.journal_label(JournalDirection::Undo), "Undo Copy");
}
