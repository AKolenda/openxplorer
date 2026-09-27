// SPDX-License-Identifier: AGPL-3.0-only
//! Copies to simulated MTP devices: staging, publication, Replace and the
//! moves a device can perform. Ports `DeviceStagingTests` in
//! `desktop/tests/test_device_staging.py`. No real devices.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, TransferError, TransferMode};

use crate::transfer_support::{
    device::Device,
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// A phone whose object paths behave like `GVfs` MTP: after a cross-folder
/// move the old path keeps resolving to the moved object until its folder
/// is listed again.
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
            *self.stale_move.lock().expect("stale move") =
                Some((node.local_path().to_path_buf(), destination));
        }
        Ok(())
    }

    fn exists(&self, node: &LocalNode, _cancel: Option<&Cancellation>) -> bool {
        if let Some((old, new)) = &*self.stale_move.lock().expect("stale move") {
            if node.local_path() == old {
                return new.exists();
            }
        }
        node.local_exists()
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if let Some((old, new)) = &*self.stale_move.lock().expect("stale move") {
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
        let mut stale = self.stale_move.lock().expect("stale move");
        let relisted_old_folder = stale
            .as_ref()
            .is_some_and(|(old, _)| old.parent() == Some(node.local_path()));
        if relisted_old_folder {
            *stale = None;
        }
        Ok(())
    }
}

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
        let source = fixture.src.join("incoming");
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
        let published = fixture.dst.join("incoming");
        let target = if kind == NodeKind::Directory {
            published.join("photo.jpg")
        } else {
            published.clone()
        };
        assert_eq!(read(&target), "photo");
        let moves = phone.device.moves();
        assert_eq!(moves.len(), 1, "{moves:?}");
        let (staged, final_name) = &moves[0];
        assert_eq!(staged.parent(), Some(fixture.dst.as_path()));
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
    let tree = fixture.src.join("tree");
    fs::create_dir_all(tree.join("sub")).expect("create the tree");
    write(&tree.join("sub/b.txt"), "b");
    write(&tree.join("a.txt"), "a");
    let phone = Arc::new(WatchedPhone {
        watched_name: fixture.dst.join("tree"),
        ..WatchedPhone::default()
    });
    let result = fixture.copy(phone.clone(), &[&tree], ConflictPolicy::Skip);
    assert!(result.errors.is_empty(), "{result:?}");
    let uploads = phone.uploads.lock().expect("upload log");
    assert_eq!(uploads.len(), 2);
    for (final_visible, written) in uploads.iter() {
        assert!(!final_visible);
        let relative = written.strip_prefix(&fixture.dst).expect("inside the phone");
        let first = relative.components().next().expect("a staged path");
        assert!(is_staging_path(&PathBuf::from(first.as_os_str())));
    }
    assert_eq!(read(&fixture.dst.join("tree/sub/b.txt")), "b");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    phone.device.assert_only_same_folder_renames_across_names();
}

/// Port of `test_second_copy_into_same_folder_succeeds`. The old engine
/// left the first copy as "payload", and the second failed with "libmtp
/// error: could not move object".
///
/// parity: XFER-021
#[test]
fn a_second_upload_into_the_same_folder_succeeds() {
    let fixture = Fixture::with_destination("phone");
    let first = fixture.src.join("a.apk");
    let second = fixture.src.join("b.apk");
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
    assert_eq!(list(&fixture.dst), ["a.apk", "b.apk"]);
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
        let source = fixture.src.join("photo.jpg");
        write(&source, "new");
        if let Some(existing) = case.existing {
            write(&fixture.dst.join("photo.jpg"), existing);
        }
        let phone = Arc::new(Device::without_move_object());
        let result = fixture.copy(phone.clone(), &[&source], case.policy);
        assert!(result.errors.is_empty(), "{:?}: {result:?}", case.policy);
        assert_eq!(read(&fixture.dst.join(case.published)), "new");
        if case.policy == ConflictPolicy::KeepBoth {
            assert_eq!(read(&fixture.dst.join("photo.jpg")), "old");
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
    let album = fixture.src.join("album");
    fs::create_dir(&album).expect("create the album");
    write(&album.join("photo.jpg"), "photo");
    let result = fixture.copy(
        Arc::new(Device::without_move_object()),
        &[&album],
        ConflictPolicy::Skip,
    );
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.dst.join("album/photo.jpg")), "photo");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Port of `test_skip_never_touches_existing` for devices.
///
/// parity: XFER-006
#[test]
fn skip_on_a_device_never_touches_the_existing_item() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.src.join("a");
    write(&source, "new");
    write(&fixture.dst.join("a"), "old");
    let phone = Arc::new(Device::default());
    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::Skip);
    assert_eq!(result.skipped, [uri(&source)]);
    assert_eq!(read(&fixture.dst.join("a")), "old");
    assert!(phone.moves().is_empty());
}

/// Port of `test_cancel_mid_copy_leaves_no_stage_and_no_final_name`.
///
/// parity: OPS-022, XFER-021
#[test]
fn cancelling_an_upload_leaves_no_stage_and_no_final_name() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.src.join("big");
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
    assert!(list(&fixture.dst).is_empty(), "{:?}", list(&fixture.dst));
}

/// Port of `test_same_device_keep_both_renames_inside_the_private_folder`
/// and `test_same_device_file_copy_is_built_inside_a_private_folder`.
///
/// parity: XFER-023
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
    let result = fixture.copy(phone.clone(), &[&source], ConflictPolicy::KeepBoth);
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.dst.join("photo.jpg")), "original");
    assert_eq!(read(&fixture.dst.join("photo (copy 2).jpg")), "incoming");
    assert_eq!(read(&source), "incoming");
    phone.device.assert_only_same_folder_renames_across_names();
    assert_eq!(phone.device.moves().len(), 2);
    assert_eq!(phone.device.refreshes().len(), 2);
    assert!(phone.stale_move.lock().expect("stale move").is_none());
    fixture.no_stage();
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
    let source = fixture.src.join("photo.jpg");
    write(&source, "photo");
    let phone = Arc::new(Phone {
        device: Device::without_move_object(),
        same_device_copy: true,
        ..Phone::default()
    });

    let result = fixture.copy(phone, &[&source], ConflictPolicy::Skip);

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(
        result.errors[0].contains("cannot move items between folders"),
        "{result:?}"
    );
    assert!(list(&fixture.dst).is_empty(), "{:?}", list(&fixture.dst));
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

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_dir: &dyn Node) -> bool {
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
    let source = fixture.src.join("a.txt");
    write(&source, "data");

    let result = fixture.copy(
        Arc::new(FailingSameDeviceCopy::default()),
        &[&source],
        ConflictPolicy::Skip,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("device copy failed"), "{result:?}");
    assert!(list(&fixture.dst).is_empty(), "{:?}", list(&fixture.dst));
    assert_eq!(read(&source), "data");
}

/// Port of `test_device_move_relists_each_source_folder_once`.
///
/// parity: XFER-025
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

/// Port of `test_device_move_with_new_name_is_refused_not_misnamed`.
///
/// parity: XFER-024
#[test]
fn a_device_move_that_needs_a_new_name_is_refused_not_misnamed() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.src.join("a");
    write(&source, "new");
    write(&fixture.dst.join("a"), "old");
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
    assert_eq!(list(&fixture.dst), ["a"]);
}

/// Records the path of every file copy, like `Watching` in
/// `test_local_destinations_keep_directory_staging`.
#[derive(Default)]
struct WatchedLocal {
    targets: Mutex<Vec<PathBuf>>,
}

impl Provider for WatchedLocal {
    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        self.targets
            .lock()
            .expect("target log")
            .push(local_path_of(target));
        node.local_copy_file(target, cancel, progress)
    }
}

/// Port of `test_local_destinations_keep_directory_staging`.
///
/// parity: XFER-001
#[test]
fn local_destinations_keep_a_private_staging_folder_with_a_payload() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
    write(&source, "a");
    let watched = Arc::new(WatchedLocal::default());
    let result = fixture.copy(watched.clone(), &[&source], ConflictPolicy::Skip);
    assert!(result.errors.is_empty(), "{result:?}");
    let targets = watched.targets.lock().expect("target log");
    assert_eq!(targets[0].file_name(), Some("payload".as_ref()));
    let folder = targets[0].parent().expect("payload has a parent");
    assert!(is_staging_path(folder), "{}", folder.display());
    assert_eq!(folder.parent(), Some(fixture.dst.as_path()));
    assert_eq!(read(&fixture.dst.join("a")), "a");
}
