// SPDX-License-Identifier: AGPL-3.0-only
//! Device capabilities of the production GIO adapter, decided from URIs
//! alone without contacting a device: where copies are staged, whether MTP
//! `CopyObject` keeps the source's name, and which device moves are
//! refused. Ports the capability cases of
//! `v2.0.0:desktop/tests/test_device_staging.py`.

use ox_core::gio_node::GioNode;
use ox_core::transfer::{Node, TransferError};

/// A photo on an MTP device. No test here contacts a device.
const DEVICE_PHOTO: &str = "mtp://test-device/Internal/source/photo.jpg";

/// Ported from `v2.0.0:desktop/tests/test_device_staging.py::GioMtpAdapterTests::test_device_schemes_request_sibling_staging`:
/// only MTP locations stage beside the final name; cameras on gphoto2 keep
/// folder staging.
///
/// parity: XFER-021
#[test]
fn only_mtp_destinations_stage_beside_the_final_name() {
    let others = ["gphoto2://cam/DCIM/x", "smb://host/share/x", "file:///tmp/x"];

    let device_stages_beside = GioNode::new(DEVICE_PHOTO).has_sibling_staging();
    let others_stage_beside: Vec<bool> = others
        .iter()
        .map(|uri| GioNode::new(uri).has_sibling_staging())
        .collect();

    assert!(device_stages_beside);
    assert_eq!(others_stage_beside, [false, false, false], "{others:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_device_staging.py::GioMtpAdapterTests::test_same_device_copies_are_detected`:
/// MTP `CopyObject` keeps the source's name, which only matters for a copy
/// within one device.
///
/// parity: XFER-023
#[test]
fn only_a_copy_within_one_mtp_device_keeps_the_source_name() {
    let source = GioNode::new(DEVICE_PHOTO);
    let same_device = GioNode::new("mtp://test-device/Internal/destination");
    let other_device = GioNode::new("mtp://other-device/Internal/destination");
    let local = GioNode::new("file:///tmp/x");

    let within_the_device = source.native_copy_keeps_name(&same_device);
    let to_another_device = source.native_copy_keeps_name(&other_device);
    let to_a_local_folder = source.native_copy_keeps_name(&local);
    let from_a_local_file = local.native_copy_keeps_name(&same_device);

    assert!(within_the_device);
    assert!(!to_another_device);
    assert!(!to_a_local_folder);
    assert!(!from_a_local_file);
}

/// A device cannot move an item to another folder under a new name in one
/// step, and it has no safe overwrite (XFER-026): both are refused before
/// the device is contacted.
///
/// parity: XFER-024
#[test]
fn a_device_move_under_a_new_name_or_with_replace_is_refused_without_device_io() {
    let source = GioNode::new(DEVICE_PHOTO);
    let renamed = GioNode::new("mtp://test-device/Internal/destination/other.jpg");

    let moved = source.move_native(&renamed, None);
    let replaced = source.replace_native(&renamed, None);

    assert!(moved.unwrap_err().to_string().contains("not both"));
    assert!(
        matches!(replaced, Err(TransferError::ReplaceUnsupported(_))),
        "{replaced:?}"
    );
}
