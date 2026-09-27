// SPDX-License-Identifier: AGPL-3.0-only
//! Copies within one simulated MTP device, where `CopyObject` keeps the
//! source's name whatever target is asked for. Ports the same-device cases
//! of `DeviceStagingTests` in `desktop/tests/test_device_staging.py`. No
//! real devices.

use std::sync::Arc;

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, TransferError};

use crate::transfer_support::{
    device::{Device, Phone},
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// Port of `test_same_device_keep_both_renames_inside_the_private_folder`
/// and `test_same_device_file_copy_is_built_inside_a_private_folder`.
///
/// parity: XFER-023
#[test]
fn same_device_keep_both_renames_inside_staging_and_refreshes_before_cleanup() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("photo.jpg");
    write(&source, "incoming");
    write(&fixture.destination_folder.join("photo.jpg"), "original");
    let phone = Arc::new(Phone::with_same_device_copies(Device::default()));

    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::KeepBoth);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("photo.jpg")), "original");
    assert_eq!(
        read(&fixture.destination_folder.join("photo (copy 2).jpg")),
        "incoming"
    );
    assert_eq!(read(&source), "incoming");
    phone.device.assert_only_moves_a_device_can_do();
    assert_eq!(phone.device.moves().len(), 2);
    assert_eq!(phone.device.refreshes().len(), 2);
    assert!(!phone.has_stale_move());
    fixture.assert_no_staging();
}

/// A copy within one device is moved out of its private folder at the end,
/// which devices without MTP `MoveObject` (Android 7 and 8) refuse. The
/// error says so instead of blaming a cross-filesystem move, and nothing is
/// left behind.
///
/// parity: XFER-023
#[test]
fn a_copy_within_a_device_without_move_object_explains_the_refusal() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("photo.jpg");
    write(&source, "photo");
    let phone = Arc::new(Phone::with_same_device_copies(Device::without_move_object()));

    let result = fixture.copy(phone, &[&source], ConflictPolicy::Skip);

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(
        result.errors[0].contains("cannot move items between folders"),
        "{result:?}"
    );
    assert!(
        list(&fixture.destination_folder).is_empty(),
        "{:?}",
        list(&fixture.destination_folder)
    );
    assert_eq!(read(&source), "photo");
}

/// A copy within one device that fails after `CopyObject` already placed
/// part of it in the private folder, like `SameDevice` in
/// `test_same_device_copy_failure_leaves_nothing_under_the_final_name`.
#[derive(Default)]
struct FailingSameDeviceCopy {
    device: Device,
}

impl Provider for FailingSameDeviceCopy {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_folder: &dyn Node) -> bool {
        true
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        _cancel: &Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        let folder = local_path_of(target)
            .parent()
            .expect("the copy lands in a folder")
            .to_path_buf();
        write(&folder.join(node.name()), "partial");
        Err(TransferError::failed("device copy failed"))
    }
}

/// Port of `test_same_device_copy_failure_leaves_nothing_under_the_final_name`.
///
/// parity: XFER-001, XFER-023
#[test]
fn a_failed_copy_within_one_device_leaves_nothing_behind() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("a.txt");
    write(&source, "data");

    let result = fixture.copy(
        Arc::new(FailingSameDeviceCopy::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("device copy failed"), "{result:?}");
    assert!(
        list(&fixture.destination_folder).is_empty(),
        "{:?}",
        list(&fixture.destination_folder)
    );
    assert_eq!(read(&source), "data");
}

/// A copy within one device that reports success without placing anything
/// in the private folder.
#[derive(Default)]
struct FalseSuccessSameDeviceCopy {
    device: Device,
}

impl Provider for FalseSuccessSameDeviceCopy {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_folder: &dyn Node) -> bool {
        true
    }

    fn copy_file(
        &self,
        _node: &LocalNode,
        _target: &dyn Node,
        _cancel: &Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        Ok(())
    }
}

/// A device's success report for `CopyObject` is not proof: the copy must
/// be in the private folder before it is renamed or published.
///
/// parity: XFER-023
#[test]
fn a_copy_within_one_device_that_was_never_placed_is_not_published() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("photo.jpg");
    write(&source, "photo");

    let result = fixture.copy(
        Arc::new(FalseSuccessSameDeviceCopy::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("did not place the copy"), "{result:?}");
    assert!(
        list(&fixture.destination_folder).is_empty(),
        "{:?}",
        list(&fixture.destination_folder)
    );
    assert_eq!(read(&source), "photo");
}
