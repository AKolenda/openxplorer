// SPDX-License-Identifier: AGPL-3.0-only
//! Rename on temporary local files through the production GIO adapter,
//! and renaming back for Undo. The rename cases of
//! `desktop/tests/gio_integration.py` are ported here; renames on phones
//! are covered by the adapter's MTP tests (`transfer_cases/mtp_adapter.rs`).

mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;

use ox_core::ops::{rename_item, undo, OperationContext, OpsError, UndoRecord};

use ops_support::{block_on, file_uri};
use snapshots::{snapshot_protection, snapshot_protection_with_folders, READ_ONLY};

/// Ported from `desktop/tests/gio_integration.py::GioLocalIntegration::test_rename_does_not_overwrite`.
///
/// parity: OPS-008
#[test]
fn rename_onto_a_taken_name_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("a.txt");
    fs::write(&original, b"original").unwrap();
    let competing = temp.path().join("b.txt");
    fs::write(&competing, b"competing").unwrap();

    let renamed = block_on(rename_item(
        &file_uri(&original),
        "b.txt",
        &OperationContext::default(),
    ));

    let taken = "An item named “b.txt” already exists. Nothing was overwritten.";
    assert_eq!(renamed, Err(OpsError::Exists(taken.into())));
    assert_eq!(fs::read(&original).unwrap(), b"original");
    assert_eq!(fs::read(&competing).unwrap(), b"competing");
}

/// Ported from `desktop/tests/gio_integration.py::GioLocalIntegration::test_rename_parent_of_backup_is_rejected`.
///
/// parity: OPS-008, XFER-020
#[test]
fn a_folder_holding_a_protected_backup_is_not_renamed() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".snapshot")).unwrap();
    fs::write(project.join(".snapshot").join("version.txt"), b"backup").unwrap();
    let context = OperationContext::new(snapshot_protection());

    let renamed = block_on(rename_item(&file_uri(&project), "renamed", &context));

    assert_eq!(renamed, Err(OpsError::Failed(READ_ONLY.into())));
    assert_eq!(
        fs::read(project.join(".snapshot").join("version.txt")).unwrap(),
        b"backup"
    );
    assert!(!temp.path().join("renamed").exists());
}

/// The rename counterpart of
/// `desktop/tests/test_operations.py::ProtectedTransferTests::test_configured_backup_descendant_is_protected`:
/// a snapshot folder configured in the previous-versions settings is
/// protected like a conventional `.snapshot` folder.
///
/// parity: XFER-020
#[test]
fn a_folder_holding_a_configured_snapshot_folder_is_not_renamed() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let history = project.join("history");
    fs::create_dir_all(&history).unwrap();
    fs::write(history.join("version.txt"), b"backup").unwrap();
    let (live, snapshots) = (file_uri(&project), file_uri(&history));
    let protection = snapshot_protection_with_folders(&[(&live, &snapshots)]);

    let renamed = block_on(rename_item(&live, "renamed", &OperationContext::new(protection)));

    assert_eq!(renamed, Err(OpsError::Failed(READ_ONLY.into())));
    assert_eq!(fs::read(history.join("version.txt")).unwrap(), b"backup");
    assert!(!temp.path().join("renamed").exists());
}

/// parity: OPS-008
#[test]
fn a_rename_moves_the_item_within_its_folder_and_can_be_undone() {
    let temp = tempfile::tempdir().unwrap();
    let draft = temp.path().join("draft.txt");
    fs::write(&draft, b"text").unwrap();
    let context = OperationContext::default();

    let renamed = block_on(rename_item(&file_uri(&draft), "final.txt", &context)).unwrap();
    let record = renamed.undo_record().expect("a rename can be undone");
    let undone = block_on(undo(&record, &context, |_| {})).unwrap();

    assert_eq!(renamed.uri, file_uri(&temp.path().join("final.txt")));
    assert_eq!(renamed.original_uri, file_uri(&draft));
    assert_eq!(record.undo_label(), "Undo: Rename");
    assert!(undone.errors.is_empty(), "{:?}", undone.errors);
    assert_eq!(fs::read(&draft).unwrap(), b"text");
    assert!(!temp.path().join("final.txt").exists());
}

/// parity: OPS-008
#[test]
fn undoing_a_rename_never_overwrites_a_new_item_with_the_old_name() {
    let temp = tempfile::tempdir().unwrap();
    let draft = temp.path().join("draft.txt");
    fs::write(&draft, b"renamed").unwrap();
    let context = OperationContext::default();
    let renamed = block_on(rename_item(&file_uri(&draft), "final.txt", &context)).unwrap();
    fs::write(&draft, b"newer").unwrap();
    let record = renamed.undo_record().unwrap();

    let undone = block_on(undo(&record, &context, |_| {})).unwrap();

    let taken = "final.txt: An item named “draft.txt” already exists. Nothing was overwritten.";
    assert_eq!(undone.errors, [taken]);
    assert_eq!(fs::read(&draft).unwrap(), b"newer");
    assert_eq!(fs::read(temp.path().join("final.txt")).unwrap(), b"renamed");
}

/// parity: OPS-008
#[test]
fn renaming_to_the_same_name_changes_nothing_and_leaves_nothing_to_undo() {
    let temp = tempfile::tempdir().unwrap();
    let item = temp.path().join("same.txt");
    fs::write(&item, b"same").unwrap();

    let renamed = block_on(rename_item(
        &file_uri(&item),
        "same.txt",
        &OperationContext::default(),
    ))
    .unwrap();

    assert!(renamed.is_unchanged());
    assert_eq!(renamed.undo_record(), None);
    assert_eq!(fs::read(&item).unwrap(), b"same");
}

/// parity: OPS-006, OPS-035
#[test]
fn invalid_names_roots_and_whole_shares_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let item = temp.path().join("item.txt");
    fs::write(&item, b"item").unwrap();
    let context = OperationContext::default();

    let slash = block_on(rename_item(&file_uri(&item), "a/b", &context));
    let root = block_on(rename_item("file:///", "renamed", &context));
    let share = block_on(rename_item("smb://nas/share", "renamed", &context));

    let slash_message = "A name cannot contain slashes or control characters.";
    assert_eq!(slash, Err(OpsError::Failed(slash_message.into())));
    assert_eq!(
        root,
        Err(OpsError::Failed("Cannot rename a filesystem root.".into()))
    );
    let Err(OpsError::Failed(share_message)) = share else {
        panic!("a whole share is refused: {share:?}");
    };
    assert!(
        share_message.starts_with("Open the network share first"),
        "{share_message}"
    );
    assert!(item.exists());
}

#[test]
fn undo_renames_back_only_within_the_same_folder() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("other")).unwrap();
    let moved = temp.path().join("other").join("b.txt");
    fs::write(&moved, b"moved").unwrap();
    let record = UndoRecord::Rename {
        original_uri: file_uri(&temp.path().join("a.txt")),
        renamed_uri: file_uri(&moved),
    };

    let undone = block_on(undo(&record, &OperationContext::default(), |_| {})).unwrap();

    let refusal = "b.txt: Undo can only rename an item back within its own folder.";
    assert_eq!(undone.errors, [refusal]);
    assert!(moved.exists());
}
