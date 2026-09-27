// SPDX-License-Identifier: AGPL-3.0-only
//! Failure injection at the copy, publication and reversible replacement
//! boundaries. Ports the failure cases of `TransferTests` in
//! `desktop/tests/test_operations.py` and adds the native adversarial cases.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeInfo, TransferError, TransferMode};

use crate::transfer_support::{
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// Where a [`Faults`] provider breaks the copy, publication or reversible
/// replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The device disconnects in the middle of a file copy.
    CopyInterrupted,
    /// The backend reports "cancelled" for a copy the user never cancelled.
    BackendCancelled,
    /// The backend cannot move natively at all.
    MoveUnsupported,
    /// Another program creates the final name just before publication.
    PublishRace,
    /// Installing the new file under the final name fails.
    Install,
    /// Installing fails, and so does putting the old file back.
    InstallAndRestore,
    /// The move aside finishes, but the backend reports an error.
    AsideAfterSuccess,
    /// The user cancels right after the old file was moved aside.
    CancelAfterAside,
    /// Like `CancelAfterAside`, and the backup cannot be deleted.
    CancelAndCleanup,
    /// Like `CancelAfterAside`, and both install and restore fail.
    CancelAndRollback,
    /// The replaced file's backup cannot be deleted.
    BackupCleanup,
    /// Another program creates the staging folder first.
    UnownedStage,
    /// Installing reports success without moving anything.
    NoOpInstall,
}

impl Fault {
    /// Faults that let the move aside finish and then interrupt the
    /// replacement. From then on the commit must not be cancellable.
    fn interrupts_after_aside(self) -> bool {
        matches!(
            self,
            Fault::CancelAfterAside
                | Fault::AsideAfterSuccess
                | Fault::CancelAndCleanup
                | Fault::CancelAndRollback
        )
    }

    /// Faults where the user cancels once the old file is aside.
    fn cancels_after_aside(self) -> bool {
        self.interrupts_after_aside() && self != Fault::AsideAfterSuccess
    }
}

/// The step of a publication or reversible replacement that a move is.
enum MoveStep {
    /// The old file is renamed to its backup name.
    Aside,
    /// The backup is renamed back to the old name.
    Restore,
    /// The new item is renamed to its final name.
    Install,
}

impl MoveStep {
    fn of(node: &LocalNode, target: &dyn Node) -> Self {
        if is_backup(target) {
            MoveStep::Aside
        } else if is_backup(node) {
            MoveStep::Restore
        } else {
            MoveStep::Install
        }
    }
}

/// A local provider that fails the way `fault` describes, like the failing
/// `LocalNode` subclasses in `desktop/tests/test_operations.py`. Direct
/// overwrite is never supported, so Replace always renames reversibly.
struct Faults {
    fault: Fault,
    /// The run's cancellation, which the `Cancel*` faults cancel.
    cancel: Cancellation,
}

impl Faults {
    fn new(fault: Fault, cancel: &Cancellation) -> Arc<Self> {
        Arc::new(Self {
            fault,
            cancel: cancel.clone(),
        })
    }

    fn move_aside(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if !self.fault.interrupts_after_aside() {
            return node.local_move_native(target, cancel);
        }
        assert!(cancel.is_none(), "the commit must not be interruptible");
        node.local_move_native(target, cancel)?;
        if self.fault == Fault::AsideAfterSuccess {
            return Err(TransferError::failed("Rename finished before timeout."));
        }
        self.cancel.cancel();
        Ok(())
    }

    fn restore(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if matches!(self.fault, Fault::InstallAndRestore | Fault::CancelAndRollback) {
            return Err(TransferError::failed("Restore failed."));
        }
        node.local_move_native(target, cancel)
    }

    fn install(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if self.fault.cancels_after_aside() {
            assert!(
                cancel.is_none(),
                "installation must finish after moving the old file aside"
            );
        }
        match self.fault {
            Fault::PublishRace => write(&local_path_of(target), "racing file"),
            Fault::Install | Fault::InstallAndRestore | Fault::CancelAndRollback => {
                return Err(TransferError::failed("Install failed."));
            }
            Fault::NoOpInstall => return Ok(()),
            _ => {}
        }
        node.local_move_native(target, cancel)
    }
}

impl Provider for Faults {
    fn mkdir(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_mkdir(cancel)?;
        if self.fault == Fault::UnownedStage && is_staging(node) {
            write(
                &node.local_path().join("not-ours"),
                "another creator owns this folder",
            );
            return Err(TransferError::Exists(
                "A racing creator reserved this name.".into(),
            ));
        }
        Ok(())
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        let message = match self.fault {
            Fault::CopyInterrupted => "Device disconnected.",
            Fault::BackendCancelled => "Operation was cancelled by the backend.",
            _ => return node.local_copy_file(target, cancel, progress),
        };
        write(&local_path_of(target), "incomplete");
        Err(TransferError::failed(message))
    }

    fn replace_native(
        &self,
        _node: &LocalNode,
        _target: &dyn Node,
        _cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        Err(TransferError::ReplaceUnsupported(
            "Use a reversible replacement.".into(),
        ))
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if self.fault == Fault::MoveUnsupported {
            return Err(TransferError::NotSupported("Native move unsupported.".into()));
        }
        match MoveStep::of(node, target) {
            MoveStep::Aside => self.move_aside(node, target, cancel),
            MoveStep::Restore => self.restore(node, target, cancel),
            MoveStep::Install => self.install(node, target, cancel),
        }
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if matches!(self.fault, Fault::BackupCleanup | Fault::CancelAndCleanup) && is_backup(node) {
            return Err(TransferError::failed("Backup deletion refused."));
        }
        node.local_delete()
    }
}

/// A `document` source with new content and an existing `document` in the
/// destination.
fn replacement(fixture: &Fixture) -> PathBuf {
    let source = fixture.src.join("document");
    write(&source, "incoming");
    write(&fixture.dst.join("document"), "original");
    source
}

/// The replacement backup left in `fixture`'s destination.
fn backup_in(fixture: &Fixture) -> PathBuf {
    let names = list(&fixture.dst);
    let backup = names
        .iter()
        .find(|name| name.starts_with(".winspace-replaced-"))
        .expect("a backup is left in the destination");
    fixture.dst.join(backup)
}

/// A copy that fails midway, even one the backend itself reports as
/// cancelled, keeps the existing item and removes its private staging.
///
/// parity: XFER-001, XFER-002
#[test]
fn failed_or_backend_cancelled_copies_keep_the_original_and_remove_private_staging() {
    for fault in [Fault::CopyInterrupted, Fault::BackendCancelled] {
        let fixture = Fixture::new();
        let source = fixture.src.join("document");
        write(&source, "complete original");
        write(&fixture.dst.join("document"), "prior destination");
        let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Copy,
            ConflictPolicy::Replace,
            None,
        );

        assert!(
            !result.cancelled,
            "backend cancellation is an error, not a user request"
        );
        assert_eq!(result.errors.len(), 1);
        assert_eq!(read(&source), "complete original");
        assert_eq!(read(&fixture.dst.join("document")), "prior destination");
        fixture.no_stage();
    }
}

/// Port of `test_move_failure_does_not_copy_delete`.
///
/// parity: XFER-011
#[test]
fn native_move_failure_never_degrades_to_copy_then_delete() {
    let fixture = Fixture::new();
    let source = fixture.src.join("document");
    write(&source, "original");
    let mut engine = fixture.engine(Faults::new(Fault::MoveUnsupported, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.errors.len(), 1);
    assert!(result.done.is_empty());
    assert_eq!(read(&source), "original");
    assert!(list(&fixture.dst).is_empty());
}

/// Port of `test_preflight_race_never_overwrites`: a name another program
/// creates after the conflict check is not overwritten when the copy is
/// published.
///
/// parity: XFER-007
#[test]
fn a_name_taken_while_copying_is_not_overwritten_at_publication() {
    let fixture = Fixture::new();
    let source = fixture.src.join("document");
    write(&source, "original");
    let mut engine = fixture.engine(Faults::new(Fault::PublishRace, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.errors.len(), 1);
    assert!(result.done.is_empty());
    assert_eq!(read(&source), "original");
    assert_eq!(read(&fixture.dst.join("document")), "racing file");
    fixture.no_stage();
}

/// A local destination that stops answering once a copy fails, like
/// `Unreachable` in `test_local_stage_query_error_is_still_reported`.
#[derive(Default)]
struct UnreachableAfterFailure {
    unreachable: Mutex<bool>,
}

impl Provider for UnreachableAfterFailure {
    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        let unreachable = *self.unreachable.lock().expect("reachability");
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
        *self.unreachable.lock().expect("reachability") = true;
        Err(TransferError::failed("Input/output error"))
    }
}

/// Port of `test_local_stage_query_error_is_still_reported`: local staging
/// that cannot even be queried gets one cleanup attempt and is reported
/// with its exact location.
///
/// parity: XFER-003
#[test]
fn a_local_stage_that_cannot_be_queried_is_still_reported() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
    write(&source, "data");

    let result = fixture.copy(
        Arc::new(UnreachableAfterFailure::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    let names = list(&fixture.dst);
    assert_eq!(names.len(), 1, "{names:?}");
    let stage = fixture.dst.join(&names[0]);
    let report = format!("Incomplete staging folder left at {}", uri(&stage));
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
    let source = fixture.src.join("document");
    write(&source, "original");
    let mut engine = fixture.engine(Faults::new(Fault::UnownedStage, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.errors.len(), 1);
    let leftovers = list(&fixture.dst);
    assert_eq!(leftovers.len(), 1);
    assert_eq!(
        read(&fixture.dst.join(&leftovers[0]).join("not-ours")),
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
        for mode in [TransferMode::Copy, TransferMode::Move] {
            let fixture = Fixture::new();
            let source = replacement(&fixture);
            let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

            let result = fixture.run(&mut engine, &[&source], mode, ConflictPolicy::Replace, None);

            assert!(result.done.is_empty(), "{result:?}");
            assert_eq!(result.errors.len(), 1, "{result:?}");
            assert_eq!(read(&fixture.dst.join("document")), "original");
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
    let source = replacement(&fixture);
    let mut engine = fixture.engine(Faults::new(Fault::InstallAndRestore, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    let leftovers = list(&fixture.dst);
    assert_eq!(leftovers.len(), 1);
    let backup = backup_in(&fixture);
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&uri(&backup)));
    assert_eq!(read(&source), "incoming");
    fixture.no_stage();
}

/// Once the old file is aside, the small rest of the commit finishes even
/// though the user cancelled; later items are not started.
///
/// parity: OPS-022, XFER-010
#[test]
fn cancellation_after_move_aside_finishes_the_small_commit_without_losing_the_old_name() {
    let fixture = Fixture::new();
    let source = replacement(&fixture);
    let later = fixture.src.join("later");
    write(&later, "later");
    let mut engine = fixture.engine(Faults::new(Fault::CancelAfterAside, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source, &later],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    assert!(result.cancelled);
    assert_eq!(result.done, [uri(&source)]);
    assert_eq!(read(&fixture.dst.join("document")), "incoming");
    assert!(!fixture.dst.join("later").exists());
    assert!(fixture.leftovers().is_empty());
}

/// A backup that cannot be deleted after a successful replacement is
/// reported with its location, and the new file stays.
///
/// parity: XFER-003, XFER-010
#[test]
fn backup_cleanup_failure_reports_the_original_and_keeps_the_new_file() {
    let fixture = Fixture::new();
    let source = replacement(&fixture);
    let mut engine = fixture.engine(Faults::new(Fault::BackupCleanup, &fixture.cancel));

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    assert_eq!(read(&fixture.dst.join("document")), "incoming");
    let backup = backup_in(&fixture);
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&uri(&backup)));
    fixture.no_stage();
}

/// A cancelled run still reports a backup the user must recover by hand.
///
/// parity: OPS-022, XFER-010
#[test]
fn cancellation_never_hides_a_backup_that_requires_manual_recovery() {
    for fault in [Fault::CancelAndCleanup, Fault::CancelAndRollback] {
        let fixture = Fixture::new();
        let source = replacement(&fixture);
        let mut engine = fixture.engine(Faults::new(fault, &fixture.cancel));

        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Copy,
            ConflictPolicy::Replace,
            None,
        );

        assert!(result.cancelled);
        assert_eq!(result.errors.len(), 1, "{result:?}");
        let backup = backup_in(&fixture);
        assert_eq!(read(&backup), "original");
        assert!(result.errors[0].contains(&uri(&backup)), "{result:?}");
        assert_eq!(read(&source), "incoming");
        fixture.no_stage();
    }
}
