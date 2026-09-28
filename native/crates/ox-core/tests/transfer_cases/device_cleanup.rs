// SPDX-License-Identifier: AGPL-3.0-only
//! Cleanup and verification after failed uploads to simulated MTP devices.
//! Ports the cleanup cases of `DeviceStagingTests` in
//! `desktop/tests/test_device_staging.py`. No real devices.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeInfo, NodeKind, TransferError};

use crate::transfer_support::{
    device::Device,
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// How a [`BrokenPhone`] misbehaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhoneFault {
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
    fault: PhoneFault,
    /// Set once an upload failed; staged items misbehave from then on.
    upload_failed: AtomicBool,
    /// Queries of staged items after the upload failed.
    stage_lookups: AtomicUsize,
    /// Attempts to delete a staged item.
    stage_deletions: AtomicUsize,
}

impl BrokenPhone {
    /// A phone failing with `fault`.
    fn new(fault: PhoneFault) -> Arc<Self> {
        Arc::new(Self {
            device: Device::default(),
            fault,
            upload_failed: AtomicBool::default(),
            stage_lookups: AtomicUsize::default(),
            stage_deletions: AtomicUsize::default(),
        })
    }

    /// How often the engine tried to delete a staged item.
    fn stage_deletions(&self) -> usize {
        self.stage_deletions.load(Ordering::SeqCst)
    }

    /// Counts one query of a staged item and returns how many there were.
    fn count_stage_lookup(&self) -> usize {
        self.stage_lookups.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Counts one deletion of a staged item and returns how many there were.
    fn count_stage_deletion(&self) -> usize {
        self.stage_deletions.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Makes staged items misbehave from now on.
    fn mark_upload_failed(&self) {
        self.upload_failed.store(true, Ordering::SeqCst);
    }

    /// True once an upload failed.
    fn has_upload_failed(&self) -> bool {
        self.upload_failed.load(Ordering::SeqCst)
    }
}

/// True for `node` or any of its ancestors named like engine staging.
fn is_inside_staging(node: &LocalNode) -> bool {
    node.local_path().ancestors().any(is_staging_path)
}

impl Provider for BrokenPhone {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn create_directory(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_create_directory(cancel)?;
        if self.fault == PhoneFault::StageRace {
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
            PhoneFault::FalsePublication | PhoneFault::UnverifiablePublication => {
                return node.local_copy_file(target, cancel, progress);
            }
            PhoneFault::StageRace => {
                write(&local_path_of(target), "belongs to another creator");
                return Err(TransferError::Exists("The staging name was taken.".into()));
            }
            PhoneFault::DiscardedUpload => {}
            _ => write(&local_path_of(target), "partial upload"),
        }
        self.mark_upload_failed();
        Err(TransferError::failed("The connection was interrupted."))
    }

    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        if !self.has_upload_failed() || !is_inside_staging(node) {
            return node.local_info(cancel);
        }
        let lookup = self.count_stage_lookup();
        match self.fault {
            PhoneFault::FalseNotFound => Err(TransferError::NotFound("Uncached device path.".into())),
            PhoneFault::TransientNotFound if lookup == 1 => {
                Err(TransferError::NotFound("Uncached device path.".into()))
            }
            PhoneFault::Disconnected | PhoneFault::UnverifiablePublication => {
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
        if self.fault == PhoneFault::FalsePublication {
            return Ok(());
        }
        self.device.move_native(node, target, cancel)?;
        if self.fault == PhoneFault::UnverifiablePublication {
            self.mark_upload_failed();
        }
        Ok(())
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if is_staging(node) {
            let deletions = self.count_stage_deletion();
            let busy = match self.fault {
                PhoneFault::TransientDelete => deletions < 3,
                PhoneFault::StuckDelete => true,
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
fn photo(fixture: &Fixture) -> PathBuf {
    let source = fixture.source_folder.join("photo");
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
    let phone = BrokenPhone::new(PhoneFault::TransientDelete);

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(phone.stage_deletions(), 3);
    assert_eq!(fixture.sleeps(), [0.5, 1.5]);
    assert_eq!(read(&source), "complete");
    fixture.assert_no_staging();
}

/// Port of `test_persistent_cleanup_failure_reports_exact_location`: after
/// every retry fails, the leftover is reported with its exact location.
///
/// parity: XFER-003, XFER-022
#[test]
fn a_device_stage_that_cannot_be_deleted_is_reported_with_its_location() {
    let fixture = Fixture::new();
    let source = photo(&fixture);
    let phone = BrokenPhone::new(PhoneFault::StuckDelete);

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

    let names = list(&fixture.destination_folder);
    assert_eq!(names.len(), 1, "{names:?}");
    let stage = fixture.destination_folder.join(&names[0]);
    assert!(is_staging_path(&stage), "{}", stage.display());
    let report = format!("Incomplete staging item left at {}", file_uri(&stage));
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
        BrokenPhone::new(PhoneFault::DiscardedUpload),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(fixture.sleeps().is_empty());
    assert_eq!(read(&source), "complete");
    assert!(!fixture.destination_folder.join("photo").exists());
    fixture.assert_no_staging();
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
        BrokenPhone::new(PhoneFault::TransientNotFound),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(!result.errors[0].contains("Incomplete staging"), "{result:?}");
    assert_eq!(fixture.sleeps(), [0.5]);
    assert!(list(&fixture.destination_folder).is_empty());
}

/// Ports `test_device_stage_query_error_is_retried_and_reported`: "not found"
/// counts only when listing the folder confirms it, and any other query
/// error is retried and reported with the stage's exact location.
///
/// parity: XFER-003, XFER-022
#[test]
fn device_not_found_requires_a_successful_parent_listing_without_the_stage() {
    for fault in [PhoneFault::FalseNotFound, PhoneFault::Disconnected] {
        let fixture = Fixture::new();
        let source = photo(&fixture);
        let phone = BrokenPhone::new(fault);

        let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

        assert_eq!(read(&source), "complete");
        assert!(!fixture.destination_folder.join("photo").exists());
        assert_eq!(result.errors.len(), 2, "{fault:?}: {result:?}");
        let stage = fixture
            .destination_folder
            .join(&list(&fixture.destination_folder)[0]);
        assert_eq!(read(&stage), "partial upload");
        let report = format!("Incomplete staging item left at {}", file_uri(&stage));
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
        BrokenPhone::new(PhoneFault::FalsePublication),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("reported success"), "{result:?}");
    assert_eq!(read(&source), "complete");
    assert!(list(&fixture.destination_folder).is_empty());
}

/// An upload whose staging name another program takes first, and where
/// that program's content is.
struct StageRaceCase {
    kind: NodeKind,
    /// The other program's file inside the taken staging name; `None` when
    /// the taken name is that file itself.
    foreign_file: Option<&'static str>,
}

/// A staging name another program created first is never used or removed:
/// neither the folder a folder upload reserves with `create_directory`, nor
/// the file a file upload creates without overwriting.
///
/// parity: XFER-002
#[test]
fn a_device_staging_name_created_by_someone_else_is_never_cleaned_up() {
    let cases = [
        StageRaceCase {
            kind: NodeKind::File,
            foreign_file: None,
        },
        StageRaceCase {
            kind: NodeKind::Directory,
            foreign_file: Some("foreign"),
        },
    ];
    for case in cases {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("photo");
        create_source(&source, case.kind, "complete");
        let phone = BrokenPhone::new(PhoneFault::StageRace);

        let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

        assert_eq!(result.errors.len(), 1, "{:?}: {result:?}", case.kind);
        assert!(result.done.is_empty());
        let names = list(&fixture.destination_folder);
        assert_eq!(names.len(), 1);
        let taken_name = fixture.destination_folder.join(&names[0]);
        let foreign_file = match case.foreign_file {
            Some(name) => taken_name.join(name),
            None => taken_name,
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
    let phone = BrokenPhone::new(PhoneFault::UnverifiablePublication);

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);

    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("could not be verified"), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("photo")), "complete");
    assert_eq!(read(&source), "complete");
    assert_eq!(phone.stage_deletions(), 0);
}
