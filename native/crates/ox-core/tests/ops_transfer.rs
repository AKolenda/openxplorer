// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move and delete requests on
//! temporary local files through the production GIO adapter. The transfer
//! engine's own safety rules are tested in `transfer.rs`; these cases cover
//! what the interface's requests add around it. The paste-conflict check
//! and its answers are in `ops_name_conflicts.rs`; the Delete plan, Duplicate,
//! moving to the Trash and Undo are in `ops_delete_plan.rs`,
//! `ops_duplicate.rs`, `ops_recycle_bin.rs` and `ops_undo.rs`.

#[path = "ops_support/folders.rs"]
mod folders;
mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ox_core::ops::{run_transfer, OperationContext, OpsError, TransferRequest, UndoRecord};
use ox_core::transfer::{ConflictPolicy, ItemCount, Progress, ProgressScope, TransferMode};

use folders::Folders;
use ops_support::{block_on, file_uri};
use snapshots::{snapshot_protection, READ_ONLY};

/// A copy or move of `sources` into `folder` under `policy`.
fn request(mode: TransferMode, sources: &[&Path], folder: &Path, policy: ConflictPolicy) -> TransferRequest {
    TransferRequest {
        mode,
        uris: sources.iter().map(|path| file_uri(path)).collect(),
        destination_folder: Some(file_uri(folder)),
        policy,
    }
}

/// Ported from `v2.0.0:desktop/tests/test_rc2.py::DispatchTests::test_operate_dispatch_protects_backup_descendants`.
///
/// parity: XFER-020
#[test]
fn copy_and_delete_requests_keep_protected_backups_intact() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source").join("project");
    let target = temp.path().join("destination").join("project");
    for (folder, contents) in [(&source, "incoming"), (&target, "saved")] {
        fs::create_dir_all(folder.join(".snapshot")).unwrap();
        fs::write(folder.join(".snapshot").join("version.txt"), contents).unwrap();
    }
    let context = OperationContext::new(snapshot_protection());
    let target_folder = target.parent().unwrap();
    let replace = request(
        TransferMode::Copy,
        &[&source],
        target_folder,
        ConflictPolicy::Replace,
    );
    let delete = TransferRequest {
        mode: TransferMode::Delete,
        uris: vec![file_uri(&target)],
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    };

    let replaced = block_on(run_transfer(&replace, &context, |_| {})).unwrap();
    let deleted = block_on(run_transfer(&delete, &context, |_| {})).unwrap();

    let refusal = format!("project: {READ_ONLY}");
    assert_eq!(replaced.result.errors, std::slice::from_ref(&refusal));
    assert_eq!(deleted.result.errors, [refusal]);
    assert!(deleted.result.done.is_empty());
    let saved = fs::read_to_string(target.join(".snapshot").join("version.txt")).unwrap();
    assert_eq!(saved, "saved");
}

/// parity: OPS-027
#[test]
fn a_copy_reports_its_copies_and_how_to_undo_it() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    fs::write(src.join("a.txt"), b"a").unwrap();
    fs::create_dir(src.join("folder")).unwrap();
    fs::write(dst.join("taken.txt"), b"existing").unwrap();
    fs::write(src.join("taken.txt"), b"incoming").unwrap();
    let sources = [src.join("a.txt"), src.join("folder"), src.join("taken.txt")];
    let sources: Vec<&Path> = sources.iter().map(std::path::PathBuf::as_path).collect();

    let copy = request(TransferMode::Copy, &sources, &dst, ConflictPolicy::Skip);
    let outcome = block_on(run_transfer(&copy, &OperationContext::default(), |_| {})).unwrap();

    let copies = vec![file_uri(&dst.join("a.txt")), file_uri(&dst.join("folder"))];
    assert_eq!(outcome.created, copies);
    assert_eq!(outcome.undo, Some(UndoRecord::Copy { copies }));
    assert_eq!(outcome.result.skipped, [file_uri(&src.join("taken.txt"))]);
    assert_eq!(fs::read(dst.join("taken.txt")).unwrap(), b"existing");
}

#[test]
fn replace_reports_its_items_but_cannot_be_undone() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    fs::write(src.join("a.txt"), b"new").unwrap();
    fs::write(dst.join("a.txt"), b"old").unwrap();

    let replace = request(
        TransferMode::Copy,
        &[&src.join("a.txt")],
        &dst,
        ConflictPolicy::Replace,
    );
    let outcome = block_on(run_transfer(&replace, &OperationContext::default(), |_| {})).unwrap();

    assert_eq!(outcome.created, [file_uri(&dst.join("a.txt"))]);
    assert_eq!(outcome.undo, None);
    assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"new");
}

/// parity: OPS-020
#[test]
fn progress_is_reported_and_the_last_report_says_the_run_is_complete() {
    let folders = Folders::new();
    fs::write(folders.source().join("a.txt"), b"a").unwrap();
    let reports: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = Arc::clone(&reports);

    let copy = request(
        TransferMode::Copy,
        &[&folders.source().join("a.txt")],
        &folders.destination(),
        ConflictPolicy::Skip,
    );
    let outcome = block_on(run_transfer(
        &copy,
        &OperationContext::default(),
        move |progress| {
            sink.lock().unwrap().push(progress);
        },
    ));

    assert!(outcome.is_ok());
    let reports = reports.lock().unwrap();
    let last = reports.last().expect("at least the final report");
    assert_eq!(last.label, "1 item(s) completed");
    assert!((last.fraction - 1.0).abs() < f64::EPSILON);
    assert_eq!(last.scope, ProgressScope::Batch);
    assert_eq!(reports[0].label, "Copy: a.txt (1/1)");
    assert_eq!(reports[0].scope, ProgressScope::Batch);
    let file_reports: Vec<&Progress> = reports
        .iter()
        .filter(|report| report.label.starts_with("Copying a.txt"))
        .collect();
    assert!(!file_reports.is_empty(), "the file's bytes are reported");
    assert!(file_reports
        .iter()
        .all(|report| report.scope == ProgressScope::File));
}

/// Every report says how many items the run has finished out of how
/// many, as the progress window's "Items remaining" needs: the files in
/// the copied folders once the free-space check has counted them, and
/// the name of the item being handled.
///
/// parity: OPS-021
#[test]
fn progress_counts_every_file_and_names_the_current_one() {
    let folders = Folders::new();
    let album = folders.source().join("Album");
    fs::create_dir(&album).unwrap();
    for name in ["one.jpg", "two.jpg", "three.jpg"] {
        fs::write(album.join(name), name).unwrap();
    }
    fs::write(folders.source().join("notes.txt"), b"notes").unwrap();
    let reports: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = Arc::clone(&reports);

    let copy = request(
        TransferMode::Copy,
        &[&album, &folders.source().join("notes.txt")],
        &folders.destination(),
        ConflictPolicy::Skip,
    );
    block_on(run_transfer(
        &copy,
        &OperationContext::default(),
        move |progress| sink.lock().unwrap().push(progress),
    ))
    .unwrap();

    let reports = reports.lock().unwrap();
    let counted: Vec<ItemCount> = reports.iter().filter_map(|report| report.items).collect();
    assert!(!counted.is_empty(), "the reports count the items");
    assert!(counted.iter().all(|count| count.total == 4), "{counted:?}");
    assert!(
        counted.windows(2).all(|pair| pair[0].done <= pair[1].done),
        "{counted:?}"
    );
    assert_eq!(
        reports.last().unwrap().items,
        Some(ItemCount { done: 4, total: 4 })
    );
    let names: Vec<&str> = reports
        .iter()
        .filter_map(|report| report.name.as_deref())
        .collect();
    for name in ["Album", "one.jpg", "two.jpg", "three.jpg", "notes.txt"] {
        assert!(names.contains(&name), "{name} is named: {names:?}");
    }
}

/// A deletion, which measures nothing first, counts the selected items.
///
/// parity: OPS-021
#[test]
fn a_deletion_counts_the_selected_items() {
    let folders = Folders::new();
    let names = ["a.txt", "b.txt"];
    for name in names {
        fs::write(folders.source().join(name), name).unwrap();
    }
    let reports: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = Arc::clone(&reports);
    let delete = TransferRequest {
        mode: TransferMode::Delete,
        uris: names
            .iter()
            .map(|name| file_uri(&folders.source().join(name)))
            .collect(),
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    };

    block_on(run_transfer(
        &delete,
        &OperationContext::default(),
        move |progress| sink.lock().unwrap().push(progress),
    ))
    .unwrap();

    let counted: Vec<ItemCount> = reports
        .lock()
        .unwrap()
        .iter()
        .filter_map(|report| report.items)
        .collect();
    assert_eq!(
        counted,
        [
            ItemCount { done: 0, total: 2 },
            ItemCount { done: 1, total: 2 },
            ItemCount { done: 2, total: 2 },
        ]
    );
}

/// Each item's batch report reaches the panel, even right after the
/// previous file's full bar.
///
/// parity: OPS-020
#[test]
fn every_items_batch_report_arrives_in_a_quick_copy() {
    let folders = Folders::new();
    let names = ["a.txt", "b.txt", "c.txt"];
    for name in names {
        fs::write(folders.source().join(name), name).unwrap();
    }
    let sources: Vec<_> = names.iter().map(|name| folders.source().join(name)).collect();
    let sources: Vec<&Path> = sources.iter().map(std::path::PathBuf::as_path).collect();
    let reports: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let sink = Arc::clone(&reports);

    let copy = request(
        TransferMode::Copy,
        &sources,
        &folders.destination(),
        ConflictPolicy::Skip,
    );
    block_on(run_transfer(
        &copy,
        &OperationContext::default(),
        move |progress| sink.lock().unwrap().push(progress),
    ))
    .unwrap();

    let bytes: Vec<_> = reports
        .lock()
        .unwrap()
        .iter()
        .filter_map(|report| report.bytes)
        .collect();
    assert_eq!(bytes.last().unwrap().batch_written, 15);
    assert_eq!(bytes.last().unwrap().batch_size, Some(15));
    assert!(bytes
        .windows(2)
        .all(|pair| pair[0].batch_written <= pair[1].batch_written));

    let batch_labels: Vec<String> = reports
        .lock()
        .unwrap()
        .iter()
        .filter(|report| report.scope == ProgressScope::Batch)
        .map(|report| report.label.clone())
        .collect();
    assert_eq!(
        batch_labels,
        [
            "Copy: a.txt (1/3)",
            "Copy: b.txt (2/3)",
            "Copy: c.txt (3/3)",
            "3 item(s) completed"
        ]
    );
}

/// parity: OPS-022
#[test]
fn a_cancelled_copy_stops_before_anything_is_copied() {
    let folders = Folders::new();
    fs::write(folders.source().join("a.txt"), b"a").unwrap();
    let context = OperationContext::default();
    context.cancel.cancel();

    let copy = request(
        TransferMode::Copy,
        &[&folders.source().join("a.txt")],
        &folders.destination(),
        ConflictPolicy::Skip,
    );
    let outcome = block_on(run_transfer(&copy, &context, |_| {}));

    assert_eq!(outcome, Err(OpsError::Cancelled));
    assert_eq!(fs::read_dir(folders.destination()).unwrap().count(), 0);
    assert_eq!(fs::read(folders.source().join("a.txt")).unwrap(), b"a");
}

/// parity: OPS-035, OPS-036
#[test]
fn requests_with_a_whole_share_or_a_server_listing_are_refused_before_anything_changes() {
    let folders = Folders::new();
    let file = folders.source().join("a.txt");
    fs::write(&file, b"a").unwrap();
    let context = OperationContext::default();
    let to_server = TransferRequest {
        mode: TransferMode::Copy,
        uris: vec![file_uri(&file)],
        destination_folder: Some("smb://nas/".into()),
        policy: ConflictPolicy::Skip,
    };
    let whole_share = TransferRequest {
        mode: TransferMode::Trash,
        uris: vec![file_uri(&file), "smb://nas/share".into()],
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    };

    let pasted = block_on(run_transfer(&to_server, &context, |_| {}));
    let trashed = block_on(run_transfer(&whole_share, &context, |_| {}));

    let open_share = "Open a network share before pasting files.";
    assert_eq!(pasted, Err(OpsError::Failed(open_share.into())));
    assert!(
        matches!(trashed, Err(OpsError::Failed(message)) if message.starts_with("Open the network share first"))
    );
    assert_eq!(fs::read(&file).unwrap(), b"a");
}

/// parity: XFER-020
#[test]
fn a_move_out_of_a_protected_folder_is_refused_before_anything_changes() {
    let folders = Folders::new();
    let snapshot = folders.source().join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    fs::write(snapshot.join("old.txt"), b"old").unwrap();

    let move_out = request(
        TransferMode::Move,
        &[&snapshot.join("old.txt")],
        &folders.destination(),
        ConflictPolicy::Skip,
    );
    let moved = block_on(run_transfer(
        &move_out,
        &OperationContext::new(snapshot_protection()),
        |_| {},
    ));

    assert_eq!(moved, Err(OpsError::Failed(READ_ONLY.into())));
    assert!(snapshot.join("old.txt").exists());
}
