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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ox_core::ops::{
    delete_from_recycle_bin, empty_recycle_bin, list_recycle_bin, recycle_bin_item_count,
    restore_from_recycle_bin, run_transfer, undo, FailureAsker, OperationContext, RecycledItem,
    TransferRequest, UndoRecord, WriteProtection,
};
use ox_core::transfer::{
    Cancellation, ConflictPolicy, FailedItem, FailureAnswer, FailureCause, TransferMode,
};

use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;
use snapshots::{snapshot_protection, READ_ONLY};

/// A context whose question about failed items answers `answer` and
/// keeps every question it was asked.
fn asking_context(answer: FailureAnswer) -> (OperationContext, Arc<Mutex<Vec<FailedItem>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let context = OperationContext {
        item_failure: Some(FailureAsker::new(move |item: &FailedItem| {
            log.lock().expect("question log").push(item.clone());
            answer
        })),
        ..OperationContext::default()
    };
    (context, asked)
}

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

/// A restore asks about an item that fails while it runs, in the progress
/// window, as Windows does, so a later Cancel hides nothing and nothing
/// is said once it ends.
///
/// parity: OPS-023, OPS-047
#[test]
fn a_restore_asks_about_a_failed_item_while_it_runs() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    let plan = temp.path().join("plan.txt");
    fs::write(&notes, b"trashed").unwrap();
    fs::write(&plan, b"plan").unwrap();

    trash(&[&notes, &plan]);
    fs::write(&notes, b"newer").unwrap();
    let items = [recycled_from(&notes).uri, recycled_from(&plan).uri];
    let cancel = Cancellation::new();
    // The user cancels while the second item is restoring.
    let cancel_at_plan = {
        let cancel = cancel.clone();
        let plan = file_uri(&plan);
        move |uri: &str| {
            if uri == plan {
                cancel.cancel();
            }
            Ok(())
        }
    };
    let (asking, asked) = asking_context(FailureAnswer::Skip);
    let context = OperationContext {
        cancel,
        item_failure: asking.item_failure,
        ..OperationContext::new(WriteProtection::new(cancel_at_plan))
    };
    let outcome = block_on(restore_from_recycle_bin(&items, &context)).unwrap();

    let refusal = "notes.txt: An item named “notes.txt” already exists in its original folder. \
                   It was left in the Recycle Bin.";
    assert!(outcome.result.cancelled);
    assert_eq!(outcome.result.errors, [refusal]);
    assert_eq!(outcome.result.presented, [refusal], "asked while it ran");
    let asked = asked.lock().expect("question log");
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].name, "notes.txt");
    assert_eq!(asked[0].mode, TransferMode::Move);
    assert!(!plan.exists(), "the cancelled item stays in the Recycle Bin");
}

/// Deleting an item that left the Recycle Bin meanwhile asks "Item Not
/// Found" about it, as Windows does: Try Again, Skip or Cancel, here
/// Skip; the other items are still deleted.
///
/// parity: OPS-043, OPS-047, OPS-023
#[test]
fn deleting_an_item_gone_from_the_recycle_bin_asks_about_it() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let (gone, kept) = (temp.path().join("gone.txt"), temp.path().join("chosen.txt"));
    fs::write(&gone, b"gone").unwrap();
    fs::write(&kept, b"chosen").unwrap();
    trash(&[&gone, &kept]);
    let (gone_item, chosen_item) = (recycled_from(&gone), recycled_from(&kept));
    // Another program empties the item out of the Recycle Bin first.
    block_on(delete_from_recycle_bin(
        std::slice::from_ref(&gone_item.uri),
        &OperationContext::default(),
    ))
    .unwrap();
    let (context, asked) = asking_context(FailureAnswer::Skip);

    let uris = [gone_item.uri.clone(), chosen_item.uri.clone()];
    let result = block_on(delete_from_recycle_bin(&uris, &context)).unwrap();

    let asked = asked.lock().expect("question log");
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].cause, FailureCause::NotFound);
    assert_eq!(asked[0].mode, TransferMode::Delete);
    assert_eq!(asked[0].uri, gone_item.uri);
    assert!(asked[0].more_items, "Do this for all current items");
    assert_eq!(result.done, [chosen_item.uri]);
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(result.presented, result.errors, "the user answered it");
    assert!(!result.cancelled);
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
        &OperationContext::default(),
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
/// Recycle Bin. Pin its private files folder's time to reproduce a
/// filesystem whose timestamp does not advance between the operations.
#[test]
fn an_item_trashed_after_deleting_one_with_the_same_name_is_listed() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let draft = temp.path().join("draft.txt");
    fs::write(&draft, b"first").unwrap();
    trash(&[&draft]);
    let first = recycled_from(&draft);

    let deleted = block_on(delete_from_recycle_bin(
        &[first.uri],
        &OperationContext::default(),
    ))
    .unwrap();
    let trash_files = fs::File::open(glib::user_data_dir().join("Trash/files")).unwrap();
    let after_delete = trash_files.metadata().unwrap().modified().unwrap();
    fs::write(&draft, b"second").unwrap();
    trash(&[&draft]);
    trash_files.set_modified(after_delete).unwrap();

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
    // A monitored Trash learns of additions asynchronously. The Empty
    // command is tested only once both fixture items are actually listed.
    recycled_from(&folder);
    recycled_from(&file);
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

    let result = block_on(empty_recycle_bin(&OperationContext::default())).unwrap();
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
        &OperationContext::default(),
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

    assert_eq!(record.undo_label(), "Undo Restore");
    assert_eq!(result.done, [file_uri(&draft)]);
    assert!(!draft.exists());
    assert_eq!(recycled_from(&draft).name, "draft.txt");
}

/// Puts `count` files named `<prefix> 00001.txt` and on straight into the
/// private home Trash, with their descriptions, as the freedesktop Trash
/// specification lays them out: much quicker than trashing each, so a
/// large Recycle Bin can be tested. Their original locations are in
/// `origin`, a temporary folder. Returns their original paths.
fn fill_home_trash(origin: &Path, prefix: &str, count: usize) -> Vec<std::path::PathBuf> {
    let trash = glib::user_data_dir().join("Trash");
    fs::create_dir_all(trash.join("files")).unwrap();
    fs::create_dir_all(trash.join("info")).unwrap();
    (1..=count)
        .map(|number| {
            let name = format!("{prefix} {number:05}.txt");
            let original = origin.join(&name);
            let description = format!(
                "[Trash Info]\nPath={}\nDeletionDate=2026-11-01T09:30:00\n",
                original.display().to_string().replace(' ', "%20")
            );
            // The description first, as the specification says a program
            // that trashes writes it, so the item is never listed without.
            fs::write(trash.join("info").join(format!("{name}.trashinfo")), description).unwrap();
            fs::write(trash.join("files").join(&name), b"old").unwrap();
            original
        })
        .collect()
}

/// Waits until the Recycle Bin lists at least `count` items, each with
/// its original location.
fn wait_for_listed(count: usize) -> Vec<RecycledItem> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let items = recycle_bin();
        if items.len() >= count && items.iter().all(|item| item.original_path.is_some()) {
            return items;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {count} items are listed",
            items.len()
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Watches `trash:///` on a thread of its own while it is held, as an open
/// window's Recycle Bin row does.
struct TrashWatcher {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TrashWatcher {
    fn start() -> Self {
        use gio::prelude::*;
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let (ready, is_ready) = std::sync::mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            let context = glib::MainContext::new();
            context
                .with_thread_default(|| {
                    let monitor = gio::File::for_uri("trash:///")
                        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                        .expect("the Trash backend watches trash:///");
                    monitor.connect_changed(|_, _, _, _| {});
                    ready.send(()).unwrap();
                    while stopped.try_recv().is_err() {
                        context.iteration(false);
                        thread::sleep(Duration::from_millis(5));
                    }
                    monitor.cancel();
                    while context.pending() {
                        context.iteration(false);
                    }
                })
                .unwrap();
        });
        is_ready.recv().unwrap();
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for TrashWatcher {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(thread::JoinHandle::join);
    }
}

/// Emptying a large Recycle Bin while a window watches it takes seconds,
/// not minutes: the items are removed from the Trash folder on disk
/// rather than one request each to the Trash backend, which listed the
/// whole Trash folder again after each one (201 s for 50,000 items).
///
/// parity: OPS-042
#[test]
fn emptying_a_large_recycle_bin_takes_seconds() {
    const ITEMS: usize = 6_000;
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let originals = fill_home_trash(temp.path(), "Big bin", ITEMS);
    let watcher = TrashWatcher::start();
    let before = wait_for_listed(ITEMS);
    let temp_root = std::env::temp_dir();
    assert!(
        before.iter().all(|item| item
            .original_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&temp_root))),
        "the Recycle Bin holds items these tests did not trash: {:?}",
        before.iter().find(|item| !item
            .original_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&temp_root)))
    );

    let started = Instant::now();
    let result = block_on(empty_recycle_bin(&OperationContext::default())).unwrap();
    let took = started.elapsed();
    drop(watcher);

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.done.len(), before.len());
    assert!(
        took < Duration::from_secs(8),
        "emptying {ITEMS} items took {took:?}"
    );
    let trash = glib::user_data_dir().join("Trash");
    assert_eq!(fs::read_dir(trash.join("files")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 0);
    assert!(recycle_bin().is_empty());
    assert!(
        originals.iter().all(|path| !path.exists()),
        "nothing was restored"
    );
}

/// A trashed folder holding read-only folders and files is deleted whole,
/// as the Trash backend of `GVfs` deletes it.
///
/// parity: OPS-042
#[test]
fn a_trashed_folder_with_read_only_folders_inside_is_emptied() {
    use std::os::unix::fs::PermissionsExt;
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Read-only project");
    let locked = folder.join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("read-only.txt"), b"kept").unwrap();
    fs::set_permissions(locked.join("read-only.txt"), fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    trash(&[&folder]);
    let item = recycled_from(&folder);

    let result = block_on(delete_from_recycle_bin(
        std::slice::from_ref(&item.uri),
        &OperationContext::default(),
    ))
    .unwrap();

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.done, [item.uri]);
    assert!(!glib::user_data_dir()
        .join("Trash/files/Read-only project")
        .exists());
    assert!(!recycle_bin()
        .iter()
        .any(|item| item.original_path.as_deref() == Some(folder.as_path())));
}

/// An item that can be deleted only in part, here a trashed folder nested
/// deeper than any deletion walks, is asked about; it stays in the
/// Recycle Bin with what is left of it, and the others are deleted.
///
/// parity: OPS-042, OPS-047
#[test]
fn an_item_emptying_cannot_delete_whole_stays_in_the_recycle_bin() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Deep project");
    let mut deepest = folder.clone();
    for _ in 0..=ox_core::transfer::MAX_DEPTH {
        deepest.push("d");
    }
    fs::create_dir_all(&deepest).unwrap();
    fs::write(deepest.join("deep.txt"), b"deep").unwrap();
    let file = temp.path().join("loose.txt");
    fs::write(&file, b"loose").unwrap();
    trash(&[&folder, &file]);
    let folder_item = recycled_from(&folder);
    let file_item = recycled_from(&file);
    let (context, asked) = asking_context(FailureAnswer::Skip);

    let uris = [folder_item.uri.clone(), file_item.uri.clone()];
    let result = block_on(delete_from_recycle_bin(&uris, &context)).unwrap();

    assert_eq!(result.done, [file_item.uri]);
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].uri, folder_item.uri);
    let left = recycled_from(&folder);
    assert_eq!(
        left.uri, folder_item.uri,
        "the folder is still listed, to delete or restore"
    );
    let stored = glib::user_data_dir().join("Trash/files/Deep project");
    let inside = deepest.strip_prefix(&folder).unwrap();
    assert!(
        stored.join(inside).join("deep.txt").exists(),
        "nothing below the limit went"
    );
    // Cleaned up for the tests that empty the Recycle Bin.
    fs::remove_dir_all(&stored).unwrap();
    fs::remove_file(glib::user_data_dir().join("Trash/info/Deep project.trashinfo")).unwrap();
}
