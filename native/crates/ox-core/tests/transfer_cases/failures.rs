// SPDX-License-Identifier: AGPL-3.0-only
//! Failure injection at the copy, publication and reversible replacement
//! boundaries. Ports the failure cases of `TransferTests` in
//! `desktop/tests/test_operations.py` and adds the native adversarial cases.
//! The failing provider is [`Faults`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, MoveByCopyingItem, Node, NodeInfo, TransferError};

use crate::transfer_support::{
    faults::{Fault, Faults},
    local::{LocalNode, Provider},
    *,
};

/// The replacement backup left in `fixture`'s destination.
fn backup_in(fixture: &Fixture) -> PathBuf {
    let names = list(&fixture.destination_folder);
    let backup = names
        .iter()
        .find(|name| name.starts_with(".winspace-replaced-"))
        .expect("a backup is left in the destination");
    fixture.destination_folder.join(backup)
}

/// A copy that fails midway, even one the backend itself reports as
/// cancelled, keeps the existing item and removes its private staging.
///
/// parity: XFER-001, XFER-002
#[test]
fn failed_or_backend_cancelled_copies_keep_the_original_and_remove_private_staging() {
    for fault in [Fault::CopyInterrupted, Fault::BackendCancelled] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("document");
        write(&source, "complete original");
        write(&fixture.destination_folder.join("document"), "prior destination");
        let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

        let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

        assert!(
            !result.cancelled,
            "backend cancellation is an error, not a user request"
        );
        assert_eq!(result.errors.len(), 1);
        assert_eq!(read(&source), "complete original");
        assert_eq!(
            read(&fixture.destination_folder.join("document")),
            "prior destination"
        );
        fixture.assert_no_staging();
    }
}

/// Ported from `desktop/tests/test_operations.py::TransferTests::test_move_failure_does_not_copy_delete`:
/// a move the backend cannot do is refused and the source kept, both
/// without a question installed and when the user declines to finish it
/// by copying, who is asked once for the whole operation.
///
/// parity: XFER-011
#[test]
fn native_move_failure_never_degrades_to_copy_then_delete() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("document");
    let second = fixture.source_folder.join("second");
    write(&source, "original");
    write(&second, "second");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let declining = fixture
        .engine(Faults::new(Fault::MoveUnsupported, &fixture.cancel))
        .with_move_by_copying_question(move |item: &MoveByCopyingItem| {
            log.lock().expect("question log").push(item.clone());
            false
        });

    for mut engine in [
        fixture.engine(Faults::new(Fault::MoveUnsupported, &fixture.cancel)),
        declining,
    ] {
        let result = fixture.run(
            &mut engine,
            &[&source, &second],
            Request::Move(ConflictPolicy::Skip),
        );

        assert_eq!(
            result.errors,
            [
                "document: Native move unsupported.",
                "second: Native move unsupported."
            ]
        );
        assert!(result.done.is_empty());
        assert_eq!(read(&source), "original");
        assert!(list(&fixture.destination_folder).is_empty());
        fixture.assert_no_staging();
    }
    let destination = fixture
        .destination_folder
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected = MoveByCopyingItem {
        name: "document".into(),
        destination,
    };
    assert_eq!(*asked.lock().unwrap(), [expected]);
}

/// Ported from `desktop/tests/test_operations.py::TransferTests::test_preflight_race_never_overwrites`: a name another program
/// creates after the conflict check is not overwritten when the copy is
/// published.
///
/// parity: XFER-007
#[test]
fn a_name_taken_while_copying_is_not_overwritten_at_publication() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("document");
    write(&source, "original");
    let mut engine = fixture.engine(Faults::new(Fault::PublishRace, &fixture.cancel));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert_eq!(result.errors.len(), 1);
    assert!(result.done.is_empty());
    assert_eq!(read(&source), "original");
    assert_eq!(read(&fixture.destination_folder.join("document")), "racing file");
    fixture.assert_no_staging();
}

/// A local destination that stops answering once a copy fails, like
/// `Unreachable` in `test_local_stage_query_error_is_still_reported`.
#[derive(Default)]
struct UnreachableAfterFailure {
    unreachable: AtomicBool,
}

impl Provider for UnreachableAfterFailure {
    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        let unreachable = self.unreachable.load(Ordering::SeqCst);
        if unreachable && is_staging(node) {
            return Err(TransferError::failed("Input/output error"));
        }
        node.local_info(cancel)
    }

    fn copy_file(
        &self,
        _node: &LocalNode,
        _target: &dyn Node,
        _cancel: &Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        self.unreachable.store(true, Ordering::SeqCst);
        Err(TransferError::failed("Input/output error"))
    }
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_local_stage_query_error_is_still_reported`: local staging
/// that cannot even be queried gets one cleanup attempt and is reported
/// with its exact location.
///
/// parity: XFER-003
#[test]
fn a_local_stage_that_cannot_be_queried_is_still_reported() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "data");

    let result = fixture.copy(
        Arc::new(UnreachableAfterFailure::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    let names = list(&fixture.destination_folder);
    assert_eq!(names.len(), 1, "{names:?}");
    let stage = fixture.destination_folder.join(&names[0]);
    let report = format!("Incomplete staging folder left at {}", file_uri(&stage));
    assert!(
        result.errors.iter().any(|error| error.contains(&report)),
        "{result:?}"
    );
    assert!(fixture.sleeps().is_empty());
}

/// A staging folder another program created first is never cleaned up.
///
/// parity: XFER-002
#[test]
fn failed_stage_reservation_grants_no_cleanup_rights() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("document");
    write(&source, "original");
    let mut engine = fixture.engine(Faults::new(Fault::UnownedStage, &fixture.cancel));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert_eq!(result.errors.len(), 1);
    let leftovers = list(&fixture.destination_folder);
    assert_eq!(leftovers.len(), 1);
    assert_eq!(
        read(&fixture.destination_folder.join(&leftovers[0]).join("not-ours")),
        "another creator owns this folder"
    );
}

/// Ports `test_replace_fallback_restores_old_file_if_install_fails`, for
/// copies and moves, and for installs that fail, pretend to succeed, or
/// follow a move aside that reported an error after finishing.
///
/// parity: XFER-010
#[test]
fn replacement_install_failure_or_false_success_restores_the_old_name() {
    for fault in [Fault::Install, Fault::NoOpInstall, Fault::AsideAfterSuccess] {
        for request in [
            Request::Copy(ConflictPolicy::Replace),
            Request::Move(ConflictPolicy::Replace),
        ] {
            let fixture = Fixture::new();
            let source = fixture.replacement_source("document", "incoming", "original");
            let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

            let result = fixture.run(&mut engine, &[&source], request);

            assert!(result.done.is_empty(), "{result:?}");
            assert_eq!(result.errors.len(), 1, "{result:?}");
            assert_eq!(read(&fixture.destination_folder.join("document")), "original");
            assert_eq!(read(&source), "incoming");
            assert!(fixture.leftovers().is_empty());
        }
    }
}

/// When the old file cannot be put back, the message names the backup
/// that holds it, and the backup keeps its contents.
///
/// parity: XFER-003, XFER-010
#[test]
fn rollback_failure_reports_the_exact_backup_and_preserves_its_contents() {
    let fixture = Fixture::new();
    let source = fixture.replacement_source("document", "incoming", "original");
    let mut engine = fixture.engine(Faults::new(Fault::InstallAndRestore, &fixture.cancel));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

    let leftovers = list(&fixture.destination_folder);
    assert_eq!(leftovers.len(), 1);
    let backup = backup_in(&fixture);
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&file_uri(&backup)));
    assert_eq!(read(&source), "incoming");
    fixture.assert_no_staging();
}

/// Once the old file is aside, the small rest of the commit finishes even
/// though the user cancelled; later items are not started.
///
/// parity: OPS-022, XFER-010
#[test]
fn cancellation_after_move_aside_finishes_the_small_commit_without_losing_the_old_name() {
    let fixture = Fixture::new();
    let source = fixture.replacement_source("document", "incoming", "original");
    let later = fixture.source_folder.join("later");
    write(&later, "later");
    let mut engine = fixture.engine(Faults::new(Fault::CancelAfterAside, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source, &later],
        Request::Copy(ConflictPolicy::Replace),
    );

    assert!(result.cancelled);
    assert_eq!(result.done, [file_uri(&source)]);
    assert_eq!(read(&fixture.destination_folder.join("document")), "incoming");
    assert!(!fixture.destination_folder.join("later").exists());
    assert!(fixture.leftovers().is_empty());
}

/// A backup that cannot be deleted after a successful replacement is
/// reported with its location, and the new file stays.
///
/// parity: XFER-003, XFER-010
#[test]
fn backup_cleanup_failure_reports_the_original_and_keeps_the_new_file() {
    let fixture = Fixture::new();
    let source = fixture.replacement_source("document", "incoming", "original");
    let mut engine = fixture.engine(Faults::new(Fault::BackupCleanup, &fixture.cancel));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

    assert_eq!(read(&fixture.destination_folder.join("document")), "incoming");
    let backup = backup_in(&fixture);
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&file_uri(&backup)));
    fixture.assert_no_staging();
}

/// A cancelled run still reports a backup the user must recover by hand.
///
/// parity: OPS-022, XFER-010
#[test]
fn cancellation_never_hides_a_backup_that_requires_manual_recovery() {
    for fault in [Fault::CancelAndCleanup, Fault::CancelAndRollback] {
        let fixture = Fixture::new();
        let source = fixture.replacement_source("document", "incoming", "original");
        let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

        let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

        assert!(result.cancelled);
        assert_eq!(result.errors.len(), 1, "{result:?}");
        let backup = backup_in(&fixture);
        assert_eq!(read(&backup), "original");
        assert!(result.errors[0].contains(&file_uri(&backup)), "{result:?}");
        assert_eq!(read(&source), "incoming");
        fixture.assert_no_staging();
    }
}
