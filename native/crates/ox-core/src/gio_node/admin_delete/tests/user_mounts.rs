// SPDX-License-Identifier: AGPL-3.0-only
//! Drives an ordinary user can mount: user entries in `/etc/fstab`, mounts
//! libmount records as a user's, and removable or hot-pluggable devices,
//! read from the [`fake_system`] in each test's folder.

use super::*;

/// A drive in a folder of its own in `root`, holding a [`closed_tree`],
/// mounted from `source` with the device number `number`. Returns the
/// drive's folder, the selected folder on it, its files and the mount
/// table with `/` on the internal disk.
fn drive_in(
    root: &Path,
    name: &str,
    source: &[u8],
    number: (u32, u32),
) -> (PathBuf, PathBuf, Vec<PathBuf>, Vec<Mount>) {
    let drive = folder(&root.join(name), 0o755);
    let (selected, files) = closed_tree(&drive);
    let mounts = vec![
        mount(Path::new("/"), "ext4"),
        Mount {
            id: 61,
            device: number,
            point: drive.clone(),
            filesystem: "ext4".into(),
            source: source.to_vec(),
            options: "rw".into(),
        },
    ];
    (drive, selected, files, mounts)
}

/// Writes `text` to the file `path` in the fake system of `rules`.
fn write_system_file(rules: &Rules, path: &str, text: &str) {
    let system = rules.seams.system.as_ref().expect("a fake system");
    let path = system.join(path);
    fs::create_dir_all(path.parent().expect("a folder")).expect("create the folder");
    fs::write(path, text).expect("write the file");
}

/// `path` as `/etc/fstab` and the mount table write it, `\040` for a space.
fn escaped(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").replace(' ', "\\040")
}

/// parity: XFER-015, OPS-039
#[test]
fn a_drive_fstab_lets_users_mount_is_refused() {
    for option in ["user", "users", "owner", "group", "x-udisks-auth"] {
        let (_folder, root) = workspace();
        let (drive, selected, files, mounts) = drive_in(&root, "drive", b"/dev/sdb1", (8, 17));
        let rules = rules(&root, mounts);
        write_system_file(
            &rules,
            "etc/fstab",
            &format!(
                "# /etc/fstab: static file system information.\n\
                 \x20 # <file system> <mount point> <type> <options> <dump> <pass>\n\
                 UUID=3a9c1f0e-7d2b-4c55-9e61-2f8d4b0a6c13 / ext4 errors=remount-ro 0 1\n\
                 \n\
                 /dev/sdb1\t{}\text4\tnoauto,nosuid,{option}\t0\t0\n",
                drive.display()
            ),
        );

        let result = delete(&selected, &rules);

        assert_eq!(result, Err(users_can_mount(&drive)), "{option}");
        assert_kept(&files);
    }
}

/// The entry is found by its mount point alone, written with `\040` for
/// a space, while its source names a drive that is not plugged in.
///
/// parity: XFER-015, OPS-039
#[test]
fn an_fstab_mount_point_with_an_escaped_space_is_matched() {
    let (_folder, root) = workspace();
    let (drive, selected, files, mounts) = drive_in(&root, "Backup drive", b"/dev/sdb1", (8, 17));
    let rules = rules(&root, mounts);
    write_system_file(
        &rules,
        "etc/fstab",
        &format!("LABEL=Elsewhere {} ext4 user,noauto 0 0\n", escaped(&drive)),
    );

    let result = delete(&selected, &rules);

    assert_eq!(result, Err(users_can_mount(&drive)));
    assert_kept(&files);
}

/// The entry is found by its source alone, a tag that `/dev/disk` resolves
/// to the drive's device, while its mount point is elsewhere.
///
/// parity: XFER-015, OPS-039
#[test]
fn an_fstab_source_given_by_uuid_or_label_is_matched() {
    let sources = [
        ("UUID=4F2A-91C3", "by-uuid/4f2a-91c3"),
        (
            "UUID=0b7e5d2c-1f3a-4e8b-a6d9-5c2e7f1b3a40",
            "by-uuid/0b7e5d2c-1f3a-4e8b-a6d9-5c2e7f1b3a40",
        ),
        ("LABEL=\"Backup\\040Disk\"", "by-label/Backup\\x20Disk"),
        ("PARTUUID=6c1e2f9a-03", "by-partuuid/6c1e2f9a-03"),
        ("PARTLABEL=photos", "by-partlabel/photos"),
    ];
    for (source, link) in sources {
        let (_folder, root) = workspace();
        let (drive, selected, files, mounts) = drive_in(&root, "drive", b"/dev/sdb1", (8, 17));
        let rules = rules(&root, mounts);
        write_system_file(
            &rules,
            "etc/fstab",
            &format!("{source} /mnt/elsewhere ext4 users,noauto 0 0\n"),
        );
        let link = rules
            .seams
            .system
            .as_ref()
            .expect("a fake system")
            .join("dev/disk")
            .join(link);
        fs::create_dir_all(link.parent().expect("a folder")).expect("create the folder");
        symlink("../../sdb1", &link).expect("link the device");

        let result = delete(&selected, &rules);

        assert_eq!(result, Err(users_can_mount(&drive)), "{source}");
        assert_kept(&files);
    }
}

/// `mount` records `user=` in libmount's `utab` when a user mounted the
/// drive; older libmount versions leave out the mount ID.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_drive_libmount_records_as_mounted_by_a_user_is_refused() {
    for id in ["ID=61 ", ""] {
        let (_folder, root) = workspace();
        let (drive, selected, files, mounts) = drive_in(&root, "my drive", b"/dev/sdb1", (8, 17));
        let rules = rules(&root, mounts);
        write_system_file(
            &rules,
            "run/mount/utab",
            &format!(
                "ID=40 SRC=/dev/sda3 TARGET=/boot/efi ROOT=/ OPTS=x-systemd.automount\n\
                 {id}SRC=/dev/sdb1 TARGET={} ROOT=/ OPTS=user=demo\n",
                escaped(&drive)
            ),
        );

        let result = delete(&selected, &rules);

        assert_eq!(result, Err(users_can_mount(&drive)), "{id:?}");
        assert_kept(&files);
    }
}

/// A drive that can be unplugged, wherever and however it is mounted, and
/// a disk drive whose device cannot be found.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_removable_or_hot_pluggable_drive_is_refused() {
    let usb = "pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host6/target6:0:0/6:0:0:0/block/sdc/sdc1";
    let flagged = "pci0000:00/0000:00:17.0/ata3/host2/target2:0:0/2:0:0:0/block/sdd/sdd1";
    let sd_card = "platform/fe320000.mmc/mmc_host/mmc1/mmc1:aaaa/block/mmcblk1/mmcblk1p1";
    let cases = [
        ("USB", Some((usb, false)), (8, 33), b"/dev/sdc1".as_slice()),
        (
            "removable flag",
            Some((flagged, true)),
            (8, 49),
            b"/dev/sdd1".as_slice(),
        ),
        (
            "SD card",
            Some((sd_card, false)),
            (179, 1),
            b"/dev/mmcblk1p1".as_slice(),
        ),
        ("no device", None, (8, 99), b"/dev/sdz9".as_slice()),
    ];
    for (case, device, number, source) in cases {
        let (_folder, root) = workspace();
        let (drive, selected, files, mounts) = drive_in(&root, "drive", source, number);
        let rules = rules(&root, mounts);
        if let Some((path, removable)) = device {
            block_device(
                rules.seams.system.as_ref().expect("a fake system"),
                number,
                path,
                removable,
            );
        }

        let result = delete(&selected, &rules);

        assert_eq!(result, Err(users_can_mount(&drive)), "{case}");
        assert_kept(&files);
    }
}

/// A user `fstab` entry that is not mounted still matters where a user
/// could mount a drive while the deletion runs: inside the folder, or on
/// its path.
///
/// parity: XFER-015, OPS-039
#[test]
fn an_unmounted_user_entry_inside_or_above_the_folder_is_refused() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(&root);
    let inside = selected.join("sub/stick");
    let rules = on_ext4(&root);
    write_system_file(
        &rules,
        "etc/fstab",
        &format!("LABEL=STICK {} vfat user,noauto 0 0\n", inside.display()),
    );

    assert_eq!(delete(&selected, &rules), Err(users_can_mount(&inside)));
    assert_kept(&files);

    let above = folder(&root.join("mnt"), 0o755);
    let (selected, files) = closed_tree(&above);
    let rules = on_ext4(&root);
    write_system_file(
        &rules,
        "etc/fstab",
        &format!("LABEL=STICK {} ext4 users,noauto 0 0\n", above.display()),
    );

    assert_eq!(delete(&selected, &rules), Err(users_can_mount(&above)));
    assert_kept(&files);
}

/// What stays allowed: internal disks the administrator mounts through
/// `fstab`, memory filesystems, and the root filesystem even on an SD
/// card, next to user entries and records for other drives.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_on_an_internal_disk_or_in_memory_is_still_deleted() {
    let cases: [(&str, &[u8], (u32, u32)); 2] =
        [("xfs", b"/dev/sdb1", (8, 17)), ("tmpfs", b"tmpfs", (0, 45))];
    for (filesystem, source, number) in cases {
        let (_folder, root) = workspace();
        let (drive, selected, _, mut mounts) = drive_in(&root, "data", source, number);
        mounts[0].device = (179, 2);
        mounts[1].filesystem = filesystem.into();
        let rules = rules(&root, mounts);
        let system = rules.seams.system.as_ref().expect("a fake system");
        let sd_card = "platform/fe340000.mmc/mmc_host/mmc0/mmc0:59b4/block/mmcblk0/mmcblk0p2";
        block_device(system, (179, 2), sd_card, false);
        let by_uuid = system.join("dev/disk/by-uuid");
        fs::create_dir_all(&by_uuid).expect("create the folder");
        symlink("../../sdb1", by_uuid.join("9d4e7a21-5b3c-4f08-8e12-c6a0d3f7b954")).expect("link the device");
        write_system_file(
            &rules,
            "etc/fstab",
            &format!(
                "/dev/mmcblk0p2 / ext4 defaults 0 1\n\
                 UUID=9d4e7a21-5b3c-4f08-8e12-c6a0d3f7b954 {} {filesystem} defaults,nouser 0 2\n\
                 /dev/sr0 /media/cdrom0 udf,iso9660 user,noauto 0 0\n\
                 UUID=1C2B-7F3E /mnt/stick vfat users,noauto 0 0\n",
                drive.display()
            ),
        );
        write_system_file(
            &rules,
            "run/mount/utab",
            &format!(
                "ID=61 SRC={} TARGET={} ROOT=/ OPTS=x-gvfs-show\n\
                 ID=88 SRC=/dev/sde1 TARGET=/mnt/stick ROOT=/ OPTS=user=demo\n",
                String::from_utf8_lossy(source),
                escaped(&drive)
            ),
        );

        let result = delete(&selected, &rules);

        assert_eq!(result, Ok(()), "{filesystem}");
        assert!(fs::symlink_metadata(&selected).is_err(), "{filesystem}");
    }
}
