// SPDX-License-Identifier: AGPL-3.0-only
//! The production GIO adapter on a simulated MTP device: which device calls
//! each move, rename and deletion makes. Ports `GioMtpAdapterTests` in
//! `desktop/tests/test_device_staging.py`.

use ox_core::transfer::{Cancellation, Node, TransferError};

use crate::transfer_support::mtp_device::{DeviceCall, FakeDevice, RenameAnswer};

/// The flags of every native move: `MOVE_FLAGS` in `desktop/gio_backend.py`.
fn move_flags() -> gio::FileCopyFlags {
    gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE
}

/// Port of `test_same_folder_move_is_set_display_name`.
///
/// parity: XFER-024
#[test]
fn a_same_folder_move_is_one_device_rename() {
    let device = FakeDevice::new();
    device.add_file(".stage");

    let moved = device
        .node(".stage")
        .move_native(&device.node("t3code.apk"), None);

    assert_eq!(moved, Ok(()));
    assert_eq!(
        device.calls(),
        [DeviceCall::Rename {
            from: device.uri(".stage"),
            name: "t3code.apk".into(),
        }]
    );
    assert_eq!(device.items(), ["t3code.apk"]);
}

/// Port of `test_same_folder_move_onto_taken_name_is_refused_before_device_call`.
///
/// parity: XFER-024
#[test]
fn a_rename_onto_a_taken_name_is_refused_before_any_device_call() {
    let device = FakeDevice::new();
    device.add_file(".stage");
    device.add_file("a");

    let moved = device.node(".stage").move_native(&device.node("a"), None);

    assert!(matches!(moved, Err(TransferError::Exists(_))), "{moved:?}");
    assert!(device.calls().is_empty());
    assert_eq!(device.items(), [".stage", "a"]);
}

/// Port of `test_cross_folder_move_with_new_name_is_refused_without_device_call`:
/// `GVfs` would report success and keep the old name.
///
/// parity: XFER-024
#[test]
fn a_move_to_another_folder_under_a_new_name_is_refused_without_a_device_call() {
    let device = FakeDevice::new();
    device.add_file("x/payload");

    let moved = device
        .node("x/payload")
        .move_native(&device.node("t3code.apk"), None);

    let message = moved
        .expect_err("the device cannot rename while moving")
        .to_string();
    assert!(message.contains("not both in one step"), "{message}");
    assert!(device.calls().is_empty());
}

/// Port of `test_cross_folder_move_with_same_name_uses_no_fallback_move`.
///
/// parity: XFER-011, XFER-024
#[test]
fn a_move_to_another_folder_keeps_the_name_and_never_falls_back_to_copying() {
    let device = FakeDevice::new();
    device.add_file("x/a");
    device.add_folder("y");

    let moved = device.node("x/a").move_native(&device.node("y/a"), None);

    assert_eq!(moved, Ok(()));
    assert_eq!(
        device.calls(),
        [DeviceCall::Move {
            from: device.uri("x/a"),
            to: device.uri("y/a"),
            flags: move_flags(),
        }]
    );
}

/// Port of `test_replace_never_uses_device_overwrite`: `GVfs` deletes the
/// existing item before it moves and cannot restore it.
///
/// parity: XFER-010
#[test]
fn replace_on_a_device_is_never_attempted_in_one_step() {
    let device = FakeDevice::new();
    device.add_file(".stage");
    device.add_file("a");

    let replaced = device.node(".stage").replace_native(&device.node("a"), None);

    assert!(
        matches!(replaced, Err(TransferError::ReplaceUnsupported(_))),
        "{replaced:?}"
    );
    assert!(device.calls().is_empty());
}

/// Port of `test_rename_the_device_finished_after_an_error_is_success`.
///
/// parity: XFER-024
#[test]
fn a_rename_the_device_finished_after_reporting_an_error_counts_as_done() {
    let device = FakeDevice::new();
    device.add_file(".stage");
    device.answer_renames(RenameAnswer::DoneButReportsError);

    let moved = device.node(".stage").move_native(&device.node("a"), None);

    assert_eq!(moved, Ok(()));
    assert_eq!(device.items(), ["a"]);
}

/// A rename the device refused because another program took the name in
/// the meantime is reported as a taken name, and both items stay.
///
/// parity: XFER-024
#[test]
fn a_rename_refused_because_the_name_was_taken_meanwhile_reports_the_taken_name() {
    let device = FakeDevice::new();
    device.add_file(".stage");
    device.answer_renames(RenameAnswer::NameTakenMeanwhile);

    let moved = device.node(".stage").move_native(&device.node("a"), None);

    assert!(matches!(moved, Err(TransferError::Exists(_))), "{moved:?}");
    assert_eq!(device.items(), [".stage", "a"]);
}

/// A rename the device refused without changing anything reports the
/// device's own error; it is never counted as done.
///
/// parity: XFER-024
#[test]
fn a_refused_rename_reports_the_devices_error() {
    let device = FakeDevice::new();
    device.add_file(".stage");
    device.answer_renames(RenameAnswer::Refused);

    let moved = device.node(".stage").move_native(&device.node("a"), None);

    assert_eq!(
        moved,
        Err(TransferError::failed("libmtp error: could not rename"))
    );
    assert_eq!(device.items(), [".stage"]);
}

/// Phones have no Trash, so a permanent delete is the only way to remove a
/// folder there. It deletes the children first, because the device only
/// removes empty folders, and never touches the rest of the device.
///
/// parity: XFER-015
#[test]
fn a_folder_on_a_phone_is_deleted_permanently_children_first() {
    let device = FakeDevice::new();
    device.add_file("album/a.jpg");
    device.add_file("album/sub/b.jpg");
    device.add_file("kept.jpg");

    let deleted = device.node("album").delete_tree(&Cancellation::new(), None);

    assert_eq!(deleted, Ok(()));
    assert_eq!(
        device.calls(),
        [
            DeviceCall::Delete(device.uri("album/a.jpg")),
            DeviceCall::Delete(device.uri("album/sub/b.jpg")),
            DeviceCall::Delete(device.uri("album/sub")),
            DeviceCall::Delete(device.uri("album")),
        ]
    );
    assert_eq!(device.items(), ["kept.jpg"]);
}
