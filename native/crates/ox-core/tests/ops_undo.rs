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
use std::thread;
use std::time::{Duration, Instant};

use ox_core::location::ItemKind;
use ox_core::ops::{
    changed_copies, create_item, create_links, duplicate_items, list_recycle_bin, removed_by_undo,
    rename_batch, rename_numbered_items, reverse, reverse_deleting, run_transfer, undo, BatchItem,
    BatchRename, LinkRequest, NumberedRename, OperationContext, TransferRequest, UndoJournal, UndoRecord,
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

/// Waits for `GVfs`'s asynchronous monitor to report the expected state.
fn assert_in_recycle_bin(path: &Path, expected: bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items = block_on(list_recycle_bin(&Cancellation::new())).expect("a readable Recycle Bin");
        let present = items
            .iter()
            .any(|item| item.original_path.as_deref() == Some(path));
        if present == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Unexpected Recycle Bin state: {items:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
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

    assert_eq!(record.undo_label(), "Undo Copy");
    assert!(!destination.join("a.txt").exists());
    assert_eq!(fs::read(source.join("a.txt")).unwrap(), b"a");
    assert_in_recycle_bin(&destination.join("a.txt"), true);
}

/// Copies on a share have no Recycle Bin to go to. Once the user agreed,
/// as to Delete there, Undo deletes those copies permanently and still
/// moves the others to the Recycle Bin; Redo can bring back only those.
/// The sources are never touched.
///
/// parity: OPS-029, OPS-030
#[test]
fn undoing_a_copy_deletes_the_agreed_copies_that_have_no_recycle_bin() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let (source, destination) = (temp.path().join("src"), temp.path().join("dst"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("a.txt"), b"a").unwrap();
    fs::write(source.join("b.txt"), b"b").unwrap();
    let record = undo_record_of(&request(
        TransferMode::Copy,
        &[&source.join("a.txt"), &source.join("b.txt")],
        Some(&destination),
    ));
    let copy_of_b = file_uri(&destination.join("b.txt"));
    let mut removed = removed_by_undo(&record);
    removed.sort();
    assert_eq!(removed, [file_uri(&destination.join("a.txt")), copy_of_b.clone()]);

    let reversal = block_on(reverse_deleting(
        &record,
        std::slice::from_ref(&copy_of_b),
        &OperationContext::default(),
        |_| {},
    ))
    .expect("an undo");

    assert!(reversal.result.errors.is_empty(), "{:?}", reversal.result.errors);
    assert!(!destination.join("a.txt").exists());
    assert!(!destination.join("b.txt").exists());
    assert_in_recycle_bin(&destination.join("a.txt"), true);
    assert_in_recycle_bin(&destination.join("b.txt"), false);
    assert_eq!(fs::read(source.join("b.txt")).unwrap(), b"b");
    let Some(UndoRecord::Trash { original_paths, .. }) = reversal.inverse else {
        panic!(
            "Redo restores what went to the Recycle Bin: {:?}",
            reversal.inverse
        );
    };
    assert_eq!(original_paths, [destination.join("a.txt")]);
}

/// A copy that overwrote one file, as Replace does with the switch off,
/// and made others is a Replace record around its Copy step. Where the
/// copies have no Recycle Bin, as on a share, Undo still lists them for
/// Delete's own question, and once the user agreed it deletes those
/// permanently and moves the others to the Recycle Bin, as for a plain
/// copy; the overwritten file stays.
///
/// parity: OPS-028, OPS-029
#[test]
fn undoing_a_replacing_copy_deletes_the_agreed_copies_that_have_no_recycle_bin() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("dst");
    fs::create_dir(&destination).unwrap();
    for name in ["a.txt", "b.txt", "replaced.txt"] {
        fs::write(destination.join(name), name).unwrap();
    }
    let (copy_of_a, copy_of_b) = (
        file_uri(&destination.join("a.txt")),
        file_uri(&destination.join("b.txt")),
    );
    let record = UndoRecord::Replace {
        operation: ox_core::ops::Operation::Copy,
        steps: vec![UndoRecord::Copy {
            copies: vec![copy_of_a.clone(), copy_of_b.clone()],
        }],
        replaced: Vec::new(),
        replaced_since: 0,
        overwritten: vec![file_uri(&destination.join("replaced.txt"))],
    };
    let mut removed = removed_by_undo(&record);
    removed.sort();
    assert_eq!(
        removed,
        [copy_of_a, copy_of_b.clone()],
        "listed for Delete's question"
    );

    let reversal = block_on(reverse_deleting(
        &record,
        std::slice::from_ref(&copy_of_b),
        &OperationContext::default(),
        |_| {},
    ))
    .expect("an undo");

    assert!(reversal.result.errors.is_empty(), "{:?}", reversal.result.errors);
    assert!(!destination.join("a.txt").exists());
    assert!(!destination.join("b.txt").exists());
    assert_in_recycle_bin(&destination.join("a.txt"), true);
    assert_in_recycle_bin(&destination.join("b.txt"), false);
    assert_eq!(
        fs::read(destination.join("replaced.txt")).unwrap(),
        b"replaced.txt",
        "the replacement stays"
    );
    let Some(UndoRecord::Replace { replaced, steps, .. }) = reversal.inverse else {
        panic!(
            "Redo restores what went to the Recycle Bin: {:?}",
            reversal.inverse
        );
    };
    assert!(steps.is_empty(), "{steps:?}");
    assert_eq!(
        replaced,
        [destination.join("a.txt")],
        "only what went to the Recycle Bin"
    );
}

/// A file edited inside a copied folder leaves the folder's own time
/// alone, but still counts as a change to the copy.
///
/// parity: OPS-030
#[test]
fn an_edit_inside_a_copied_folder_counts_as_a_change() {
    let temp = tempfile::tempdir().unwrap();
    let (source, destination) = (temp.path().join("src"), temp.path().join("dst"));
    fs::create_dir_all(source.join("project/notes")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("project/notes/work.txt"), b"draft").unwrap();
    let record = undo_record_of(&request(
        TransferMode::Copy,
        &[&source.join("project")],
        Some(&destination),
    ));
    let now = std::time::SystemTime::now();
    let since = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let untouched = block_on(changed_copies(&record, since));

    let edited = fs::File::options()
        .write(true)
        .open(destination.join("project/notes/work.txt"))
        .unwrap();
    edited
        .set_modified(now + std::time::Duration::from_secs(60))
        .unwrap();

    assert!(untouched.is_empty(), "{untouched:?}");
    assert_eq!(block_on(changed_copies(&record, since)), ["project"]);
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

    assert_eq!(record.undo_label(), "Undo Move");
    assert_eq!(result.done, [file_uri(&destination.join("a.txt"))]);
    assert_eq!(result.skipped, [file_uri(&destination.join("b.txt"))]);
    assert_eq!(fs::read(source.join("a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(source.join("b.txt")).unwrap(), b"newer b");
    assert_eq!(fs::read(destination.join("b.txt")).unwrap(), b"b");
}

/// A move answered with Keep both lands under a new name; Undo brings
/// the item back under its old one, and Redo moves it on again under
/// the new one.
#[test]
fn undoing_a_keep_both_move_restores_the_original_name() {
    let temp = tempfile::tempdir().unwrap();
    let (source, destination) = (temp.path().join("src"), temp.path().join("dst"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("Report.txt"), b"moved").unwrap();
    fs::write(destination.join("Report.txt"), b"already there").unwrap();
    let mut keep_both = request(
        TransferMode::Move,
        &[&source.join("Report.txt")],
        Some(&destination),
    );
    keep_both.policy = ConflictPolicy::KeepBoth;
    let record = undo_record_of(&keep_both);
    let context = OperationContext::default();

    let reversal = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    let restored = fs::read(source.join("Report.txt")).unwrap();
    let left_in_destination: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let redo = reversal.inverse.expect("the undo can be redone");
    let redone = block_on(undo(&redo, &context, |_| {})).expect("a redo");

    assert!(reversal.result.errors.is_empty(), "{:?}", reversal.result.errors);
    assert!(
        reversal.result.skipped.is_empty(),
        "{:?}",
        reversal.result.skipped
    );
    assert_eq!(restored, b"moved");
    assert_eq!(left_in_destination, ["Report.txt"]);
    assert!(redone.errors.is_empty(), "{:?}", redone.errors);
    assert!(!source.join("Report.txt").exists());
    assert_eq!(
        fs::read(destination.join("Report.txt")).unwrap(),
        b"already there"
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
}

#[test]
fn undoing_move_to_trash_restores_the_items_where_they_were() {
    require_private_trash();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Documents");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("report.txt"), b"report").unwrap();

    let record = undo_record_of(&request(TransferMode::Trash, &[&folder.join("report.txt")], None));
    let trashed = !folder.join("report.txt").exists();
    assert_in_recycle_bin(&folder.join("report.txt"), true);
    undo_fully(&record);

    assert!(trashed);
    assert_eq!(record.undo_label(), "Undo Delete");
    assert_eq!(fs::read(folder.join("report.txt")).unwrap(), b"report");
    assert_in_recycle_bin(&folder.join("report.txt"), false);
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
    assert_in_recycle_bin(&item, true);
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

    assert_eq!(labels, ["Undo Duplicate", "Undo New"]);
    assert!(!temp.path().join("New folder").exists());
    assert!(!temp.path().join("a - Copy.txt").exists());
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a");
    assert_in_recycle_bin(&temp.path().join("a - Copy.txt"), true);
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

    assert_eq!(record.undo_label(), "Undo Rename");
    assert_eq!(names_after_undo, [true, true]);
    assert!(redone.errors.is_empty(), "{:?}", redone.errors);
    assert_eq!(fs::read(temp.path().join("Trip 1.txt")).unwrap(), b"b.txt");
    assert_eq!(fs::read(temp.path().join("Trip 2.jpg")).unwrap(), b"a.jpg");
}

/// parity: OPS-029, OPS-050
#[test]
fn a_numbered_rename_is_undone_and_redone_as_one_step() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["b.txt", "a.jpg"] {
        fs::write(temp.path().join(name), name).unwrap();
    }
    let batch = NumberedRename {
        items: ["b.txt", "a.jpg"]
            .map(|name| BatchItem {
                uri: file_uri(&temp.path().join(name)),
                name: name.to_owned(),
                is_dir: false,
            })
            .to_vec(),
        name: "Trip".to_owned(),
        edited_name: "b.txt".to_owned(),
        edited_is_dir: false,
    };
    let context = OperationContext::default();

    let outcome = block_on(rename_numbered_items(&batch, &context)).expect("a valid name");
    let record = outcome.undo.expect("an undoable rename");
    let reversal = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    let names_after_undo = [
        temp.path().join("b.txt").is_file(),
        temp.path().join("a.jpg").is_file(),
    ];
    let redo = reversal.inverse.expect("the undo can be redone");
    let redone = block_on(undo(&redo, &context, |_| {})).expect("a redo");

    assert_eq!(record.undo_label(), "Undo Rename");
    assert_eq!(names_after_undo, [true, true]);
    assert!(redone.errors.is_empty(), "{:?}", redone.errors);
    assert_eq!(fs::read(temp.path().join("Trip (1).txt")).unwrap(), b"b.txt");
    assert_eq!(fs::read(temp.path().join("Trip (2).jpg")).unwrap(), b"a.jpg");
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
    let made = links.join("a.txt - Shortcut").symlink_metadata().is_ok();
    undo_fully(&record);

    assert_eq!(record.undo_label(), "Undo Link");
    assert!(made, "the link is named as a shortcut");
    assert!(
        links.join("a.txt - Shortcut").symlink_metadata().is_err(),
        "the link is gone"
    );
    assert_eq!(fs::read(items.join("a.txt")).unwrap(), b"a", "its item stays");
}
