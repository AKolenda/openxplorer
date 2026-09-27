// SPDX-License-Identifier: AGPL-3.0-only
//! Simulated MTP restrictions and post-disconnect cleanup. No real devices.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeInfo, TransferError, TransferMode};

use crate::transfer_support::{
    device::Device,
    local::{local_path_of, LocalNode, Provider},
    *,
};

#[derive(Default)]
struct Phone {
    device: Device,
    same_device_copy: bool,
    stale_move: Mutex<Option<(PathBuf, PathBuf)>>,
}

impl Provider for Phone {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn uri(&self, node: &LocalNode) -> String {
        uri(node.local_path()).replacen("file://", "mtp://test-device", 1)
    }

    fn path(&self, _node: &LocalNode) -> Option<PathBuf> {
        None
    }

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_dir: &dyn Node) -> bool {
        self.same_device_copy
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        if self.same_device_copy {
            assert_eq!(node.name(), target.name(), "CopyObject retains its source name");
        }
        node.local_copy_file(target, cancel, progress)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        self.device.move_native(node, target, cancel)?;
        let destination = local_path_of(target);
        if node.local_path().parent() != destination.parent() {
            *self.stale_move.lock().unwrap() = Some((node.local_path().to_path_buf(), destination));
        }
        Ok(())
    }

    fn exists(&self, node: &LocalNode, _cancel: Option<&Cancellation>) -> bool {
        if let Some((old, new)) = &*self.stale_move.lock().unwrap() {
            if node.local_path() == old {
                return new.exists();
            }
        }
        node.local_exists()
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if let Some((old, new)) = &*self.stale_move.lock().unwrap() {
            if node.local_path() == old {
                // GVfs can still resolve the old path to the moved object.
                // Correct publication must relist before cleanup gets here.
                fs::remove_file(new)?;
                return Ok(());
            }
        }
        node.local_delete()
    }

    fn refresh_listing(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.device.refresh_listing(node, cancel)?;
        let mut stale = self.stale_move.lock().unwrap();
        if stale
            .as_ref()
            .is_some_and(|(old, _)| old.parent() == Some(node.local_path()))
        {
            *stale = None;
        }
        Ok(())
    }
}

#[test]
fn uploads_publish_only_through_moves_the_device_can_perform() {
    for directory in [false, true] {
        let fixture = Fixture::with_destination("phone");
        let source = fixture.src.join("incoming");
        if directory {
            fs::create_dir(&source).unwrap();
            write(&source.join("photo.jpg"), "photo");
        } else {
            write(&source, "photo");
        }
        let phone = Arc::new(Phone::default());
        let mut engine = fixture.engine(phone.clone());
        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Copy,
            ConflictPolicy::Skip,
            None,
        );
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [uri(&source)]);
        let target = if directory {
            fixture.dst.join("incoming/photo.jpg")
        } else {
            fixture.dst.join("incoming")
        };
        assert_eq!(read(&target), "photo");
        phone.device.assert_only_same_folder_renames_across_names();
        let moves = phone.device.moves();
        assert_eq!(moves.len(), 1);
        if directory {
            assert_eq!(moves[0].0.parent(), Some(fixture.dst.as_path()));
            assert!(moves[0]
                .0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".winspace-transfer-"));
        } else {
            assert_eq!(moves[0].0.file_name(), moves[0].1.file_name());
            assert_ne!(moves[0].0.parent(), moves[0].1.parent());
        }
        fixture.no_stage();
    }
}

#[test]
fn same_device_keep_both_renames_inside_staging_and_refreshes_before_cleanup() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.src.join("photo.jpg");
    write(&source, "incoming");
    write(&fixture.dst.join("photo.jpg"), "original");
    let phone = Arc::new(Phone {
        same_device_copy: true,
        ..Phone::default()
    });
    let mut engine = fixture.engine(phone.clone());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::KeepBoth,
        None,
    );
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.dst.join("photo.jpg")), "original");
    assert_eq!(read(&fixture.dst.join("photo (copy 2).jpg")), "incoming");
    assert_eq!(read(&source), "incoming");
    phone.device.assert_only_same_folder_renames_across_names();
    assert_eq!(phone.device.moves().len(), 2);
    assert_eq!(phone.device.refreshes().len(), 2);
    assert!(phone.stale_move.lock().unwrap().is_none());
    fixture.no_stage();
}

#[test]
fn device_moves_relist_the_old_folder_once_per_batch() {
    let fixture = Fixture::new();
    let first = fixture.src.join("first");
    let second = fixture.src.join("second");
    write(&first, "first");
    write(&second, "second");
    let device = Arc::new(Device::default());
    let mut engine = fixture.engine(device.clone());
    let result = fixture.run(
        &mut engine,
        &[&first, &second],
        TransferMode::Move,
        ConflictPolicy::Skip,
        None,
    );
    assert_eq!(result.done.len(), 2);
    assert_eq!(device.refreshes().as_slice(), std::slice::from_ref(&fixture.src));
    assert_eq!(read(&fixture.dst.join("first")), "first");
    assert_eq!(read(&fixture.dst.join("second")), "second");
    device.assert_only_same_folder_renames_across_names();
}

#[derive(Clone, Copy)]
enum CleanupFault {
    TransientDelete,
    FalseNotFound,
    DiscardedUpload,
    Disconnected,
    FalsePublication,
    StageRace,
    UnverifiablePublication,
}

struct BrokenPhone {
    device: Device,
    fault: CleanupFault,
    failed: Mutex<bool>,
    attempts: Mutex<usize>,
}

impl BrokenPhone {
    fn new(fault: CleanupFault) -> Arc<Self> {
        Arc::new(Self {
            device: Device::default(),
            fault,
            failed: Mutex::default(),
            attempts: Mutex::default(),
        })
    }
}

impl Provider for BrokenPhone {
    fn mkdir(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_mkdir(cancel)?;
        if matches!(self.fault, CleanupFault::StageRace) {
            write(&node.local_path().join("foreign"), "belongs to another creator");
            return Err(TransferError::Exists("The staging name was taken.".into()));
        }
        Ok(())
    }

    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        if matches!(
            self.fault,
            CleanupFault::FalsePublication | CleanupFault::UnverifiablePublication
        ) {
            return node.local_copy_file(target, cancel, progress);
        }
        if !matches!(self.fault, CleanupFault::DiscardedUpload) {
            write(&local_path_of(target), "partial upload");
        }
        *self.failed.lock().unwrap() = true;
        Err(TransferError::failed("The connection was interrupted."))
    }

    fn info(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        if *self.failed.lock().unwrap()
            && node.local_path().ancestors().any(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".winspace-transfer-"))
            })
        {
            if matches!(self.fault, CleanupFault::FalseNotFound) {
                return Err(TransferError::NotFound("Uncached device path.".into()));
            }
            if matches!(
                self.fault,
                CleanupFault::Disconnected | CleanupFault::UnverifiablePublication
            ) {
                return Err(TransferError::failed("Device is disconnected."));
            }
        }
        node.local_info(cancel)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if matches!(self.fault, CleanupFault::FalsePublication) {
            return Ok(());
        }
        self.device.move_native(node, target, cancel)?;
        if matches!(self.fault, CleanupFault::UnverifiablePublication) {
            *self.failed.lock().unwrap() = true;
        }
        Ok(())
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if node.name().starts_with(".winspace-transfer-") {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            if matches!(self.fault, CleanupFault::TransientDelete) && *attempts < 3 {
                return Err(TransferError::failed("The phone is still busy."));
            }
        }
        node.local_delete()
    }
}

#[test]
fn aborted_device_upload_cleanup_retries_transient_errors() {
    let fixture = Fixture::new();
    let source = fixture.src.join("photo");
    write(&source, "complete");
    let phone = BrokenPhone::new(CleanupFault::TransientDelete);
    let mut engine = fixture.engine(phone.clone());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(*phone.attempts.lock().unwrap(), 3);
    assert_eq!(fixture.sleeps(), [0.5, 1.5]);
    assert_eq!(read(&source), "complete");
    fixture.no_stage();
}

#[test]
fn device_not_found_requires_a_successful_parent_listing_without_the_stage() {
    for fault in [
        CleanupFault::FalseNotFound,
        CleanupFault::DiscardedUpload,
        CleanupFault::Disconnected,
    ] {
        let fixture = Fixture::new();
        let source = fixture.src.join("photo");
        write(&source, "complete");
        let phone = BrokenPhone::new(fault);
        let mut engine = fixture.engine(phone.clone());
        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Copy,
            ConflictPolicy::Skip,
            None,
        );
        assert_eq!(read(&source), "complete");
        assert!(!fixture.dst.join("photo").exists());
        if matches!(fault, CleanupFault::DiscardedUpload) {
            assert_eq!(result.errors.len(), 1, "{result:?}");
            assert!(fixture.sleeps().is_empty());
            fixture.no_stage();
        } else {
            assert_eq!(result.errors.len(), 2, "{result:?}");
            let stage = fixture.dst.join(&list(&fixture.dst)[0]);
            assert_eq!(read(&stage.join("photo")), "partial upload");
            assert!(result.errors[1].contains(&uri(&stage)));
            assert_eq!(fixture.sleeps(), [0.5, 1.5]);
            assert_eq!(*phone.attempts.lock().unwrap(), 0);
        }
    }
}

#[test]
fn a_devices_false_success_is_not_counted_as_a_published_copy() {
    let fixture = Fixture::new();
    let source = fixture.src.join("photo");
    write(&source, "complete");
    let mut engine = fixture.engine(BrokenPhone::new(CleanupFault::FalsePublication));
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("reported success"));
    assert_eq!(read(&source), "complete");
    assert!(list(&fixture.dst).is_empty());
}

#[test]
fn a_device_staging_folder_created_by_someone_else_is_never_cleaned_up() {
    let fixture = Fixture::new();
    let source = fixture.src.join("photo");
    write(&source, "complete");
    let phone = BrokenPhone::new(CleanupFault::StageRace);
    let mut engine = fixture.engine(phone.clone());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert_eq!(result.errors.len(), 1);
    assert!(result.done.is_empty());
    let names = list(&fixture.dst);
    assert_eq!(names.len(), 1);
    assert_eq!(
        read(&fixture.dst.join(&names[0]).join("foreign")),
        "belongs to another creator"
    );
    assert_eq!(*phone.attempts.lock().unwrap(), 0);
    assert_eq!(read(&source), "complete");
}

#[test]
fn an_unreachable_staged_name_is_not_mistaken_for_definite_absence() {
    let fixture = Fixture::new();
    let source = fixture.src.join("photo");
    write(&source, "complete");
    let phone = BrokenPhone::new(CleanupFault::UnverifiablePublication);
    let mut engine = fixture.engine(phone.clone());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("could not be verified"));
    assert_eq!(read(&fixture.dst.join("photo")), "complete");
    assert_eq!(read(&source), "complete");
    assert_eq!(*phone.attempts.lock().unwrap(), 0);
}
