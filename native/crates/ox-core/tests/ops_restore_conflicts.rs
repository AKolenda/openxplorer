// SPDX-License-Identifier: AGPL-3.0-only
//! Restore from the Recycle Bin onto names that are taken again, answered
//! as Windows 11's "Replace or Skip Files" question answers it, on the
//! test run's private Recycle Bin (OPS-041, OPS-026, OPS-028).
//!
//! The tests share one Recycle Bin, so they run one at a time.

mod ops_support;
#[path = "ops_support/private_trash.rs"]
mod private_trash;

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ox_core::ops::{
    list_recycle_bin, restore_answered_items, restore_free_items, restores_into_one_folder, reverse,
    run_transfer, scan_restore_conflicts, summarize_restore, ConflictScan, OperationContext,
    OperationSummary, PlannedKind, RecycledItem, Resolution, TransferOutcome, TransferRequest, UndoRecord,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;

/// Lets one test at a time use the shared Recycle Bin.
static RECYCLE_BIN: Mutex<()> = Mutex::new(());

fn use_recycle_bin() -> MutexGuard<'static, ()> {
    require_private_trash();
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

/// Every Recycle Bin item that came from `path`, waiting up to five
/// seconds for `count` of them: the backend may list an item late.
fn recycled_from(path: &Path, count: usize) -> Vec<RecycledItem> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items: Vec<RecycledItem> = block_on(list_recycle_bin(&Cancellation::new()))
            .expect("a readable Recycle Bin")
            .into_iter()
            .filter(|item| item.original_path.as_deref() == Some(path))
            .collect();
        if items.len() >= count {
            return items;
        }
        assert!(
            Instant::now() < deadline,
            "{} is not in the Recycle Bin",
            path.display()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn scan(uris: &[String]) -> ConflictScan {
    block_on(scan_restore_conflicts(uris, &Cancellation::new())).expect("a plan")
}

/// Restores `scan` with `answers`, as the window does: the free items
/// first, then the answered ones.
fn restore(scan: &ConflictScan, answers: &[Resolution], context: &OperationContext) -> TransferOutcome {
    let so_far = block_on(restore_free_items(scan, context)).expect("the free items");
    block_on(restore_answered_items(scan, answers, so_far, context)).expect("the answered items")
}

/// A taken name is a conflict of the plan, named as the original folder
/// names it; Skip leaves the item in the Recycle Bin and the newer file in
/// place, and nothing is reported, as in Windows.
///
/// parity: OPS-041, OPS-026
#[test]
fn skip_leaves_the_item_in_the_recycle_bin_without_a_report() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    let plan = temp.path().join("plan.txt");
    fs::write(&notes, b"trashed").unwrap();
    fs::write(&plan, b"plan").unwrap();
    trash(&[&notes, &plan]);
    fs::write(&notes, b"newer").unwrap();
    let uris = [
        recycled_from(&notes, 1)[0].uri.clone(),
        recycled_from(&plan, 1)[0].uri.clone(),
    ];

    let plan_scan = scan(&uris);
    let outcome = restore(&plan_scan, &[Resolution::Skip], &OperationContext::default());

    assert_eq!(plan_scan.conflicts.len(), 1);
    assert_eq!(plan_scan.conflicts[0].name(), "notes.txt");
    assert_eq!(plan_scan.conflicts[0].existing_uri, file_uri(&notes));
    assert_eq!(plan_scan.destination_folder, file_uri(temp.path()));
    assert!(restores_into_one_folder(&plan_scan));
    assert_eq!(plan_scan.items[1].kind, PlannedKind::Free);
    assert_eq!(fs::read(&notes).unwrap(), b"newer");
    assert_eq!(fs::read(&plan).unwrap(), b"plan");
    assert_eq!(recycled_from(&notes, 1).len(), 1, "the skipped item stays");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(
        summarize_restore(&outcome.result),
        OperationSummary::Toast("1 item(s) restored.".into())
    );
}

/// Replace moves the newer file to the Recycle Bin and restores the old
/// one; Undo puts the restored file back in the Recycle Bin and brings
/// the replaced one back.
///
/// parity: OPS-041, OPS-028, OPS-029
#[test]
fn replace_keeps_the_replaced_file_in_the_recycle_bin_and_undo_brings_it_back() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    fs::write(&notes, b"trashed").unwrap();
    trash(&[&notes]);
    fs::write(&notes, b"newer").unwrap();
    let uri = recycled_from(&notes, 1)[0].uri.clone();

    let context = OperationContext::default();
    let outcome = restore(&scan(&[uri]), &[Resolution::Replace], &context);

    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(fs::read(&notes).unwrap(), b"trashed");
    assert_eq!(outcome.created, [file_uri(&notes)]);
    assert_eq!(
        recycled_from(&notes, 1).len(),
        1,
        "the newer file is in the Recycle Bin"
    );
    let Some(record @ UndoRecord::Replace { .. }) = outcome.undo else {
        panic!("a Replace record: {:?}", outcome.undo);
    };
    assert_eq!(record.operation(), ox_core::ops::Operation::Restore);

    let reversal = block_on(reverse(&record, &context, |_| {})).expect("Undo runs");
    assert!(reversal.result.errors.is_empty(), "{:?}", reversal.result.errors);
    assert_eq!(fs::read(&notes).unwrap(), b"newer");
    assert_eq!(
        recycled_from(&notes, 1).len(),
        1,
        "the restored file is back in the Recycle Bin"
    );
}

/// Both kept: two items trashed from the same place, restored over a
/// third file of their name, take Windows' numbers in their original
/// folder, never the Recycle Bin's own names for them.
///
/// parity: OPS-041, OPS-028
#[test]
fn keeping_both_numbers_the_restored_items_by_their_original_name() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    for contents in [b"first".as_slice(), b"second".as_slice()] {
        fs::write(&notes, contents).unwrap();
        trash(&[&notes]);
    }
    fs::write(&notes, b"newer").unwrap();
    let uris: Vec<String> = recycled_from(&notes, 2)
        .into_iter()
        .map(|item| item.uri)
        .collect();

    let plan = scan(&uris);
    let outcome = restore(&plan, &[Resolution::KeepBoth; 2], &OperationContext::default());

    assert_eq!(plan.conflicts.len(), 2);
    assert!(plan
        .conflicts
        .iter()
        .all(|conflict| conflict.name() == "notes.txt"));
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(fs::read(&notes).unwrap(), b"newer");
    let mut kept = [
        fs::read(temp.path().join("notes (2).txt")).unwrap(),
        fs::read(temp.path().join("notes (3).txt")).unwrap(),
    ];
    kept.sort();
    assert_eq!(kept, [b"first".to_vec(), b"second".to_vec()]);
    assert_eq!(
        outcome.undo,
        Some(UndoRecord::Restore {
            restored: outcome.created.clone()
        })
    );
}

/// Replace never puts a file in place of a folder: the item stays in the
/// Recycle Bin and the report says why.
///
/// parity: OPS-041
#[test]
fn replace_never_puts_a_file_in_place_of_a_folder() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let report = temp.path().join("Report");
    fs::write(&report, b"a file").unwrap();
    trash(&[&report]);
    fs::create_dir(&report).unwrap();
    let uri = recycled_from(&report, 1)[0].uri.clone();

    let plan = scan(&[uri]);
    let outcome = restore(&plan, &[Resolution::Replace], &OperationContext::default());

    assert!(plan.conflicts[0].existing.is_folder);
    assert!(report.is_dir());
    assert_eq!(
        outcome.result.errors,
        [
            "Report: The item named “Report” in its original folder cannot be replaced. \
          It was left in the Recycle Bin."
        ]
    );
    assert_eq!(recycled_from(&report, 1).len(), 1);
}

/// A cancelled question keeps what was restored and says nothing.
///
/// parity: OPS-041, OPS-023
#[test]
fn cancel_at_the_question_keeps_the_free_items_and_says_nothing() {
    let _recycle_bin = use_recycle_bin();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes.txt");
    let plan = temp.path().join("plan.txt");
    fs::write(&notes, b"trashed").unwrap();
    fs::write(&plan, b"plan").unwrap();
    trash(&[&notes, &plan]);
    fs::write(&notes, b"newer").unwrap();
    let uris = [
        recycled_from(&notes, 1)[0].uri.clone(),
        recycled_from(&plan, 1)[0].uri.clone(),
    ];

    let so_far = block_on(restore_free_items(&scan(&uris), &OperationContext::default())).unwrap();
    let outcome = so_far.into_outcome();

    assert!(plan.is_file());
    assert_eq!(fs::read(&notes).unwrap(), b"newer");
    assert_eq!(outcome.created, [file_uri(&plan)]);
    assert_eq!(summarize_restore(&outcome.result), OperationSummary::Cancelled);
}
