// SPDX-License-Identifier: AGPL-3.0-only
//! Failure injection at the copy, publication and reversible replacement boundaries.

use std::fs;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, TransferError, TransferMode};

use crate::transfer_support::{
    local::{local_path_of, LocalNode, Provider},
    *,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    CopyInterrupted,
    BackendCancelled,
    MoveUnsupported,
    PublishRace,
    Install,
    InstallAndRestore,
    AsideAfterSuccess,
    CancelAfterAside,
    CancelAndCleanup,
    CancelAndRollback,
    BackupCleanup,
    UnownedStage,
    NoOpInstall,
}

struct Faults {
    fault: Fault,
    cancel: Cancellation,
    writes: Mutex<Vec<String>>,
}

impl Faults {
    fn new(fault: Fault, cancel: &Cancellation) -> Arc<Self> {
        Arc::new(Self {
            fault,
            cancel: cancel.clone(),
            writes: Mutex::default(),
        })
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
        if matches!(self.fault, Fault::CopyInterrupted | Fault::BackendCancelled) {
            write(&local_path_of(target), "incomplete");
            return Err(TransferError::failed(if self.fault == Fault::BackendCancelled {
                "Operation was cancelled by the backend."
            } else {
                "Device disconnected."
            }));
        }
        node.local_copy_file(target, cancel, progress)
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
        self.writes
            .lock()
            .unwrap()
            .push(format!("{} -> {}", node.display_name(), target.display_name()));
        if self.fault == Fault::MoveUnsupported {
            return Err(TransferError::NotSupported("Native move unsupported.".into()));
        }
        let aside = is_backup(target);
        let restore = is_backup(node);
        let install = !aside && !restore;
        if install && self.fault == Fault::PublishRace {
            write(&local_path_of(target), "racing file");
        }
        if install
            && matches!(
                self.fault,
                Fault::Install | Fault::InstallAndRestore | Fault::CancelAndRollback
            )
        {
            return Err(TransferError::failed("Install failed."));
        }
        if restore && matches!(self.fault, Fault::InstallAndRestore | Fault::CancelAndRollback) {
            return Err(TransferError::failed("Restore failed."));
        }
        if install && self.fault == Fault::NoOpInstall {
            return Ok(());
        }
        if aside
            && matches!(
                self.fault,
                Fault::CancelAfterAside
                    | Fault::AsideAfterSuccess
                    | Fault::CancelAndCleanup
                    | Fault::CancelAndRollback
            )
        {
            assert!(cancel.is_none(), "the commit must not be interruptible");
            node.local_move_native(target, cancel)?;
            if self.fault != Fault::AsideAfterSuccess {
                self.cancel.cancel();
                return Ok(());
            }
            return Err(TransferError::failed("Rename finished before timeout."));
        }
        if install
            && matches!(
                self.fault,
                Fault::CancelAfterAside | Fault::CancelAndCleanup | Fault::CancelAndRollback
            )
        {
            assert!(
                cancel.is_none(),
                "installation must finish after moving the old file aside"
            );
        }
        node.local_move_native(target, cancel)
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if matches!(self.fault, Fault::BackupCleanup | Fault::CancelAndCleanup) && is_backup(node) {
            return Err(TransferError::failed("Backup deletion refused."));
        }
        node.local_delete()
    }
}

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

#[test]
fn replacement_install_failure_or_false_success_restores_the_old_name() {
    for fault in [Fault::Install, Fault::NoOpInstall, Fault::AsideAfterSuccess] {
        for mode in [TransferMode::Copy, TransferMode::Move] {
            let fixture = Fixture::new();
            let source = fixture.src.join("document");
            write(&source, "incoming");
            write(&fixture.dst.join("document"), "original");
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

#[test]
fn rollback_failure_reports_the_exact_backup_and_preserves_its_contents() {
    let fixture = Fixture::new();
    let source = fixture.src.join("document");
    write(&source, "incoming");
    write(&fixture.dst.join("document"), "original");
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
    let backup = fixture.dst.join(&leftovers[0]);
    assert!(leftovers[0].starts_with(".winspace-replaced-"));
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&uri(&backup)));
    assert_eq!(read(&source), "incoming");
    fixture.no_stage();
}

#[test]
fn cancellation_after_move_aside_finishes_the_small_commit_without_losing_the_old_name() {
    let fixture = Fixture::new();
    let source = fixture.src.join("document");
    let later = fixture.src.join("later");
    write(&source, "incoming");
    write(&later, "later");
    write(&fixture.dst.join("document"), "original");
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

#[test]
fn backup_cleanup_failure_reports_the_original_and_keeps_the_new_file() {
    let fixture = Fixture::new();
    let source = fixture.src.join("document");
    write(&source, "incoming");
    write(&fixture.dst.join("document"), "original");
    let mut engine = fixture.engine(Faults::new(Fault::BackupCleanup, &fixture.cancel));
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );
    assert_eq!(read(&fixture.dst.join("document")), "incoming");
    let backup = fs::read_dir(&fixture.dst)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".winspace-replaced-")
        })
        .unwrap();
    assert_eq!(read(&backup), "original");
    assert!(result.errors[0].contains(&uri(&backup)));
    fixture.no_stage();
}

#[test]
fn cancellation_never_hides_a_backup_that_requires_manual_recovery() {
    for fault in [Fault::CancelAndCleanup, Fault::CancelAndRollback] {
        let fixture = Fixture::new();
        let source = fixture.src.join("document");
        write(&source, "incoming");
        write(&fixture.dst.join("document"), "original");
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
        let names = list(&fixture.dst);
        let backup_name = names
            .iter()
            .find(|name| name.starts_with(".winspace-replaced-"))
            .unwrap();
        let backup = fixture.dst.join(backup_name);
        assert_eq!(read(&backup), "original");
        assert!(result.errors[0].contains(&uri(&backup)), "{result:?}");
        assert_eq!(read(&source), "incoming");
        fixture.no_stage();
    }
}
