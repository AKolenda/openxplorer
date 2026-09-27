// SPDX-License-Identifier: AGPL-3.0-only
//! Previous-version (snapshot) folders stay read-only through every
//! operation (XFER-020). Ports `ProtectedTransferTests` in
//! `desktop/tests/test_operations.py`.

use std::fs;
use std::os::unix::fs::symlink;

use ox_core::transfer::{ConflictPolicy, TransferMode};

use crate::transfer_support::{local, versions::PreviousVersions, *};

/// Port of `test_replace_cannot_overwrite_nested_snapshot`: a snapshot
/// inside the existing folder stops the whole Replace before any file in
/// that folder changes.
///
/// parity: XFER-020
#[test]
fn replace_never_overwrites_a_nested_snapshot() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    let target = fixture.destination_folder.join("project");
    for (folder, content) in [(&source, "incoming"), (&target, "original")] {
        fs::create_dir_all(folder.join(".snapshot")).expect("create the snapshot folder");
        write(&folder.join("ordinary.txt"), content);
        write(&folder.join(".snapshot/version.txt"), content);
    }
    let versions = PreviousVersions::new();
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert!(result.errors[0].contains("read-only"), "{result:?}");
    assert_eq!(read(&target.join("ordinary.txt")), "original");
    assert_eq!(read(&target.join(".snapshot/version.txt")), "original");
    fixture.assert_no_staging();
}

/// Port of `test_configured_backup_descendant_is_protected`: a configured
/// snapshot folder inside the selection stops a permanent delete.
///
/// parity: XFER-020
#[test]
fn a_configured_snapshot_folder_inside_a_deleted_folder_survives() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    let backup = source.join("history");
    fs::create_dir_all(&backup).expect("create the backup folder");
    write(&backup.join("version.txt"), "backup");
    let versions = PreviousVersions::new();
    versions.configure(&file_uri(&source), &file_uri(&backup));
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Delete,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(read(&backup.join("version.txt")), "backup");
}

/// Port of `test_snapshot_file_can_be_restored_to_another_folder`.
///
/// parity: XFER-020
#[test]
fn a_snapshot_file_can_be_copied_to_another_folder() {
    let fixture = Fixture::new();
    let snapshot = fixture.source_folder.join(".snapshot");
    fs::create_dir(&snapshot).expect("create the snapshot folder");
    let saved = snapshot.join("document.txt");
    write(&saved, "saved");
    let versions = PreviousVersions::new();
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

    let result = fixture.run(
        &mut engine,
        &[&saved],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("document.txt")), "saved");
}

/// Port of `test_symlink_to_snapshot_is_removed_without_traversal`: the
/// link itself is not protected, and deleting it never enters the
/// snapshot it points to.
///
/// parity: XFER-015, XFER-017, XFER-020
#[test]
fn a_link_to_a_snapshot_is_deleted_without_entering_the_snapshot() {
    let fixture = Fixture::new();
    let snapshot = fixture.source_folder.join(".snapshot");
    fs::create_dir(&snapshot).expect("create the snapshot folder");
    write(&snapshot.join("version.txt"), "backup");
    let link = fixture.source_folder.join("shortcut");
    symlink(&snapshot, &link).expect("create the link");
    let versions = PreviousVersions::new();
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

    let result = fixture.run(
        &mut engine,
        &[&link],
        TransferMode::Delete,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert!(!exists_without_following_links(&link));
    assert_eq!(read(&snapshot.join("version.txt")), "backup");
}

/// Port of `test_removal_or_move_preserves_whole_tree_containing_snapshot`.
///
/// parity: XFER-020
#[test]
fn protected_descendants_stop_mutations_before_any_item_changes() {
    for mode in [TransferMode::Move, TransferMode::Trash, TransferMode::Delete] {
        let fixture = Fixture::new();
        let folder = fixture.source_folder.join("tree");
        fs::create_dir_all(folder.join(".snapshot")).unwrap();
        write(&folder.join("a"), "live");
        write(&folder.join(".snapshot/old"), "snapshot");
        let versions = PreviousVersions::new();
        let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

        let result = fixture.run(&mut engine, &[&folder], mode, ConflictPolicy::Replace, None);

        assert!(result.done.is_empty());
        assert!(result.errors[0].contains("read-only"));
        assert_eq!(read(&folder.join("a")), "live");
        assert_eq!(read(&folder.join(".snapshot/old")), "snapshot");
        assert!(list(&fixture.destination_folder).is_empty());
    }
}

/// A configured snapshot folder is read-only as a destination, while a copy
/// out of it is allowed.
///
/// parity: XFER-020
#[test]
fn configured_snapshot_destination_is_protected_but_restoring_a_copy_is_allowed() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("photo.jpg");
    write(&source, "photo");
    let versions = PreviousVersions::new();
    versions.configure(
        &file_uri(&fixture.destination_folder),
        &file_uri(&fixture.source_folder),
    );
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.done, [file_uri(&source)]);

    let reverse = fixture.run(
        &mut engine,
        &[&fixture.destination_folder.join("photo.jpg")],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        Some(&fixture.source_folder),
    );

    assert!(reverse.done.is_empty());
    assert!(reverse.errors[0].contains("read-only"));
    assert_eq!(read(&source), "photo");
}
