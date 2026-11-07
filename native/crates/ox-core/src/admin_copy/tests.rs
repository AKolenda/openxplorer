// SPDX-License-Identifier: AGPL-3.0-only
//! The checks run here on ordinary temporary folders. An ordinary user
//! cannot make a root-owned folder or mount a drive, so the tests trust
//! their own account in root's place, judge folders from their own
//! temporary folder down (the machine's `/tmp` above it is world-writable)
//! and supply the mount table, through [`Seams`]. `/etc/fstab`, libmount's
//! `utab`, `/sys` and `/dev` are read from a stand-in folder with two
//! internal SATA disks ([`fake_system`]), as #194's tests did.

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::test_support::temporary_folder;

/// A temporary folder for one test, and its path with links resolved.
fn workspace() -> (TempDir, PathBuf) {
    let folder = temporary_folder();
    let path = fs::canonicalize(folder.path()).expect("resolve the temporary folder");
    (folder, path)
}

/// The account the tests run as, trusted in root's place.
fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

/// A mount of `filesystem` at `mount_point`, from the internal disk's
/// partition `sda2`.
fn mount(mount_point: &Path, filesystem: &str) -> Mount {
    Mount {
        id: 22,
        device: (8, 2),
        point: mount_point.to_owned(),
        filesystem: filesystem.into(),
        source: b"/dev/sda2".to_vec(),
        options: "rw".into(),
    }
}

/// Rules that trust this account in root's place, judge the folders from
/// `root` down, read `mounts` as the mount table and read the rest of the
/// system from a [`fake_system`] in `root`.
fn rules(root: &Path, mounts: Vec<Mount>) -> Rules {
    Rules {
        seams: Seams {
            owner: Some(me()),
            mounts: Some(mounts),
            judged_from: Some(root.to_owned()),
            system: Some(fake_system(root)),
        },
    }
}

/// A stand-in in `root` for the folders that hold `/etc/fstab`, libmount's
/// `utab`, `/sys` and `/dev`, with no `fstab` or `utab` and two internal
/// SATA disks: the partitions `sda2` (8:2) and `sdb1` (8:17).
fn fake_system(root: &Path) -> PathBuf {
    let system = root.join("fake system");
    let ata = "pci0000:00/0000:00:17.0/ata";
    block_device(
        &system,
        (8, 2),
        &format!("{ata}1/host0/target0:0:0/0:0:0:0/block/sda/sda2"),
        false,
    );
    block_device(
        &system,
        (8, 17),
        &format!("{ata}2/host1/target1:0:0/1:0:0:0/block/sdb/sdb1"),
        false,
    );
    system
}

/// Adds to the [`fake_system`] `system` the partition at `device_path`
/// under `/sys/devices`, numbered `number`, on a disk whose `removable`
/// flag is `removable`, with its node in `/dev`.
fn block_device(system: &Path, number: (u32, u32), device_path: &str, removable: bool) {
    let sys = system.join("sys");
    let partition = sys.join("devices").join(device_path);
    fs::create_dir_all(&partition).expect("create the device");
    fs::write(partition.join("partition"), "1\n").expect("write the partition number");
    let disk = partition.parent().expect("a disk above the partition");
    fs::write(disk.join("removable"), if removable { "1\n" } else { "0\n" }).expect("write the flag");
    let name = partition.file_name().expect("a device name");
    let target = format!("../../devices/{device_path}");
    let (major, minor) = number;
    for link in [
        sys.join(format!("dev/block/{major}:{minor}")),
        sys.join("class/block").join(name),
    ] {
        fs::create_dir_all(link.parent().expect("a folder")).expect("create the folder");
        let _ = fs::remove_file(&link);
        symlink(&target, link).expect("link the device");
    }
    fs::create_dir_all(system.join("dev")).expect("create /dev");
    fs::write(system.join("dev").join(name), "").expect("create the device node");
}

/// [`rules`] with the whole machine on one ext4 drive.
fn on_ext4(root: &Path) -> Rules {
    rules(root, vec![mount(Path::new("/"), "ext4")])
}

/// Creates the folder `path` with the permission bits `mode`.
fn folder(path: &Path, mode: u32) -> PathBuf {
    fs::create_dir_all(path).expect("create the folder");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set the mode");
    path.to_owned()
}

/// The `file:` URI of `path`.
fn uri(path: &Path) -> String {
    glib::filename_to_uri(path, None).expect("a file URI").to_string()
}

/// What the check answers for the folder at `path` under `rules`.
fn check(path: &Path, rules: &Rules) -> Result<String, TransferError> {
    elevated_destination_with(&uri(path), rules)
}

/// `path` under `admin://`.
fn admin(path: &Path) -> String {
    uri(path).replacen("file://", "admin://", 1)
}

/// A folder only its owner can change, with a space and a non-ASCII
/// letter in its name, is written as administrator at the same place
/// under `admin://`.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_folder_only_root_can_change_is_written_under_admin() {
    let (_folder, root) = workspace();
    let program_files = folder(&root.join("Program Files/Café"), 0o755);

    assert_eq!(check(&program_files, &on_ext4(&root)), Ok(admin(&program_files)));
}

/// Group write, other write and the sticky bit with other write each let
/// another account swap a folder on the path for a link, so the
/// destination is refused, whether it is the destination or a folder
/// above it.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_folder_others_can_change_on_the_path_is_refused() {
    for mode in [0o775, 0o757, 0o1777] {
        let (_folder, root) = workspace();
        let shared = folder(&root.join("shared"), 0o755);
        let inside = folder(&shared.join("inside"), 0o755);
        fs::set_permissions(&shared, fs::Permissions::from_mode(mode)).expect("open the folder");

        assert_eq!(
            check(&inside, &on_ext4(&root)),
            Err(others_can_change(&inside)),
            "{mode:o}"
        );
        assert_eq!(
            check(&shared, &on_ext4(&root)),
            Err(others_can_change(&shared)),
            "{mode:o}"
        );
    }
}

/// A folder another account owns is refused, as only root may own the
/// folders root writes through.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_folder_another_account_owns_is_refused() {
    let (_folder, root) = workspace();
    let theirs = folder(&root.join("theirs"), 0o755);
    let mut rules = on_ext4(&root);
    rules.seams.owner = Some(me().wrapping_add(1));

    assert_eq!(check(&theirs, &rules), Err(others_can_change(&theirs)));
}

/// A link in a closed folder is followed, and the destination is named by
/// where it leads; a link in a folder others can change is not followed.
///
/// parity: OPS-047, XFER-015
#[test]
fn links_are_followed_only_from_closed_folders() {
    let (_folder, root) = workspace();
    let real = folder(&root.join("var/home/Apps"), 0o755);
    symlink("var/home", root.join("home")).expect("link home");
    let through_link = root.join("home/Apps");

    assert_eq!(check(&through_link, &on_ext4(&root)), Ok(admin(&real)));

    let open = folder(&root.join("open"), 0o777);
    symlink(&real, open.join("Apps")).expect("plant a link");
    let planted = open.join("Apps");
    assert_eq!(check(&planted, &on_ext4(&root)), Err(others_can_change(&planted)));
}

/// A filesystem whose owners and modes a user could have written, a
/// user's drive under `/media`, and a disk image are refused.
///
/// parity: OPS-047, XFER-015
#[test]
fn drives_whose_owners_cannot_be_believed_are_refused() {
    let (_folder, root) = workspace();
    let destination = folder(&root.join("drive/Apps"), 0o755);
    let drive = root.join("drive");
    let untrusted = |filesystem: &str, source: &[u8]| {
        let mut on_drive = mount(&drive, filesystem);
        on_drive.source = source.to_vec();
        rules(&root, vec![mount(Path::new("/"), "ext4"), on_drive])
    };

    for (filesystem, source) in [
        ("fuse.sshfs", b"server:/".as_slice()),
        ("vfat", b"/dev/sda2".as_slice()),
        ("ext4", b"/dev/loop3".as_slice()),
    ] {
        assert_eq!(
            check(&destination, &untrusted(filesystem, source)),
            Err(others_can_change(&destination)),
            "{filesystem}"
        );
    }
    let mut zfs = mount(&drive, "zfs");
    zfs.options = "rw,nfs4acl".into();
    let zfs_rules = rules(&root, vec![mount(Path::new("/"), "ext4"), zfs]);
    assert_eq!(
        check(&destination, &zfs_rules),
        Err(others_can_change(&destination))
    );
}

/// A drive `/etc/fstab` lets users mount, or a mount point of such an
/// entry on the destination's path, is refused.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_drive_users_may_mount_is_refused() {
    let (_folder, root) = workspace();
    let drive = folder(&root.join("drive"), 0o755);
    let destination = folder(&drive.join("Apps"), 0o755);
    let mut on_drive = mount(&drive, "ext4");
    on_drive.device = (8, 17);
    on_drive.source = b"/dev/sdb1".to_vec();
    let rules = rules(&root, vec![mount(Path::new("/"), "ext4"), on_drive]);
    let fstab = fake_system(&root).join("etc/fstab");
    fs::create_dir_all(fstab.parent().expect("etc")).expect("create etc");
    fs::write(
        &fstab,
        format!("/dev/sdb1\t{}\text4\tnoauto,user\t0\t0\n", drive.display()),
    )
    .expect("write fstab");
    let rules = Rules {
        seams: Seams {
            system: Some(fake_system(&root)),
            ..rules.seams
        },
    };

    assert_eq!(check(&destination, &rules), Err(users_can_mount(&destination)));
}

/// A removable drive mounted inside the destination, or a user `fstab`
/// mount point inside it, is refused; the system's own mounts inside it
/// are not.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_users_drive_inside_the_destination_is_refused() {
    let (_folder, root) = workspace();
    let destination = folder(&root.join("Apps"), 0o755);
    let inner = folder(&destination.join("cache"), 0o755);
    let system_mount = rules(&root, vec![mount(Path::new("/"), "ext4"), mount(&inner, "tmpfs")]);
    assert_eq!(check(&destination, &system_mount), Ok(admin(&destination)));

    let users = rules(
        &root,
        vec![mount(Path::new("/"), "ext4"), mount(&inner, "fuse.sshfs")],
    );
    assert_eq!(check(&destination, &users), Err(users_can_mount(&destination)));

    let rules = on_ext4(&root);
    let fstab = fake_system(&root).join("etc/fstab");
    fs::create_dir_all(fstab.parent().expect("etc")).expect("create etc");
    fs::write(
        &fstab,
        format!("/dev/sdc1 {} vfat user,noauto 0 0\n", inner.display()),
    )
    .expect("write fstab");
    let rules = Rules {
        seams: Seams {
            system: Some(fake_system(&root)),
            ..rules.seams
        },
    };
    assert_eq!(check(&destination, &rules), Err(users_can_mount(&destination)));
}

/// A share, a phone or a path that is not a folder is never written as
/// administrator, and an `admin://` destination stays as it is.
///
/// parity: OPS-047, XFER-015
#[test]
fn only_folders_on_this_computer_are_written_as_administrator() {
    let (_folder, root) = workspace();
    let rules = on_ext4(&root);
    assert!(matches!(
        elevated_destination_with("smb://nas.example/share", &rules),
        Err(TransferError::NotSupported(_))
    ));
    assert!(matches!(
        elevated_destination_with("file://nas.example/share", &rules),
        Err(TransferError::NotSupported(_))
    ));
    let file = root.join("plan.txt");
    fs::write(&file, "plan").expect("write the file");
    assert_eq!(check(&file, &rules), Err(not_a_folder(&file)));
}

/// An `admin://` destination is checked by its path like any other: one
/// only root can change keeps its place, one others can change is
/// refused.
///
/// parity: OPS-047, XFER-015
#[test]
fn an_admin_destination_is_checked_by_its_path() {
    let (_folder, root) = workspace();
    let closed = folder(&root.join("Apps"), 0o755);
    let open = folder(&root.join("Shared"), 0o775);
    let rules = on_ext4(&root);

    assert_eq!(
        elevated_destination_with(&admin(&closed), &rules),
        Ok(admin(&closed))
    );
    assert_eq!(
        elevated_destination_with(&admin(&open), &rules),
        Err(others_can_change(&open))
    );
}

/// The existing folder a Replace run as administrator merges into is
/// refused when another account could change it: group-writable,
/// other-writable or sticky and world-writable, or anything on its path.
///
/// parity: OPS-047, XFER-015
#[test]
fn an_existing_folder_to_merge_into_is_checked_like_the_destination() {
    let (_folder, root) = workspace();
    let destination = folder(&root.join("var"), 0o755);
    let closed = folder(&destination.join("lib"), 0o755);
    let rules = on_ext4(&root);
    assert_eq!(check_elevated_folder_with(&admin(&closed), &rules), Ok(()));
    assert_eq!(check_elevated_folder_with(&uri(&closed), &rules), Ok(()));

    for mode in [0o775, 0o757, 0o1777] {
        let existing = folder(&destination.join(format!("tmp {mode:o}")), mode);
        let inside = folder(&existing.join("x"), 0o755);
        assert_eq!(
            check_elevated_folder_with(&admin(&existing), &rules),
            Err(others_can_change(&existing)),
            "{mode:o}"
        );
        assert_eq!(
            check_elevated_folder_with(&admin(&inside), &rules),
            Err(others_can_change(&inside)),
            "below {mode:o}"
        );
    }
}

/// Without a mount table to believe, as in a Flatpak, nothing is written
/// as administrator.
///
/// parity: OPS-047, XFER-015
#[test]
fn without_a_mount_table_nothing_is_written() {
    let (_folder, root) = workspace();
    let destination = folder(&root.join("Apps"), 0o755);
    let rules = on_ext4(&root);
    let without = Rules {
        seams: Seams {
            mounts: Some(Vec::new()),
            ..rules.seams
        },
    };

    assert_eq!(
        check(&destination, &without),
        Err(others_can_change(&destination))
    );
}

/// An item is deleted as administrator under `admin://` only when the
/// folder that holds it passes: a file in a closed folder does, one in a
/// folder others can change (sticky or group-writable) does not, nor does
/// a filesystem root.
///
/// parity: OPS-047, XFER-015
#[test]
fn an_item_is_deleted_as_administrator_only_from_a_closed_folder() {
    let (_folder, root) = workspace();
    let closed = folder(&root.join("usr/local"), 0o755);
    let file = closed.join("tool.cfg");
    fs::write(&file, "settings").expect("write the file");
    let rules = on_ext4(&root);
    assert_eq!(elevated_item_with(&uri(&file), &rules), Ok(admin(&file)));
    assert_eq!(elevated_item_with(&admin(&file), &rules), Ok(admin(&file)));

    for mode in [0o1777, 0o775] {
        let open = folder(&root.join(format!("shared {mode:o}")), mode);
        let theirs = open.join("notes.txt");
        fs::write(&theirs, "notes").expect("write the file");
        assert_eq!(
            elevated_item_with(&uri(&theirs), &rules),
            Err(others_can_change(&open)),
            "{mode:o}"
        );
    }
    assert!(elevated_item_with("file:///", &rules).is_err());
}
