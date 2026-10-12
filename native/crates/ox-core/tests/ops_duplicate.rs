// SPDX-License-Identifier: AGPL-3.0-only
//! Duplicate on temporary local files through the production GIO adapter.
//! Duplicate is new in the native app, from the Dolphin baseline, so no
//! Python test is ported here; its undo is in `ops_undo.rs`.

#[path = "ops_support/folders.rs"]
mod folders;
mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;

use ox_core::ops::{duplicate_items, summarize, OperationContext, OperationSummary, OpsError, UndoRecord};
use ox_core::transfer::TransferMode;

use folders::Folders;
use ops_support::{block_on, file_uri};
use snapshots::{snapshot_protection, READ_ONLY};

#[test]
fn duplicate_names_each_copy_like_keep_both_and_reports_the_copies() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("report.pdf"), b"pdf").unwrap();
    fs::write(temp.path().join("report - Copy.pdf"), b"older duplicate").unwrap();
    fs::create_dir(temp.path().join("Folder.v1")).unwrap();
    fs::write(temp.path().join("Folder.v1").join("inside.txt"), b"inside").unwrap();
    let items = [
        file_uri(&temp.path().join("report.pdf")),
        file_uri(&temp.path().join("Folder.v1")),
    ];

    let outcome = block_on(duplicate_items(&items, &OperationContext::default(), |_| {})).unwrap();

    let copies = vec![
        file_uri(&temp.path().join("report - Copy (2).pdf")),
        file_uri(&temp.path().join("Folder.v1 - Copy")),
    ];
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(outcome.created, copies);
    assert_eq!(outcome.undo, Some(UndoRecord::Duplicate { copies }));
    assert_eq!(
        fs::read(temp.path().join("report - Copy (2).pdf")).unwrap(),
        b"pdf"
    );
    assert_eq!(
        fs::read(temp.path().join("report - Copy.pdf")).unwrap(),
        b"older duplicate"
    );
    let inside = temp.path().join("Folder.v1 - Copy").join("inside.txt");
    assert_eq!(fs::read(inside).unwrap(), b"inside");
}

#[test]
fn duplicate_copies_items_of_several_folders_into_their_own_folders() {
    let folders = Folders::new();
    fs::write(folders.source().join("a.txt"), b"a").unwrap();
    fs::write(folders.destination().join("a.txt"), b"b").unwrap();
    let items = [
        file_uri(&folders.source().join("a.txt")),
        file_uri(&folders.destination().join("a.txt")),
    ];

    let outcome = block_on(duplicate_items(&items, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(fs::read(folders.source().join("a - Copy.txt")).unwrap(), b"a");
    assert_eq!(
        fs::read(folders.destination().join("a - Copy.txt")).unwrap(),
        b"b"
    );
    assert_eq!(outcome.created.len(), 2);
}

/// parity: OPS-035
#[test]
fn duplicate_refuses_roots_shares_and_protected_folders() {
    let temp = tempfile::tempdir().unwrap();
    let snapshot = temp.path().join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    fs::write(snapshot.join("old.txt"), b"old").unwrap();
    let context = OperationContext::new(snapshot_protection());

    let root = block_on(duplicate_items(&["file:///".into()], &context, |_| {}));
    let share = block_on(duplicate_items(&["smb://nas/share".into()], &context, |_| {}));
    let protected = block_on(duplicate_items(
        &[file_uri(&snapshot.join("old.txt"))],
        &context,
        |_| {},
    ));

    let roots = "Filesystem roots cannot be copied, moved or trashed as items.";
    assert_eq!(root, Err(OpsError::Failed(roots.into())));
    assert!(share.is_err());
    assert_eq!(protected, Err(OpsError::Failed(READ_ONLY.into())));
    assert_eq!(fs::read_dir(&snapshot).unwrap().count(), 1);
}

#[test]
fn a_cancelled_duplicate_reports_one_cancellation_and_copies_nothing() {
    let folders = Folders::new();
    fs::write(folders.source().join("a.txt"), b"a").unwrap();
    fs::write(folders.destination().join("b.txt"), b"b").unwrap();
    let items = [
        file_uri(&folders.source().join("a.txt")),
        file_uri(&folders.destination().join("b.txt")),
    ];
    let context = OperationContext::default();
    context.cancel.cancel();

    let outcome = block_on(duplicate_items(&items, &context, |_| {})).unwrap();

    assert!(outcome.result.cancelled);
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert!(outcome.created.is_empty());
    assert_eq!(outcome.undo, None);
    let report = "0 completed.\nCancelled. Completed items remain in place.";
    assert_eq!(
        summarize(TransferMode::Copy, &outcome.result),
        OperationSummary::Report(report.into())
    );
    assert_eq!(fs::read_dir(folders.source()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(folders.destination()).unwrap().count(), 1);
}
