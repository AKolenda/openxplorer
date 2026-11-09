// SPDX-License-Identifier: AGPL-3.0-only
//! A paste in which some items cannot be done: everything that can be
//! copied or moved is, and each item that cannot is asked about in the
//! progress window (OPS-047), as Windows Explorer asks about it while the
//! rest of the job goes on. Nothing stops the whole paste before it
//! starts because one item is unreadable or gone. A folder pasted into
//! one of its own subfolders is refused, without Try Again (XFER-016).
//! Undo reverses only what was done. All names are fictional.
//!
//! An unreadable folder is unreadable only to an ordinary user; run as
//! root, those tests say so and pass without checking anything.

#[path = "ops_support/folders.rs"]
mod folders;
mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ox_core::ops::{
    reverse, run_resolved_transfer, scan_name_conflicts, FailureAsker, OperationContext, OpsError,
    TransferOutcome, UndoRecord,
};
use ox_core::transfer::{Cancellation, FailedItem, FailureAnswer, FailureCause, TransferMode};

use folders::Folders;
use ops_support::{block_on, file_uri};
use snapshots::{snapshot_protection, READ_ONLY};

/// Writes `contents` at `path`, making its folders.
fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// A folder made unreadable (mode 000) for as long as it lives, and
/// readable again afterwards, so the temporary folder can be removed.
struct Locked(PathBuf);

impl Locked {
    /// Makes the folder at `path`, with a file in it, and locks it.
    fn new(path: &Path) -> Self {
        write(&path.join("secret.txt"), b"secret");
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        Self(path.to_owned())
    }

    /// False when this user can read the folder all the same (root).
    fn holds(&self) -> bool {
        let holds = fs::read_dir(&self.0).is_err();
        if !holds {
            eprintln!("skipped: this user reads folders with mode 000 (root)");
        }
        holds
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// Every question asked, answered with `answer`.
#[derive(Clone, Default)]
struct Questions(Arc<Mutex<Vec<FailedItem>>>);

impl Questions {
    /// A context with the app's write protection, whose question about a
    /// failed item records it and answers `answer`.
    fn context(&self, answer: FailureAnswer) -> OperationContext {
        let asked = Arc::clone(&self.0);
        OperationContext {
            item_failure: Some(FailureAsker::new(move |item: &FailedItem| {
                asked.lock().unwrap().push(item.clone());
                answer
            })),
            ..OperationContext::new(snapshot_protection())
        }
    }

    /// The names and causes asked about.
    fn asked(&self) -> Vec<(String, FailureCause)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|item| (item.name.clone(), item.cause))
            .collect()
    }
}

/// Pastes `names` of `from` into `into` as `mode`, as the app does: the
/// name-conflict check, then the run.
fn paste(
    mode: TransferMode,
    from: &Path,
    names: &[&str],
    into: &Path,
    context: &OperationContext,
) -> TransferOutcome {
    let uris: Vec<String> = names.iter().map(|name| file_uri(&from.join(name))).collect();
    let scan = block_on(scan_name_conflicts(
        mode,
        &uris,
        &file_uri(into),
        &Cancellation::new(),
    ))
    .expect("a scan");
    block_on(run_resolved_transfer(&scan, &[], context, |_| {})).expect("the paste runs")
}

/// A copied folder with an unreadable subfolder: the rest is copied and
/// the subfolder alone is asked about, as "Folder Access Denied". Skip
/// leaves only it out, and Undo removes only the copy.
///
/// parity: OPS-047
#[test]
fn an_unreadable_subfolder_is_asked_about_and_the_rest_is_copied() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("CopyTest/ok.txt"), b"readable");
    let locked = Locked::new(&src.join("CopyTest/Locked"));
    if !locked.holds() {
        return;
    }
    let questions = Questions::default();

    let outcome = paste(
        TransferMode::Copy,
        &src,
        &["CopyTest"],
        &dst,
        &questions.context(FailureAnswer::Skip),
    );

    assert_eq!(fs::read(dst.join("CopyTest/ok.txt")).unwrap(), b"readable");
    assert!(!dst.join("CopyTest/Locked").exists(), "skipped");
    assert_eq!(
        questions.asked(),
        [("CopyTest/Locked".to_owned(), FailureCause::AccessDenied)]
    );
    assert_eq!(
        outcome.undo,
        Some(UndoRecord::Copy {
            copies: vec![file_uri(&dst.join("CopyTest"))]
        })
    );
}

/// The same two levels down, beside another selected item.
///
/// parity: OPS-047
#[test]
fn a_nested_unreadable_folder_is_asked_about_by_its_path() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("CopyTest/Deep/ok.txt"), b"readable");
    write(&src.join("notes.txt"), b"notes");
    let locked = Locked::new(&src.join("CopyTest/Deep/Locked"));
    if !locked.holds() {
        return;
    }
    let questions = Questions::default();

    paste(
        TransferMode::Copy,
        &src,
        &["CopyTest", "notes.txt"],
        &dst,
        &questions.context(FailureAnswer::Skip),
    );

    assert!(dst.join("CopyTest/Deep/ok.txt").is_file());
    assert!(dst.join("notes.txt").is_file());
    assert_eq!(
        questions.asked(),
        [("CopyTest/Deep/Locked".to_owned(), FailureCause::AccessDenied)]
    );
}

/// A moved folder whose subfolder cannot be read cannot be checked for
/// protected locations (XFER-020), so it is asked about as one item and
/// stays where it is; the other items move all the same, and Undo moves
/// only those back.
///
/// parity: OPS-047, XFER-020
#[test]
fn a_moved_folder_with_an_unreadable_subfolder_does_not_stop_the_others() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("CopyTest/ok.txt"), b"readable");
    write(&src.join("notes.txt"), b"notes");
    let locked = Locked::new(&src.join("CopyTest/Locked"));
    if !locked.holds() {
        return;
    }
    let questions = Questions::default();
    let context = questions.context(FailureAnswer::Skip);

    let outcome = paste(
        TransferMode::Move,
        &src,
        &["CopyTest", "notes.txt"],
        &dst,
        &context,
    );

    assert!(dst.join("notes.txt").is_file(), "moved");
    assert!(src.join("CopyTest/ok.txt").is_file(), "kept");
    assert_eq!(
        questions.asked(),
        [("CopyTest".to_owned(), FailureCause::AccessDenied)]
    );
    let record = outcome.undo.expect("an undoable move");
    block_on(reverse(&record, &context, |_| {})).expect("an undo");
    assert!(src.join("notes.txt").is_file(), "moved back");
    assert!(src.join("CopyTest/ok.txt").is_file());
    assert!(!dst.join("CopyTest").exists());
}

/// A cut item deleted before the paste: the others move, the missing one
/// is asked about as "Item Not Found", and Undo moves back only what
/// moved.
///
/// parity: OPS-047
#[test]
fn a_missing_item_is_asked_about_and_the_others_move() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("A.txt"), b"a");
    write(&src.join("B.txt"), b"b");
    write(&src.join("C.txt"), b"c");
    let uris: Vec<String> = ["A.txt", "B.txt", "C.txt"]
        .iter()
        .map(|name| file_uri(&src.join(name)))
        .collect();
    let scan = block_on(scan_name_conflicts(
        TransferMode::Move,
        &uris,
        &file_uri(&dst),
        &Cancellation::new(),
    ))
    .expect("a scan");
    // Deleted from outside the app after it was cut.
    fs::remove_file(src.join("B.txt")).unwrap();
    fs::remove_file(src.join("C.txt")).unwrap();
    let questions = Questions::default();
    let context = questions.context(FailureAnswer::Skip);

    let outcome = block_on(run_resolved_transfer(&scan, &[], &context, |_| {})).expect("the paste runs");

    assert_eq!(fs::read(dst.join("A.txt")).unwrap(), b"a");
    assert_eq!(
        questions.asked(),
        [
            ("B.txt".to_owned(), FailureCause::NotFound),
            ("C.txt".to_owned(), FailureCause::NotFound)
        ]
    );
    let record = outcome.undo.expect("an undoable move");
    block_on(reverse(&record, &context, |_| {})).expect("an undo");
    assert_eq!(fs::read(src.join("A.txt")).unwrap(), b"a");
    assert!(!src.join("B.txt").exists() && !dst.join("B.txt").exists());
}

/// A folder pasted into its own subfolder is refused for what it is, not
/// as an unexpected error the user could try again, and nothing moves.
///
/// parity: XFER-016, OPS-047
#[test]
fn a_folder_pasted_into_its_own_subfolder_is_refused_as_such() {
    for mode in [TransferMode::Move, TransferMode::Copy] {
        let folders = Folders::new();
        let src = folders.source();
        write(&src.join("Projects/Sub/plan.txt"), b"plan");
        let questions = Questions::default();

        let outcome = paste(
            mode,
            &src,
            &["Projects"],
            &src.join("Projects/Sub"),
            &questions.context(FailureAnswer::Skip),
        );

        let asked = questions.asked();
        assert_eq!(asked.len(), 1, "{mode:?}: {asked:?}");
        assert_ne!(
            asked[0].1,
            FailureCause::Other,
            "{mode:?}: not an unexpected error"
        );
        assert_eq!(asked[0].1, FailureCause::InsideSource, "{mode:?}");
        assert!(src.join("Projects/Sub/plan.txt").is_file());
        assert!(!src.join("Projects/Sub/Projects").exists());
        assert_eq!(outcome.undo, None, "{mode:?}");
    }
}

/// A missing item does not loosen XFER-020: a selected folder holding a
/// protected snapshot still stops the whole move before anything moves.
///
/// parity: XFER-020
#[test]
fn a_protected_folder_still_stops_a_move_with_a_missing_item() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("A.txt"), b"a");
    write(&src.join("B.txt"), b"b");
    write(&src.join("project/sub/.snapshot/version.txt"), b"kept");
    let uris: Vec<String> = ["A.txt", "B.txt", "project"]
        .iter()
        .map(|name| file_uri(&src.join(name)))
        .collect();
    let scan = block_on(scan_name_conflicts(
        TransferMode::Move,
        &uris,
        &file_uri(&dst),
        &Cancellation::new(),
    ))
    .expect("a scan");
    fs::remove_file(src.join("B.txt")).unwrap();
    let questions = Questions::default();

    let refused = block_on(run_resolved_transfer(
        &scan,
        &[],
        &questions.context(FailureAnswer::Skip),
        |_| {},
    ));

    assert_eq!(refused.map(|_| ()), Err(OpsError::Failed(READ_ONLY.into())));
    assert!(src.join("A.txt").is_file(), "nothing moved");
    assert!(questions.asked().is_empty());
}

/// A folder merged into the folder of its name is planned entry by entry,
/// so a part of it that cannot be read when the paste runs stops the
/// whole paste before anything moves: a protected snapshot in it must
/// keep the whole selected tree as it is (XFER-020). Here the folder can
/// still be passed through but no longer listed (mode 0300).
///
/// parity: XFER-020
#[test]
fn a_merged_folder_that_cannot_be_listed_moves_nothing() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("Projects/a.txt"), b"a");
    write(&src.join("Projects/sub/.snapshot/version.txt"), b"kept");
    fs::create_dir_all(dst.join("Projects")).unwrap();
    let uris = vec![file_uri(&src.join("Projects"))];
    let scan = block_on(scan_name_conflicts(
        TransferMode::Move,
        &uris,
        &file_uri(&dst),
        &Cancellation::new(),
    ))
    .expect("a scan");
    fs::set_permissions(src.join("Projects"), fs::Permissions::from_mode(0o300)).unwrap();
    let unlisted = fs::read_dir(src.join("Projects")).is_err();
    let questions = Questions::default();

    let outcome = block_on(run_resolved_transfer(
        &scan,
        &[],
        &questions.context(FailureAnswer::Skip),
        |_| {},
    ));

    fs::set_permissions(src.join("Projects"), fs::Permissions::from_mode(0o755)).unwrap();
    if !unlisted {
        eprintln!("skipped: this user lists folders with mode 0300 (root)");
        return;
    }
    assert!(outcome.is_err(), "{outcome:?}");
    assert!(src.join("Projects/a.txt").is_file(), "nothing moved");
    assert!(!dst.join("Projects/a.txt").exists());
}

/// A folder the copy could not list is asked about; once the user made it
/// readable and answered Try Again, what it holds is checked like the
/// rest of the tree, so a protected snapshot in it stops the whole item
/// and nothing of it is published (XFER-020).
///
/// parity: XFER-020, OPS-047
#[test]
fn a_retried_folder_with_a_snapshot_inside_publishes_nothing() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("CopyTest/ok.txt"), b"readable");
    write(&src.join("CopyTest/Locked/.snapshot/version.txt"), b"kept");
    let locked = Locked(src.join("CopyTest/Locked"));
    fs::set_permissions(&locked.0, fs::Permissions::from_mode(0o000)).unwrap();
    if !locked.holds() {
        return;
    }
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let path = locked.0.clone();
    let context = OperationContext {
        item_failure: Some(FailureAsker::new(move |item: &FailedItem| {
            let mut seen = seen.lock().unwrap();
            seen.push((item.name.clone(), item.cause));
            if seen.len() == 1 {
                // The user makes the folder readable and tries again.
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                FailureAnswer::Retry
            } else {
                FailureAnswer::Skip
            }
        })),
        ..OperationContext::new(snapshot_protection())
    };

    paste(TransferMode::Copy, &src, &["CopyTest"], &dst, &context);

    assert!(
        !dst.join("CopyTest/Locked/.snapshot").exists(),
        "the snapshot is never copied"
    );
    assert!(!dst.join("CopyTest").exists(), "nothing of the item is published");
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked.first().map(|(_, cause)| *cause),
        Some(FailureCause::AccessDenied)
    );
}

/// A folder whose name has characters a URI escapes (`#`, `?`) is copied
/// whole under the app's own previous-versions protection: every location
/// it is asked about is a proper URI.
///
/// parity: XFER-020
#[test]
fn a_folder_named_with_a_hash_or_question_mark_copies_under_the_protection() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("Plan #1/notes?.txt"), b"plan");
    write(&src.join("Plan #1/Sub #2/item.txt"), b"item");
    let settings = tempfile::tempdir().unwrap();
    let versions = std::sync::Arc::new(ox_core::versions::PreviousVersions::new(settings.path()));
    let questions = Questions::default();
    let context = OperationContext {
        protection: ox_core::ops::WriteProtection::new(versions.write_guard()),
        ..questions.context(FailureAnswer::Skip)
    };

    paste(TransferMode::Copy, &src, &["Plan #1"], &dst, &context);

    assert_eq!(questions.asked(), []);
    assert_eq!(fs::read(dst.join("Plan #1/notes?.txt")).unwrap(), b"plan");
    assert_eq!(fs::read(dst.join("Plan #1/Sub #2/item.txt")).unwrap(), b"item");
}

/// An item deleted before Paste whose name the destination already has
/// does not stop the check for name conflicts: the others are pasted and
/// the missing one is asked about.
///
/// parity: OPS-047, OPS-026
#[test]
fn an_item_missing_before_the_check_does_not_stop_the_paste() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("A.txt"), b"a");
    write(&dst.join("B.txt"), b"existing");
    let questions = Questions::default();

    let outcome = paste(
        TransferMode::Copy,
        &src,
        &["A.txt", "B.txt"],
        &dst,
        &questions.context(FailureAnswer::Skip),
    );

    assert_eq!(fs::read(dst.join("A.txt")).unwrap(), b"a");
    assert_eq!(fs::read(dst.join("B.txt")).unwrap(), b"existing");
    assert_eq!(questions.asked(), [("B.txt".to_owned(), FailureCause::NotFound)]);
    assert_eq!(
        outcome.undo,
        Some(UndoRecord::Copy {
            copies: vec![file_uri(&dst.join("A.txt"))]
        })
    );
}
