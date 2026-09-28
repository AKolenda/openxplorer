// SPDX-License-Identifier: AGPL-3.0-only
//! The paste-conflict check, copy and move requests, the Delete plan and
//! Duplicate on temporary local files through the production GIO adapter.
//! The transfer engine's own safety rules are tested in `transfer.rs`;
//! these cases cover what the interface's requests add around it. Moving
//! to the Trash and Undo are in `ops_recycle_bin.rs` and `ops_undo.rs`.

mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gio::prelude::*;

use ox_core::ops::{
    duplicate_items, find_conflicts, plan_delete, run_transfer, trash_support, DeleteItem, OperationContext,
    OpsError, TransferRequest, UndoRecord,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, Progress, TransferMode, MAX_ITEMS};

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

/// A source folder `src` and a destination folder `dst` in a temporary
/// folder.
struct Folders {
    temp: tempfile::TempDir,
}

impl Folders {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::create_dir(temp.path().join("dst")).unwrap();
        Self { temp }
    }

    fn source(&self) -> std::path::PathBuf {
        self.temp.path().join("src")
    }

    fn destination(&self) -> std::path::PathBuf {
        self.temp.path().join("dst")
    }
}

/// parity: OPS-027
#[test]
fn conflicts_list_the_items_whose_names_are_taken_hidden_and_dangling_ones_included() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    for name in ["free.txt", "taken.txt", ".hidden", "link"] {
        fs::write(src.join(name), b"incoming").unwrap();
    }
    fs::write(dst.join("taken.txt"), b"existing").unwrap();
    fs::write(dst.join(".hidden"), b"existing").unwrap();
    symlink("missing-target", dst.join("link")).unwrap();
    let uris: Vec<String> = ["free.txt", "taken.txt", ".hidden", "link"]
        .iter()
        .map(|name| file_uri(&src.join(name)))
        .collect();

    let conflicts = block_on(find_conflicts(&uris, &file_uri(&dst), &Cancellation::new())).unwrap();

    assert_eq!(conflicts, uris[1..]);
}

/// parity: OPS-027, OPS-035
#[test]
fn a_conflict_check_needs_a_folder_and_between_one_and_the_maximum_items() {
    let folders = Folders::new();
    let file = folders.source().join("a.txt");
    fs::write(&file, b"a").unwrap();
    let cancel = Cancellation::new();
    let one = [file_uri(&file)];
    let too_many = vec![file_uri(&file); MAX_ITEMS + 1];

    let into_file = block_on(find_conflicts(&one, &file_uri(&file), &cancel));
    let none = block_on(find_conflicts(&[], &file_uri(&folders.destination()), &cancel));
    let oversized = block_on(find_conflicts(
        &too_many,
        &file_uri(&folders.destination()),
        &cancel,
    ));
    let share = block_on(find_conflicts(
        &["smb://nas/share".into()],
        "file:///tmp",
        &cancel,
    ));

    let open_folder = "Open a destination folder before pasting.";
    assert_eq!(into_file, Err(OpsError::Failed(open_folder.into())));
    let count = OpsError::Failed("Select between 1 and 100,000 items.".into());
    assert_eq!(none, Err(count.clone()));
    assert_eq!(oversized, Err(count));
    assert!(
        matches!(share, Err(OpsError::Failed(message)) if message.starts_with("Open the network share first"))
    );
}

/// Ported from `desktop/tests/test_rc2.py::DispatchTests::test_operate_dispatch_protects_backup_descendants`.
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

/// parity: OPS-027
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
    assert_eq!(reports[0].label, "Copy: a.txt (1/1)");
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

/// parity: OPS-035
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

#[test]
fn duplicate_names_each_copy_like_keep_both_and_reports_the_copies() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("report.pdf"), b"pdf").unwrap();
    fs::write(temp.path().join("report (copy 2).pdf"), b"older duplicate").unwrap();
    fs::create_dir(temp.path().join("Folder.v1")).unwrap();
    fs::write(temp.path().join("Folder.v1").join("inside.txt"), b"inside").unwrap();
    let items = [
        file_uri(&temp.path().join("report.pdf")),
        file_uri(&temp.path().join("Folder.v1")),
    ];

    let outcome = block_on(duplicate_items(&items, &OperationContext::default(), |_| {})).unwrap();

    let copies = vec![
        file_uri(&temp.path().join("report (copy 3).pdf")),
        file_uri(&temp.path().join("Folder.v1 (copy 2)")),
    ];
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(outcome.created, copies);
    assert_eq!(outcome.undo, Some(UndoRecord::Duplicate { copies }));
    assert_eq!(fs::read(temp.path().join("report (copy 3).pdf")).unwrap(), b"pdf");
    assert_eq!(
        fs::read(temp.path().join("report (copy 2).pdf")).unwrap(),
        b"older duplicate"
    );
    let inside = temp.path().join("Folder.v1 (copy 2)").join("inside.txt");
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

    assert_eq!(fs::read(folders.source().join("a (copy 2).txt")).unwrap(), b"a");
    assert_eq!(
        fs::read(folders.destination().join("a (copy 2).txt")).unwrap(),
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
fn local_items_go_to_the_trash_and_a_folder_without_trash_is_deleted_permanently() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("a.txt");
    fs::write(&local, b"a").unwrap();
    let vanished = temp.path().join("gone").join("b.txt");
    let items = [
        DeleteItem {
            uri: file_uri(&local),
            name: "a.txt".into(),
        },
        DeleteItem {
            uri: file_uri(&vanished),
            name: "b.txt".into(),
        },
    ];
    let cancel = Cancellation::new();

    let local_trash = block_on(trash_support(&file_uri(temp.path()), &cancel));
    let plan = block_on(plan_delete(&items, &cancel)).unwrap();

    assert_eq!(local_trash, Ok(true));
    assert_eq!(plan.to_trash, [file_uri(&local)]);
    assert_eq!(plan.to_delete, [file_uri(&vanished)]);
    assert_eq!(plan.confirmation().title, "Delete items?");
}

#[test]
fn a_share_that_cannot_answer_counts_as_having_a_trash() {
    let schemes = gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "smb"),
        "this check needs GVfs with its SMB backend (gvfs-backends); found {schemes:?}"
    );
    let item = DeleteItem {
        uri: "smb://example.invalid/share/report.pdf".into(),
        name: "report.pdf".into(),
    };
    let cancel = Cancellation::new();

    let support = block_on(trash_support("smb://example.invalid/share", &cancel));
    let plan = block_on(plan_delete(std::slice::from_ref(&item), &cancel)).unwrap();

    assert!(matches!(support, Err(OpsError::NotMounted(_))), "{support:?}");
    assert!(support.unwrap_err().needs_mount());
    assert_eq!(plan.to_trash, [item.uri]);
    assert!(plan.to_delete.is_empty());
    assert_eq!(plan.confirmation().title, "Move to Trash?");
}
