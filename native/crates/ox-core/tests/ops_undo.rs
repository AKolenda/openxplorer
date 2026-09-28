// SPDX-License-Identifier: AGPL-3.0-only
//! Undo of each kind of operation, on temporary local files and the test
//! run's private Recycle Bin. Undo is new in the native app (OPS-029), so
//! no Python test is ported here; the rename cases are in
//! `ops_rename.rs`.

mod ops_support;
#[path = "ops_support/private_trash.rs"]
mod private_trash;

use std::fs;
use std::path::Path;

use ox_core::location::ItemKind;
use ox_core::ops::{
    create_item, duplicate_items, list_recycle_bin, run_transfer, undo, OperationContext, TransferRequest,
    UndoJournal, UndoRecord,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;

/// A request for `mode` over `items`, into `folder` when it has one.
fn request(mode: TransferMode, items: &[&Path], folder: Option<&Path>) -> TransferRequest {
    TransferRequest {
        mode,
        uris: items.iter().map(|path| file_uri(path)).collect(),
        destination_folder: folder.map(file_uri),
        policy: ConflictPolicy::Skip,
    }
}

/// True when the Recycle Bin holds an item that came from `path`.
fn is_in_recycle_bin(path: &Path) -> bool {
    let items = block_on(list_recycle_bin(&Cancellation::new())).expect("a readable Recycle Bin");
    items
        .iter()
        .any(|item| item.original_path.as_deref() == Some(path))
}

/// Runs `request` and returns how to undo it.
fn undo_record_of(request: &TransferRequest) -> UndoRecord {
    let outcome =
        block_on(run_transfer(request, &OperationContext::default(), |_| {})).expect("an accepted request");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    outcome.undo.expect("an undoable operation")
}

/// Undoes `record` and asserts that every step succeeded.
fn undo_fully(record: &UndoRecord) {
    let result = block_on(undo(record, &OperationContext::default(), |_| {})).expect("an undo");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.skipped.is_empty(), "{:?}", result.skipped);
}

#[test]
fn undoing_a_copy_moves_the_copies_to_the_trash_and_keeps_the_sources() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let (source, destination) = (temp.path().join("src"), temp.path().join("dst"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("a.txt"), b"a").unwrap();

    let record = undo_record_of(&request(
        TransferMode::Copy,
        &[&source.join("a.txt")],
        Some(&destination),
    ));
    undo_fully(&record);

    assert_eq!(record.undo_label(), "Undo: Copy");
    assert!(!destination.join("a.txt").exists());
    assert_eq!(fs::read(source.join("a.txt")).unwrap(), b"a");
    assert!(is_in_recycle_bin(&destination.join("a.txt")));
}

#[test]
fn undoing_a_move_puts_items_back_but_never_over_a_new_item() {
    let temp = tempfile::tempdir().unwrap();
    let (source, destination) = (temp.path().join("src"), temp.path().join("dst"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("a.txt"), b"a").unwrap();
    fs::write(source.join("b.txt"), b"b").unwrap();
    let items = [source.join("a.txt"), source.join("b.txt")];
    let items: Vec<&Path> = items.iter().map(std::path::PathBuf::as_path).collect();
    let record = undo_record_of(&request(TransferMode::Move, &items, Some(&destination)));
    fs::write(source.join("b.txt"), b"newer b").unwrap();

    let result = block_on(undo(&record, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(record.undo_label(), "Undo: Move");
    assert_eq!(result.done, [file_uri(&destination.join("a.txt"))]);
    assert_eq!(result.skipped, [file_uri(&destination.join("b.txt"))]);
    assert_eq!(fs::read(source.join("a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(source.join("b.txt")).unwrap(), b"newer b");
    assert_eq!(fs::read(destination.join("b.txt")).unwrap(), b"b");
}

#[test]
fn undoing_move_to_trash_restores_the_items_where_they_were() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Documents");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("report.txt"), b"report").unwrap();

    let record = undo_record_of(&request(TransferMode::Trash, &[&folder.join("report.txt")], None));
    let trashed = !folder.join("report.txt").exists() && is_in_recycle_bin(&folder.join("report.txt"));
    undo_fully(&record);

    assert!(trashed);
    assert_eq!(record.undo_label(), "Undo: Move to Trash");
    assert_eq!(fs::read(folder.join("report.txt")).unwrap(), b"report");
    assert!(!is_in_recycle_bin(&folder.join("report.txt")));
}

#[test]
fn undoing_move_to_trash_leaves_items_trashed_before_the_operation() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let item = temp.path().join("old.txt");
    fs::write(&item, b"old").unwrap();
    let record = undo_record_of(&request(TransferMode::Trash, &[&item], None));
    let UndoRecord::Trash { original_paths, .. } = record else {
        panic!("Move to Trash is undone from the Recycle Bin: {record:?}");
    };
    let later_operation = UndoRecord::Trash {
        original_paths,
        trashed_since: u64::MAX,
    };

    let result = block_on(undo(&later_operation, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(result.errors, ["old.txt: It is no longer in the Recycle Bin."]);
    assert!(!item.exists());
    assert!(is_in_recycle_bin(&item));
}

#[test]
fn undoing_new_folder_and_duplicate_moves_the_new_items_to_the_trash() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"a").unwrap();
    let context = OperationContext::default();
    let created = block_on(create_item(
        &file_uri(temp.path()),
        "New folder",
        ItemKind::Folder,
        &context,
    ))
    .unwrap();
    let duplicated = block_on(duplicate_items(
        &[file_uri(&temp.path().join("a.txt"))],
        &context,
        |_| {},
    ))
    .unwrap();
    let mut journal = UndoJournal::new();
    journal.record(created.undo_record());
    journal.record(duplicated.undo.expect("a duplicate can be undone"));

    let mut labels = Vec::new();
    while let Some(record) = journal.take_last() {
        undo_fully(&record);
        labels.push(record.undo_label());
    }

    assert_eq!(labels, ["Undo: Duplicate", "Undo: New folder"]);
    assert!(!temp.path().join("New folder").exists());
    assert!(!temp.path().join("a (copy 2).txt").exists());
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a");
    assert!(is_in_recycle_bin(&temp.path().join("a (copy 2).txt")));
}

#[test]
fn permanent_delete_leaves_nothing_to_undo() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"a").unwrap();

    let deletion = request(TransferMode::Delete, &[&temp.path().join("a.txt")], None);
    let outcome = block_on(run_transfer(&deletion, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(outcome.result.done.len(), 1);
    assert_eq!(outcome.undo, None);
    assert!(outcome.created.is_empty());
}
