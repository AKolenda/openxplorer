// SPDX-License-Identifier: AGPL-3.0-only
//! Replace and publication races on simulated MTP devices. Ports the
//! Replace cases of `DeviceStagingTests` in
//! `desktop/tests/test_device_staging.py`. No real devices.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, TransferError};

use crate::transfer_support::{
    device::Device,
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_replace_file_uses_reversible_renames`: the device cannot
/// overwrite safely (XFER-026), so the old file is renamed aside, the new
/// one renamed in and the backup removed, all within the destination
/// folder.
///
/// parity: XFER-010, XFER-026
#[test]
fn device_replace_uses_reversible_same_folder_renames() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.replacement_source("photo", "new", "old");
    let phone = Arc::new(Device::default());

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Replace);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [file_uri(&source)]);
    assert_eq!(read(&fixture.destination_folder.join("photo")), "new");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    for recorded in phone.moves() {
        assert_eq!(
            recorded.from.parent(),
            recorded.to.parent(),
            "only renames in one folder"
        );
    }
}

/// Refuses to install a staged item under the name `photo`, like
/// `FailInstall` in `test_device_staging.py`.
#[derive(Default)]
struct FailingInstall {
    device: Device,
}

impl Provider for FailingInstall {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if is_staging(node) && target.display_name() == "photo" {
            return Err(TransferError::failed("simulated device refusal"));
        }
        self.device.move_native(node, target, cancel)
    }
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_replace_install_failure_restores_original_and_cleans_stage`.
///
/// parity: XFER-010
#[test]
fn a_failed_device_install_restores_the_original_and_removes_the_stage() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.replacement_source("photo", "new", "old");

    let result = fixture.copy(
        Arc::new(FailingInstall::default()),
        &[&source],
        ConflictPolicy::Replace,
    );

    assert!(result.done.is_empty());
    assert!(
        result.errors[0].contains("simulated device refusal"),
        "{result:?}"
    );
    assert_eq!(read(&fixture.destination_folder.join("photo")), "old");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_replace_merges_folders_and_keeps_destination_only_items`
/// for devices: a folder merge on a phone behaves as on a local disk
/// (XFER-026).
///
/// parity: XFER-009, XFER-026
#[test]
fn device_replace_merges_folders_and_keeps_destination_only_items() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("d");
    fs::create_dir(&source).expect("create the source folder");
    write(&source.join("same"), "new");
    write(&source.join("added"), "added");
    let existing = fixture.destination_folder.join("d");
    fs::create_dir(&existing).expect("create the existing folder");
    write(&existing.join("same"), "old");
    write(&existing.join("keep"), "keep");
    let phone = Arc::new(Device::default());

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Replace);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(list(&existing), ["added", "keep", "same"]);
    assert_eq!(read(&existing.join("same")), "new");
    assert_eq!(read(&existing.join("added")), "added");
    assert_eq!(read(&existing.join("keep")), "keep");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    phone.assert_only_moves_a_device_can_do();
}

/// Records whether each move to a backup name could be cancelled.
#[derive(Default)]
struct WatchedAside {
    device: Device,
    cancellable_asides: Mutex<Vec<bool>>,
}

impl Provider for WatchedAside {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if is_backup(target) {
            self.cancellable_asides
                .lock()
                .expect("aside log")
                .push(cancel.is_some());
        }
        self.device.move_native(node, target, cancel)
    }
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_replace_move_aside_is_not_cancellable`: a device can finish
/// a rename after the client stopped waiting, so the move-aside is never
/// interrupted.
///
/// parity: XFER-010
#[test]
fn the_device_move_aside_cannot_be_cancelled() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.replacement_source("photo", "new", "old");
    let phone = Arc::new(WatchedAside::default());

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Replace);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(*phone.cancellable_asides.lock().expect("aside log"), [false]);
    assert_eq!(read(&fixture.destination_folder.join("photo")), "new");
}

/// Finishes the first move to a backup name but reports an error, like a
/// device answering after the client's wait was cancelled (`LateRename`).
#[derive(Default)]
struct LateAside {
    device: Device,
    /// Set once the error was reported, so later moves succeed.
    reported: AtomicBool,
}

impl Provider for LateAside {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        self.device.move_native(node, target, cancel)?;
        let first_aside = is_backup(target) && !self.reported.swap(true, Ordering::SeqCst);
        if first_aside {
            return Err(TransferError::failed("Operation was cancelled"));
        }
        Ok(())
    }
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_replace_move_aside_finished_by_the_device_is_restored`: the
/// original must not stay under the hidden backup name.
///
/// parity: XFER-010
#[test]
fn a_move_aside_the_device_finished_after_an_error_is_restored() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.replacement_source("photo", "new", "old");

    let result = fixture.copy(
        Arc::new(LateAside::default()),
        &[&source],
        ConflictPolicy::Replace,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("photo")), "old");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Another program writes `photo` just before the staged upload is
/// published, like `Racing` in `test_device_staging.py`.
#[derive(Default)]
struct RacingWriter {
    device: Device,
}

impl Provider for RacingWriter {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if is_staging(node) {
            write(&local_path_of(target), "racing writer");
        }
        self.device.move_native(node, target, cancel)
    }
}

/// Ported from `desktop/tests/test_device_staging.py::DeviceStagingTests::test_publish_race_never_overwrites` for devices.
///
/// parity: XFER-007
#[test]
fn a_name_taken_during_an_upload_is_never_overwritten() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("photo");
    write(&source, "new");

    let result = fixture.copy(
        Arc::new(RacingWriter::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("photo")), "racing writer");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}
