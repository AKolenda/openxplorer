// SPDX-License-Identifier: AGPL-3.0-only
//! Uploads to simulated MTP devices, and the moves a device can perform.
//! Ports the staging and move cases of `DeviceStagingTests` in
//! `desktop/tests/test_device_staging.py`; copies within one device are in
//! `same_device.rs`, Replace in `device_replace.rs`. No real devices.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, TransferError, TransferMode};

use crate::transfer_support::{
    device::{Device, Phone},
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// Port of `test_file_copy_publishes_by_same_folder_rename` and
/// `test_partial_copy_never_visible_under_final_name`: files and folders
/// are built under a staging name beside the final name and published by
/// exactly one same-folder rename.
///
/// parity: XFER-021
#[test]
fn uploads_publish_by_one_same_folder_rename_from_a_staged_sibling() {
    for kind in [NodeKind::File, NodeKind::Directory] {
        let fixture = Fixture::with_destination("phone");
        let source = fixture.source_folder.join("incoming");
        if kind == NodeKind::Directory {
            fs::create_dir(&source).expect("create the source folder");
            write(&source.join("photo.jpg"), "photo");
        } else {
            write(&source, "photo");
        }
        let phone = Arc::new(Phone::default());
        let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [uri(&source)]);
        let published = fixture.destination_folder.join("incoming");
        let target = if kind == NodeKind::Directory {
            published.join("photo.jpg")
        } else {
            published.clone()
        };
        assert_eq!(read(&target), "photo");
        let moves = phone.device.moves();
        assert_eq!(moves.len(), 1, "{moves:?}");
        let (staged, final_name) = &moves[0];
        assert_eq!(staged.parent(), Some(fixture.destination_folder.as_path()));
        assert!(is_staging_path(staged), "{}", staged.display());
        assert_eq!(final_name, &published);
        assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    }
}

/// Records, for every file the device receives, whether the final name was
/// already visible and where the file was written.
#[derive(Default)]
struct WatchedPhone {
    device: Device,
    watched_name: PathBuf,
    uploads: Mutex<Vec<(bool, PathBuf)>>,
}

impl Provider for WatchedPhone {
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
        let final_visible = lexists(&self.watched_name);
        let upload = (final_visible, local_path_of(target));
        self.uploads.lock().expect("upload log").push(upload);
        node.local_copy_file(target, cancel, progress)
    }
}

/// Port of `test_partial_copy_never_visible_under_final_name`.
///
/// parity: XFER-001, XFER-021
#[test]
fn a_partial_folder_upload_is_never_visible_under_its_final_name() {
    let fixture = Fixture::with_destination("phone");
    let tree = fixture.source_folder.join("tree");
    fs::create_dir_all(tree.join("sub")).expect("create the tree");
    write(&tree.join("sub/b.txt"), "b");
    write(&tree.join("a.txt"), "a");
    let phone = Arc::new(WatchedPhone {
        watched_name: fixture.destination_folder.join("tree"),
        ..WatchedPhone::default()
    });
    let result = fixture.copy(phone.clone(), &[&tree], ConflictPolicy::Skip);
    assert!(result.errors.is_empty(), "{result:?}");
    let uploads = phone.uploads.lock().expect("upload log");
    assert_eq!(uploads.len(), 2);
    for (final_visible, written) in uploads.iter() {
        assert!(!final_visible);
        let relative = written
            .strip_prefix(&fixture.destination_folder)
            .expect("inside the phone");
        let first = relative.components().next().expect("a staged path");
        assert!(is_staging_path(&PathBuf::from(first.as_os_str())));
    }
    assert_eq!(read(&fixture.destination_folder.join("tree/sub/b.txt")), "b");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    phone.device.assert_only_moves_a_device_can_do();
}

/// Port of `test_second_copy_into_same_folder_succeeds`. The old engine
/// left the first copy as "payload", and the second failed with "libmtp
/// error: could not move object".
///
/// parity: XFER-021
#[test]
fn a_second_upload_into_the_same_folder_succeeds() {
    let fixture = Fixture::with_destination("phone");
    let first = fixture.source_folder.join("a.apk");
    let second = fixture.source_folder.join("b.apk");
    write(&first, "a");
    write(&second, "b");
    let phone = Arc::new(Device::default());
    assert!(fixture
        .copy(phone.clone(), &[&first], ConflictPolicy::Skip)
        .errors
        .is_empty());
    assert!(fixture
        .copy(phone, &[&second], ConflictPolicy::Skip)
        .errors
        .is_empty());
    assert_eq!(list(&fixture.destination_folder), ["a.apk", "b.apk"]);
}

/// One file upload under a conflict policy.
struct UploadCase {
    policy: ConflictPolicy,
    /// Content already at the destination name, if any.
    existing: Option<&'static str>,
    /// Where the new content must end up.
    published: &'static str,
}

/// Devices without MTP `MoveObject` (Android 7 and 8) refuse every
/// cross-folder move. Uploads publish by same-folder renames only, so file
/// copies work there with every policy, as in the Python app.
///
/// parity: XFER-021
#[test]
fn file_uploads_work_on_devices_without_move_object() {
    let cases = [
        UploadCase {
            policy: ConflictPolicy::Skip,
            existing: None,
            published: "photo.jpg",
        },
        UploadCase {
            policy: ConflictPolicy::KeepBoth,
            existing: Some("old"),
            published: "photo (copy 2).jpg",
        },
        UploadCase {
            policy: ConflictPolicy::Replace,
            existing: Some("old"),
            published: "photo.jpg",
        },
    ];
    for case in cases {
        let fixture = Fixture::with_destination("phone");
        let source = fixture.source_folder.join("photo.jpg");
        write(&source, "new");
        if let Some(existing) = case.existing {
            write(&fixture.destination_folder.join("photo.jpg"), existing);
        }
        let phone = Arc::new(Device::without_move_object());
        let result = fixture.copy(phone.clone(), &[&source], case.policy);
        assert!(result.errors.is_empty(), "{:?}: {result:?}", case.policy);
        assert_eq!(read(&fixture.destination_folder.join(case.published)), "new");
        if case.policy == ConflictPolicy::KeepBoth {
            assert_eq!(read(&fixture.destination_folder.join("photo.jpg")), "old");
        }
        assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    }
}

/// See [`file_uploads_work_on_devices_without_move_object`].
///
/// parity: XFER-021
#[test]
fn folder_uploads_work_on_devices_without_move_object() {
    let fixture = Fixture::with_destination("phone");
    let album = fixture.source_folder.join("album");
    fs::create_dir(&album).expect("create the album");
    write(&album.join("photo.jpg"), "photo");
    let result = fixture.copy(
        Arc::new(Device::without_move_object()),
        &[&album],
        ConflictPolicy::Skip,
    );
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("album/photo.jpg")), "photo");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Port of `test_skip_never_touches_existing` for devices.
///
/// parity: XFER-006
#[test]
fn skip_on_a_device_never_touches_the_existing_item() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("a");
    write(&source, "new");
    write(&fixture.destination_folder.join("a"), "old");
    let phone = Arc::new(Device::default());
    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
    assert_eq!(result.skipped, [uri(&source)]);
    assert_eq!(read(&fixture.destination_folder.join("a")), "old");
    assert!(phone.moves().is_empty());
}

/// Port of `test_cancel_mid_copy_leaves_no_stage_and_no_final_name`.
///
/// parity: OPS-022, XFER-021
#[test]
fn cancelling_an_upload_leaves_no_stage_and_no_final_name() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("big");
    fs::write(&source, random_bytes(100_000)).expect("write the source");
    let cancel = fixture.cancel.clone();
    let mut engine = fixture
        .engine(Arc::new(Device::default()))
        .with_progress(move |progress| {
            if progress.label.starts_with("Copying ") {
                cancel.cancel();
            }
        });
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert!(result.cancelled);
    assert!(result.errors.is_empty(), "{result:?}");
    assert!(
        list(&fixture.destination_folder).is_empty(),
        "{:?}",
        list(&fixture.destination_folder)
    );
}

/// Port of `test_device_move_relists_each_source_folder_once`.
///
/// parity: XFER-025
#[test]
fn device_moves_relist_the_old_folder_once_per_batch() {
    let fixture = Fixture::new();
    let first = fixture.source_folder.join("first");
    let second = fixture.source_folder.join("second");
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
    assert_eq!(
        device.refreshes().as_slice(),
        std::slice::from_ref(&fixture.source_folder)
    );
    assert_eq!(read(&fixture.destination_folder.join("first")), "first");
    assert_eq!(read(&fixture.destination_folder.join("second")), "second");
    device.assert_only_moves_a_device_can_do();
}

/// Port of `test_device_move_with_new_name_is_refused_not_misnamed`.
///
/// parity: XFER-024
#[test]
fn a_device_move_that_needs_a_new_name_is_refused_not_misnamed() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("a");
    write(&source, "new");
    write(&fixture.destination_folder.join("a"), "old");
    let mut engine = fixture.engine(Arc::new(Device::default()));
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::KeepBoth,
        None,
    );
    assert!(result.done.is_empty());
    assert!(result.errors[0].contains("not both"), "{result:?}");
    assert_eq!(read(&source), "new");
    assert_eq!(list(&fixture.destination_folder), ["a"]);
}
