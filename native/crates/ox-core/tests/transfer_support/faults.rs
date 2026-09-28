// SPDX-License-Identifier: AGPL-3.0-only
//! A local provider that fails at a chosen step of a copy, a publication or
//! a reversible replacement, like the failing `LocalNode` subclasses in
//! `desktop/tests/test_operations.py`.

use std::sync::Arc;

use ox_core::transfer::{Cancellation, Node, TransferError};

use super::local::{local_path_of, LocalNode, Provider};
use super::{is_backup, is_staging, write};

/// Where a [`Faults`] provider breaks the copy, publication or reversible
/// replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
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

    /// Faults where putting the old file back fails.
    fn fails_restore(self) -> bool {
        matches!(self, Fault::InstallAndRestore | Fault::CancelAndRollback)
    }

    /// Faults where the replaced file's backup cannot be deleted.
    fn keeps_backup(self) -> bool {
        matches!(self, Fault::BackupCleanup | Fault::CancelAndCleanup)
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
    /// The step a move of `node` to `target` performs.
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

/// A local provider that fails the way `fault` describes. Direct overwrite
/// is never supported, so Replace always renames reversibly.
pub struct Faults {
    fault: Fault,
    /// The run's cancellation, which the `Cancel*` faults cancel.
    cancel: Cancellation,
}

impl Faults {
    /// A provider failing with `fault` that cancels `cancel` where the fault
    /// says so.
    pub fn new(fault: Fault, cancel: &Cancellation) -> Arc<Self> {
        Arc::new(Self {
            fault,
            cancel: cancel.clone(),
        })
    }

    /// Renames the old file to its backup name, then interrupts the commit
    /// if the fault says so.
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

    /// Puts the backup back under the old name, unless the fault fails it.
    fn restore(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if self.fault.fails_restore() {
            return Err(TransferError::failed("Restore failed."));
        }
        node.local_move_native(target, cancel)
    }

    /// Installs the new item under its final name, failing or racing as the
    /// fault says.
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
    fn create_directory(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_create_directory(cancel)?;
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
        if self.fault.keeps_backup() && is_backup(node) {
            return Err(TransferError::failed("Backup deletion refused."));
        }
        node.local_delete()
    }
}
