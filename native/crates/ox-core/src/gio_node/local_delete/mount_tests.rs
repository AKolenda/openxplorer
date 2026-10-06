// SPDX-License-Identifier: AGPL-3.0-only
//! A permanent deletion never leaves the drive the item is on. The tests
//! that mount run in a user and mount namespace of their own, with bind
//! mounts of temporary folders standing in for drives.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::test_support::{in_private_mount_namespace, temporary_folder, BindMount};

/// A temporary "drive" with one file in it, to be mounted somewhere.
fn drive_with_a_file(root: &Path) -> (PathBuf, PathBuf) {
    let drive = root.join("drive");
    fs::create_dir(&drive).expect("the drive folder is created");
    let file = drive.join("on the drive.txt");
    fs::write(&file, b"keep me").expect("the drive's file is written");
    (drive, file)
}

/// Deletes without the mount table check, as when a drive is mounted
/// after it: only the walk's own checks apply.
fn delete_by_walk_only(path: &Path) -> Result<(), TransferError> {
    let cancel = Cancellation::new();
    let mut deletion = Deletion {
        cancel: Some(&cancel),
        guard: None,
        max_depth: MAX_DEPTH,
        folders: FolderAccess::AsFound,
        root_identity: None,
        drive: None,
    };
    deletion.delete_path(path)
}

/// parity: XFER-015
#[test]
fn a_folder_where_a_drive_is_mounted_is_refused() {
    if !in_private_mount_namespace(concat!(
        module_path!(),
        "::a_folder_where_a_drive_is_mounted_is_refused"
    )) {
        return;
    }
    let root = temporary_folder();
    let (drive, file) = drive_with_a_file(root.path());
    let mount_point = root.path().join("Data");
    fs::create_dir(&mount_point).expect("the mount point is created");
    let _mounted = BindMount::new(&drive, &mount_point);

    let result = delete_tree(&mount_point, &Cancellation::new(), None);
    let by_walk = delete_by_walk_only(&mount_point);

    assert_eq!(result, Err(mount_point_refusal(&mount_point)));
    assert_eq!(by_walk, Err(mount_point_refusal(&mount_point)));
    assert_eq!(fs::read(&file).expect("the drive's file is kept"), b"keep me");
    assert!(mount_point.join("on the drive.txt").exists());
}

/// parity: XFER-015
#[test]
fn a_folder_with_a_drive_mounted_inside_is_refused_before_anything_is_deleted() {
    if !in_private_mount_namespace(concat!(
        module_path!(),
        "::a_folder_with_a_drive_mounted_inside_is_refused_before_anything_is_deleted"
    )) {
        return;
    }
    let root = temporary_folder();
    let (drive, file) = drive_with_a_file(root.path());
    let selected = root.path().join("selected");
    let mount_point = selected.join("sub/mnt");
    fs::create_dir_all(&mount_point).expect("the folders are created");
    fs::write(selected.join("a.txt"), b"mine").expect("the selected folder's file is written");
    let _mounted = BindMount::new(&drive, &mount_point);

    let result = delete_tree(&selected, &Cancellation::new(), None);

    let message = result.expect_err("the deletion is refused").to_string();
    assert!(message.contains(&mount_point.display().to_string()), "{message}");
    assert!(message.contains("Nothing was deleted"), "{message}");
    assert!(selected.join("a.txt").exists());
    assert_eq!(fs::read(&file).expect("the drive's file is kept"), b"keep me");
}

/// parity: XFER-015
#[test]
fn the_walk_never_enters_a_drive_mounted_inside() {
    if !in_private_mount_namespace(concat!(
        module_path!(),
        "::the_walk_never_enters_a_drive_mounted_inside"
    )) {
        return;
    }
    let root = temporary_folder();
    let (drive, file) = drive_with_a_file(root.path());
    let selected = root.path().join("selected");
    let mount_point = selected.join("mnt");
    fs::create_dir_all(&mount_point).expect("the folders are created");
    let _mounted = BindMount::new(&drive, &mount_point);

    let result = delete_by_walk_only(&selected);

    assert_eq!(result, Err(contained_mount_refusal(&mount_point)));
    assert_eq!(fs::read(&file).expect("the drive's file is kept"), b"keep me");
    assert!(mount_point.join("on the drive.txt").exists());
}

/// parity: XFER-015
#[test]
fn a_file_mounted_on_another_file_is_refused() {
    if !in_private_mount_namespace(concat!(
        module_path!(),
        "::a_file_mounted_on_another_file_is_refused"
    )) {
        return;
    }
    let root = temporary_folder();
    let (_drive, file) = drive_with_a_file(root.path());
    let target = root.path().join("target.txt");
    fs::write(&target, b"covered").expect("the target file is written");
    let _mounted = BindMount::new(&file, &target);

    let result = delete_tree(&target, &Cancellation::new(), None);

    assert_eq!(result, Err(mount_point_refusal(&target)));
    assert_eq!(fs::read(&file).expect("the drive's file is kept"), b"keep me");
}

/// parity: XFER-015
#[test]
fn a_folder_reached_through_a_link_finds_the_mount_inside_it() {
    if !in_private_mount_namespace(concat!(
        module_path!(),
        "::a_folder_reached_through_a_link_finds_the_mount_inside_it"
    )) {
        return;
    }
    let root = temporary_folder();
    let (drive, file) = drive_with_a_file(root.path());
    let real = root.path().join("real");
    fs::create_dir_all(real.join("selected/mnt")).expect("the folders are created");
    std::os::unix::fs::symlink(&real, root.path().join("Music")).expect("the link is created");
    let _mounted = BindMount::new(&drive, &real.join("selected/mnt"));
    let selected = root.path().join("Music/selected");

    let result = delete_tree(&selected, &Cancellation::new(), None);

    let message = result.expect_err("the deletion is refused").to_string();
    assert!(
        message.contains(&selected.join("mnt").display().to_string()),
        "{message}"
    );
    assert_eq!(fs::read(&file).expect("the drive's file is kept"), b"keep me");
}

/// parity: XFER-015
#[test]
fn a_folder_without_mounts_is_still_deleted() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    fs::create_dir_all(selected.join("sub")).expect("the folders are created");
    fs::write(selected.join("sub/a.txt"), b"delete me").expect("the file is written");

    let result = delete_tree(&selected, &Cancellation::new(), None);

    assert_eq!(result, Ok(()));
    assert!(!selected.exists());
}

#[test]
fn the_mount_table_check_finds_the_item_or_the_first_mount_inside_it() {
    let mounts: Vec<PathBuf> = [
        "/",
        "/home",
        "/home/u/Data",
        "/home/u/Data/b",
        "/home/u/Data/a",
        "/home/u/Database",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();

    let itself = first_mount_in(Path::new("/home/u/Data"), &mounts);
    let inside = first_mount_in(Path::new("/home/u"), &mounts);
    let sibling_prefix = first_mount_in(Path::new("/home/u/Data/c"), &mounts);

    assert_eq!(itself, Some(&PathBuf::from("/home/u/Data")));
    assert_eq!(inside, Some(&PathBuf::from("/home/u/Data")));
    assert_eq!(sibling_prefix, None);
}

#[test]
fn mount_ids_decide_when_both_are_known() {
    let disk = Mount {
        id: Some(30),
        device: (8, 1),
    };
    let bind_on_same_disk = Mount {
        id: Some(31),
        device: (8, 1),
    };
    let subvolume_on_same_mount = Mount {
        id: Some(30),
        device: (0, 45),
    };
    let without_id = Mount {
        id: None,
        device: (8, 1),
    };

    assert!(!disk.is_same_as(bind_on_same_disk));
    assert!(disk.is_same_as(subvolume_on_same_mount));
    assert!(disk.is_same_as(without_id));
    assert!(!without_id.is_same_as(Mount {
        id: None,
        device: (8, 2)
    }));
}
