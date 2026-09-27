// SPDX-License-Identifier: AGPL-3.0-only
//! Cleanup and verification after failed uploads to simulated MTP devices.
//! Ports the cleanup cases of `DeviceStagingTests` in
//! `desktop/tests/test_device_staging.py`. No real devices.

use std::fs;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeInfo, NodeKind, TransferError};

use crate::transfer_support::{
    device::Device,
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// How a [`BrokenPhone`] misbehaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Deleting the stage fails twice, then works.
    TransientDelete,
    /// Deleting the stage always fails.
    StuckDelete,
    /// The stage answers "not found" although it exists.
    FalseNotFound,
    /// The stage answers "not found" once, then is found again.
    TransientNotFound,
    /// The device dropped the aborted upload entirely.
    DiscardedUpload,
    /// The stage cannot be queried at all after the failure.
    Disconnected,
    /// Moves report success without doing anything.
    FalsePublication,
    /// Another program creates the staging name first.
    StageRace,
    /// After publishing, the staged name cannot be queried.
    UnverifiablePublication,
}

/// A device whose uploads fail in the way `fault` describes.
struct BrokenPhone {
    device: Device,
    fault: Fault,
    upload_failed: Mutex<bool>,
    /// Queries of staged items after the upload failed.
    stage_lookups: Mutex<usize>,
    stage_deletions: Mutex<usize>,
}

impl BrokenPhone {
    fn new(fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            device: Device::default(),
            fault,
            upload_failed: Mutex::default(),
            stage_lookups: Mutex::default(),
            stage_deletions: Mutex::default(),
        })
    }

    fn stage_deletions(&self) -> usize {
        *self.stage_deletions.lock().expect("deletion count")
    }

    /// Counts one query of a staged item and returns how many there were.
    fn count_stage_lookup(&self) -> usize {
        let mut lookups = self.stage_lookups.lock().expect("lookup count");
        *lookups += 1;
        *lookups
    }

    fn mark_upload_failed(&self) {
        *self.upload_failed.lock().expect("failure flag") = true;
    }

    fn has_upload_failed(&self) -> bool {
        *self.upload_failed.lock().expect("failure flag")
    }
}

/// True for `path` or any of its ancestors named like engine staging.
fn inside_staging(node: &LocalNode) -> bool {
    node.local_path().ancestors().any(is_staging_path)
}

impl Provider for BrokenPhone {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn mkdir(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_mkdir(cancel)?;
        if self.fault == Fault::StageRace {
            write(&node.local_path().join("foreign"), "belongs to another creator");
            return Err(TransferError::Exists("The staging name was taken.".into()));
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
        match self.fault {
            Fault::FalsePublication | Fault::UnverifiablePublication => {
                return node.local_copy_file(target, cancel, progress);
            }
            Fault::StageRace => {
                write(&local_path_of(target), "belongs to another creator");
                return Err(TransferError::Exists("The staging name was taken.".into()));
            }
            Fault::DiscardedUpload => {}
            _ => write(&local_path_of(target), "partial upload"),
        }
        self.mark_upload_failed();
        Err(TransferError::failed("The connection was interrupted."))
    }

    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        if !self.has_upload_failed() || !inside_staging(node) {
            return node.local_info(cancel);
        }
        let lookup = self.count_stage_lookup();
        match self.fault {
            Fault::FalseNotFound => Err(TransferError::NotFound("Uncached device path.".into())),
            Fault::TransientNotFound if lookup == 1 => {
                Err(TransferError::NotFound("Uncached device path.".into()))
            }
            Fault::Disconnected | Fault::UnverifiablePublication => {
                Err(TransferError::failed("Device is disconnected."))
            }
            _ => node.local_info(cancel),
        }
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if self.fault == Fault::FalsePublication {
            return Ok(());
        }
        self.device.move_native(node, target, cancel)?;
        if self.fault == Fault::UnverifiablePublication {
            self.mark_upload_failed();
        }
        Ok(())
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if is_staging(node) {
            let mut deletions = self.stage_deletions.lock().expect("deletion count");
            *deletions += 1;
            let busy = match self.fault {
                Fault::TransientDelete => *deletions < 3,
                Fault::StuckDelete => true,
                _ => false,
            };
            if busy {
                return Err(TransferError::failed("The phone is still busy."));
            }
        }
        node.local_delete()
    }
}

/// A `photo` source file with complete content.
fn photo(fixture: &Fixture) -> std::path::PathBuf {
    let source = fixture.src.join("photo");
    write(&source, "complete");
    source
}

/// Port of `test_cleanup_retries_a_transient_device_error`, with a device
/// that stays busy for two attempts.
///
/// parity: XFER-022
#[test]
fn aborted_device_upload_cleanup_retries_transient_errors() {
    let fixture = Fixture::new();
    let source = photo(&fixture);
    let phone = BrokenPhone::new(Fault::TransientDelete);
    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(phone.stage_deletions(), 3);
    assert_eq!(fixture.sleeps(), [0.5, 1.5]);
    assert_eq!(read(&source), "complete");
    fixture.no_stage();
}

/// Port of `test_persistent_cleanup_failure_reports_exact_location`: after
/// every retry fails, the leftover is reported with its exact location.
///
/// parity: XFER-003, XFER-022
#[test]
fn a_device_stage_that_cannot_be_deleted_is_reported_with_its_location() {
    let fixture = Fixture::new();
    let source = photo(&fixture);
    let phone = BrokenPhone::new(Fault::StuckDelete);

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

    let names = list(&fixture.dst);
    assert_eq!(names.len(), 1, "{names:?}");
    let stage = fixture.dst.join(&names[0]);
    assert!(is_staging_path(&stage), "{}", stage.display());
    let report = format!("Incomplete staging item left at {}", uri(&stage));
    assert!(
        result.errors.iter().any(|error| error.contains(&report)),
        "{result:?}"
    );
    assert_eq!(fixture.sleeps(), [0.5, 1.5]);
    assert_eq!(phone.stage_deletions(), 3);
}

/// Port of `test_missing_stage_after_aborted_upload_is_not_reported_as_leftover`:
/// an upload the device discarded needs no cleanup and no retry.
///
/// parity: XFER-022
#[test]
fn a_discarded_device_upload_is_not_reported_as_a_leftover() {
    let fixture = Fixture::new();
    let source = photo(&fixture);

    let result = fixture.copy(
        BrokenPhone::new(Fault::DiscardedUpload),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(fixture.sleeps().is_empty());
    assert_eq!(read(&source), "complete");
    assert!(!fixture.dst.join("photo").exists());
    fixture.no_stage();
}

/// Port of `test_not_found_for_an_existing_stage_is_confirmed_by_listing`: a
/// "not found" the folder listing contradicts is retried, and the stage is
/// then removed without being reported.
///
/// parity: XFER-022
#[test]
fn a_false_not_found_for_a_device_stage_is_retried_until_it_is_removed() {
    let fixture = Fixture::new();
    let source = photo(&fixture);

    let result = fixture.copy(
        BrokenPhone::new(Fault::TransientNotFound),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(!result.errors[0].contains("Incomplete staging"), "{result:?}");
    assert_eq!(fixture.sleeps(), [0.5]);
    assert!(list(&fixture.dst).is_empty());
}

/// Ports `test_device_stage_query_error_is_retried_and_reported`: "not found"
/// counts only when listing the folder confirms it, and any other query
/// error is retried and reported with the stage's exact location.
///
/// parity: XFER-003, XFER-022
#[test]
fn device_not_found_requires_a_successful_parent_listing_without_the_stage() {
    for fault in [Fault::FalseNotFound, Fault::Disconnected] {
        let fixture = Fixture::new();
        let source = photo(&fixture);
        let phone = BrokenPhone::new(fault);

        let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

        assert_eq!(read(&source), "complete");
        assert!(!fixture.dst.join("photo").exists());
        assert_eq!(result.errors.len(), 2, "{fault:?}: {result:?}");
        let stage = fixture.dst.join(&list(&fixture.dst)[0]);
        assert_eq!(read(&stage), "partial upload");
        let report = format!("Incomplete staging item left at {}", uri(&stage));
        assert!(result.errors[1].contains(&report), "{result:?}");
        assert_eq!(fixture.sleeps(), [0.5, 1.5]);
        assert_eq!(phone.stage_deletions(), 0);
    }
}

/// Port of `test_success_report_without_rename_is_an_error`.
///
/// parity: XFER-021
#[test]
fn a_devices_false_success_is_not_counted_as_a_published_copy() {
    let fixture = Fixture::new();
    let source = photo(&fixture);
    let result = fixture.copy(
        BrokenPhone::new(Fault::FalsePublication),
        &[&source],
        ConflictPolicy::Skip,
    );
    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("reported success"), "{result:?}");
    assert_eq!(read(&source), "complete");
    assert!(list(&fixture.dst).is_empty());
}

/// A staging name another program created first is never used or removed:
/// neither the folder a folder upload reserves with `mkdir`, nor the file a
/// file upload creates without overwriting.
///
/// parity: XFER-002
#[test]
fn a_device_staging_name_created_by_someone_else_is_never_cleaned_up() {
    for kind in [NodeKind::File, NodeKind::Directory] {
        let fixture = Fixture::new();
        let source = fixture.src.join("photo");
        if kind == NodeKind::Directory {
            fs::create_dir(&source).expect("create the source folder");
            write(&source.join("inner"), "complete");
        } else {
            write(&source, "complete");
        }
        let phone = BrokenPhone::new(Fault::StageRace);
        let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
        assert_eq!(result.errors.len(), 1, "{result:?}");
        assert!(result.done.is_empty());
        let names = list(&fixture.dst);
        assert_eq!(names.len(), 1);
        let foreign = fixture.dst.join(&names[0]);
        let foreign_file = if kind == NodeKind::Directory {
            foreign.join("foreign")
        } else {
            foreign
        };
        assert_eq!(read(&foreign_file), "belongs to another creator");
        assert_eq!(phone.stage_deletions(), 0);
    }
}

/// A staged name that cannot be queried after publishing is not proof that
/// it is gone, so the copy is not counted as verified.
///
/// parity: XFER-021
#[test]
fn an_unreachable_staged_name_is_not_mistaken_for_definite_absence() {
    let fixture = Fixture::new();
    let source = photo(&fixture);
    let phone = BrokenPhone::new(Fault::UnverifiablePublication);
    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("could not be verified"), "{result:?}");
    assert_eq!(read(&fixture.dst.join("photo")), "complete");
    assert_eq!(read(&source), "complete");
    assert_eq!(phone.stage_deletions(), 0);
}
