// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin (`trash:///`) through the Trash backend of `GVfs`, on
//! the test run's private Recycle Bin: listing, restoring, deleting and
//! emptying.
//! The Recycle Bin is new in the native app (OPS-040 to OPS-043), so no
//! Python test is ported here.
//!
//! The tests share one Recycle Bin, and emptying it removes every item, so
//! they run one at a time.

mod ops_support;
#[path = "ops_support/private_trash.rs"]
mod private_trash;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ox_core::ops::{
    delete_from_recycle_bin, empty_recycle_bin, list_recycle_bin, recycle_bin_item_count,
    restore_from_recycle_bin, run_transfer, undo, OperationContext, RecycledItem, TransferRequest,
    UndoRecord,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;
use snapshots::{snapshot_protection, READ_ONLY};

/// Lets one test at a time use the shared Recycle Bin.
static RECYCLE_BIN: Mutex<()> = Mutex::new(());

/// Waits for the Recycle Bin, after checking that it is the test run's own.
fn use_recycle_bin() -> MutexGuard<'static, ()> {
    require_private_trash();
    // A failed test must not keep the others from running.
    RECYCLE_BIN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Moves `items` to the Trash.
fn trash(items: &[&Path]) {
    let request = TransferRequest {
        mode: TransferMode::Trash,
        uris: items.iter().map(|path| file_uri(path)).collect(),
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    };
    let outcome =
        block_on(run_transfer(&request, &OperationContext::default(), |_| {})).expect("a Trash request");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
}

/// The Recycle Bin's items.
fn recycle_bin() -> Vec<RecycledItem> {
    block_on(list_recycle_bin(&Cancellation::new())).expect("a readable Recycle Bin")
}

/// The Recycle Bin item that came from `path`.
///
/// `GVfs`'s Trash backend learns of a new item from a file monitor, so a
/// listing right after a trash or a delete can still miss it; this waits
/// up to five seconds for it.
fn recycled_from(path: &Path) -> RecycledItem {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items = recycle_bin();
        let found = items
            .iter()
            .find(|item| item.original_path.as_deref() == Some(path));
        if let Some(item) = found {
            return item.clone();
        }
        assert!(
            Instant::now() < deadline,
            "{} is not in the Recycle Bin: {items:?}",
            path.display()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_trashed_item_is_listed_with_its_origin_and_restored_there() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let report = temp.path().join("report.txt");
    fs::write(&report, b"report").unwrap();

    trash(&[&report]);
    let item = recycled_from(&report);
    let context = OperationContext::default();
    let outcome = block_on(restore_from_recycle_bin(
        std::slice::from_ref(&item.uri),
        &context,
    ))
    .unwrap();

    assert_eq!(item.name, "report.txt");
    assert!(item.deleted_at.is_some());
    assert_eq!(outcome.result.done, [item.uri]);
    assert_eq!(outcome.created, [file_uri(&report)]);
    let restored = vec![file_uri(&report)];
    assert_eq!(outcome.undo, Some(UndoRecord::Restore { restored }));
    assert_eq!(fs::read(&report).unwrap(), b"report");
}

#[test]
fn restore_creates_the_missing_folders_of_the_original_location() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("Projects").join("2026");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("plan.txt"), b"plan").unwrap();

    trash(&[&nested.join("plan.txt")]);
    fs::remove_dir_all(temp.path().join("Projects")).unwrap();
    let item = recycled_from(&nested.join("plan.txt"));
    let outcome = block_on(restore_from_recycle_bin(
        &[item.uri],
        &OperationContext::default(),
    ))
    .unwrap();

    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(fs::read(nested.join("plan.txt")).unwrap(), b"plan");
}

#[test]
fn restore_never_overwrites_a_new_item_with_the_original_name() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    fs::write(&notes, b"trashed").unwrap();

    trash(&[&notes]);
    fs::write(&notes, b"newer").unwrap();
    let item = recycled_from(&notes);
    let outcome = block_on(restore_from_recycle_bin(
        &[item.uri],
        &OperationContext::default(),
    ))
    .unwrap();

    let refusal = "notes.txt: An item named “notes.txt” already exists in its original folder. \
                   It was left in the Recycle Bin.";
    assert_eq!(outcome.result.errors, [refusal]);
    assert_eq!(outcome.undo, None);
    assert_eq!(fs::read(&notes).unwrap(), b"newer");
    assert_eq!(
        recycled_from(&notes).original_path.as_deref(),
        Some(notes.as_path())
    );
}

#[test]
fn restore_into_a_protected_location_is_refused() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let snapshot = temp.path().join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    fs::write(snapshot.join("old.txt"), b"old").unwrap();

    trash(&[&snapshot.join("old.txt")]);
    let item = recycled_from(&snapshot.join("old.txt"));
    let context = OperationContext::new(snapshot_protection());
    let outcome = block_on(restore_from_recycle_bin(&[item.uri], &context)).unwrap();

    assert_eq!(outcome.result.errors, [format!("old.txt: {READ_ONLY}")]);
    assert!(!snapshot.join("old.txt").exists());
}

/// Every location the restored tree recreates is checked, not only the
/// original location of the folder itself.
///
/// parity: XFER-020
#[test]
fn restoring_a_folder_that_holds_a_protected_backup_is_refused() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".snapshot")).unwrap();
    fs::write(project.join(".snapshot").join("version.txt"), b"backup").unwrap();

    trash(&[&project]);
    let item = recycled_from(&project);
    let context = OperationContext::new(snapshot_protection());
    let outcome = block_on(restore_from_recycle_bin(&[item.uri], &context)).unwrap();

    assert_eq!(outcome.result.errors, [format!("project: {READ_ONLY}")]);
    assert_eq!(outcome.undo, None);
    assert!(!project.exists());
    assert_eq!(recycled_from(&project).name, "project");
}

#[test]
fn deleting_from_the_recycle_bin_removes_only_the_chosen_items() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let (chosen, kept) = (temp.path().join("chosen.txt"), temp.path().join("kept.txt"));
    fs::write(&chosen, b"chosen").unwrap();
    fs::write(&kept, b"kept").unwrap();

    trash(&[&chosen, &kept]);
    let item = recycled_from(&chosen);
    let result = block_on(delete_from_recycle_bin(
        std::slice::from_ref(&item.uri),
        &Cancellation::new(),
    ))
    .unwrap();

    assert_eq!(result.done, [item.uri]);
    let remaining: Vec<RecycledItem> = recycle_bin();
    assert!(!remaining
        .iter()
        .any(|item| item.original_path.as_deref() == Some(chosen.as_path())));
    assert!(remaining
        .iter()
        .any(|item| item.original_path.as_deref() == Some(kept.as_path())));
}

/// Regression: the Trash backend of `GVfs` stopped listing an item that
/// was trashed after an item with the same name had been deleted from the
/// Recycle Bin.
#[test]
fn an_item_trashed_after_deleting_one_with_the_same_name_is_listed() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let draft = temp.path().join("draft.txt");
    fs::write(&draft, b"first").unwrap();
    trash(&[&draft]);
    let first = recycled_from(&draft);

    let deleted = block_on(delete_from_recycle_bin(&[first.uri], &Cancellation::new())).unwrap();
    fs::write(&draft, b"second").unwrap();
    trash(&[&draft]);

    assert!(deleted.errors.is_empty(), "{:?}", deleted.errors);
    assert_eq!(recycled_from(&draft).name, "draft.txt");
}

#[test]
fn emptying_the_recycle_bin_deletes_everything_in_it_folders_included() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Old project");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("inside.txt"), b"inside").unwrap();
    let file = temp.path().join("old.txt");
    fs::write(&file, b"old").unwrap();
    trash(&[&folder, &file]);
    let before = recycle_bin();
    let temp_root = std::env::temp_dir();
    // Everything about to be deleted was trashed by these tests; see
    // `private_trash.rs` for why another volume's Trash is checked here.
    assert!(
        before.iter().all(|item| item
            .original_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&temp_root))),
        "the Recycle Bin holds items these tests did not trash: {before:?}"
    );

    let result = block_on(empty_recycle_bin(&Cancellation::new())).unwrap();
    let count = block_on(recycle_bin_item_count(&Cancellation::new())).unwrap();

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.done.len(), before.len());
    let listed: HashSet<&String> = before.iter().map(|item| &item.uri).collect();
    let emptied: HashSet<&String> = result.done.iter().collect();
    assert_eq!(emptied, listed, "emptying deleted only what was checked");
    assert_eq!(count, 0);
    assert!(recycle_bin().is_empty());
}

#[test]
fn only_whole_items_in_the_recycle_bin_are_restored_or_deleted() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, b"outside").unwrap();

    let restored = block_on(restore_from_recycle_bin(
        &[file_uri(&outside)],
        &OperationContext::default(),
    ))
    .unwrap();
    let deleted = block_on(delete_from_recycle_bin(
        &["trash:///folder/inside.txt".into()],
        &Cancellation::new(),
    ));

    let refusal = "Only whole items in the Recycle Bin can be restored or deleted there.";
    assert_eq!(restored.result.errors, [format!("outside.txt: {refusal}")]);
    assert_eq!(deleted.unwrap().errors, [format!("inside.txt: {refusal}")]);
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
}

#[test]
fn undoing_a_restore_moves_the_items_to_the_trash_again() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let draft = temp.path().join("draft.txt");
    fs::write(&draft, b"draft").unwrap();
    trash(&[&draft]);
    let item = recycled_from(&draft);
    let outcome = block_on(restore_from_recycle_bin(
        &[item.uri],
        &OperationContext::default(),
    ))
    .unwrap();
    let record = outcome.undo.expect("a restore can be undone");

    let result = block_on(undo(&record, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(record.undo_label(), "Undo: Restore");
    assert_eq!(result.done, [file_uri(&draft)]);
    assert!(!draft.exists());
    assert_eq!(recycled_from(&draft).name, "draft.txt");
}
