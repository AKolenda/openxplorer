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
    changed_copies, create_item, create_links, duplicate_items, list_recycle_bin, rename_batch, reverse,
    run_transfer, undo, BatchItem, BatchRename, LinkRequest, OperationContext, TransferRequest, UndoJournal,
    UndoRecord,
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

/// parity: OPS-030
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
    let an_hour_ahead = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    assert!(block_on(changed_copies(&record, an_hour_ahead)).is_empty());
    assert_eq!(
        block_on(changed_copies(&record, 0)),
        ["a.txt"],
        "changed since 1970"
    );
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

/// parity: OPS-029, OPS-014
#[test]
fn a_batch_rename_is_undone_and_redone_as_one_step() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["b.txt", "a.jpg"] {
        fs::write(temp.path().join(name), name).unwrap();
    }
    let batch = BatchRename {
        items: ["b.txt", "a.jpg"]
            .map(|name| BatchItem {
                uri: file_uri(&temp.path().join(name)),
                name: name.to_owned(),
                is_dir: false,
            })
            .to_vec(),
        pattern: "Trip #".to_owned(),
        first_number: 1,
    };
    let context = OperationContext::default();

    let outcome = block_on(rename_batch(&batch, &context)).expect("valid names");
    let record = outcome.undo.expect("an undoable batch");
    let reversal = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    let names_after_undo = [
        temp.path().join("b.txt").is_file(),
        temp.path().join("a.jpg").is_file(),
    ];
    let redo = reversal.inverse.expect("the undo can be redone");
    let redone = block_on(undo(&redo, &context, |_| {})).expect("a redo");

    assert_eq!(record.undo_label(), "Undo: Batch rename");
    assert_eq!(names_after_undo, [true, true]);
    assert!(redone.errors.is_empty(), "{:?}", redone.errors);
    assert_eq!(fs::read(temp.path().join("Trip 1.txt")).unwrap(), b"b.txt");
    assert_eq!(fs::read(temp.path().join("Trip 2.jpg")).unwrap(), b"a.jpg");
}

/// parity: OPS-029, DND-019
#[test]
fn undoing_links_moves_only_the_links_to_the_trash() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let (items, links) = (temp.path().join("items"), temp.path().join("links"));
    fs::create_dir(&items).unwrap();
    fs::create_dir(&links).unwrap();
    fs::write(items.join("a.txt"), b"a").unwrap();
    let request = LinkRequest {
        uris: vec![file_uri(&items.join("a.txt"))],
        destination_folder: file_uri(&links),
    };

    let outcome = block_on(create_links(&request, &OperationContext::default())).expect("a local folder");
    let record = outcome.undo.expect("undoable links");
    undo_fully(&record);

    assert_eq!(record.undo_label(), "Undo: Link");
    assert!(
        links.join("a.txt").symlink_metadata().is_err(),
        "the link is gone"
    );
    assert_eq!(fs::read(items.join("a.txt")).unwrap(), b"a", "its item stays");
}
