// SPDX-License-Identifier: AGPL-3.0-only
//! The checks run here on `file://` trees, which exercise the GIO calls
//! `gvfsd-admin` receives. An ordinary user cannot make a root-owned
//! folder, mount a drive or see `admin://`, so the tests trust their own
//! account in root's place, judge folders from their own temporary folder
//! down (the machine's `/tmp` above it is world-writable) and supply the
//! mount table, through [`Seams`]. `/etc/fstab`, libmount's `utab`, `/sys`
//! and `/dev` are read from a stand-in folder with two internal SATA disks
//! ([`fake_system`]).

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::test_support::temporary_folder;

/// A temporary folder for one test, and its path with links resolved, as
/// the walk resolves it.
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
            device: None,
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
/// flag is `removable`, as the kernel lays it out, with its node in `/dev`.
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
        // A second fake system in the same folder lays out the same disks.
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

/// Deletes `path` under `rules` as a folder under `admin://` is deleted.
fn delete(path: &Path, rules: &Rules) -> Result<(), TransferError> {
    delete_tree_with(
        &gio::File::for_path(path),
        None,
        &Cancellation::new(),
        None,
        rules,
    )
}

/// Sets the permission bits of `path`.
fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set the mode");
}

/// Creates the folder `path` with the permission bits `mode`.
fn folder(path: &Path, mode: u32) -> PathBuf {
    fs::create_dir_all(path).expect("create the folder");
    set_mode(path, mode);
    path.to_owned()
}

/// Writes a small file at `path`.
fn file(path: &Path) -> PathBuf {
    fs::write(path, b"keep me").expect("write the file");
    path.to_owned()
}

/// The selected folder under `root`, with a file, and a folder holding a
/// file and a deeper folder with a file, all closed to others.
fn closed_tree(root: &Path) -> (PathBuf, Vec<PathBuf>) {
    let selected = folder(&root.join("selected"), 0o755);
    let sub = folder(&selected.join("sub"), 0o700);
    let deeper = folder(&sub.join("deeper"), 0o755);
    let files = vec![
        file(&selected.join("top.txt")),
        file(&selected.join(".hidden")),
        file(&sub.join("inside.txt")),
        file(&deeper.join("deepest.txt")),
    ];
    (selected, files)
}

/// Asserts that every one of `paths` is still there.
fn assert_kept(paths: &[PathBuf]) {
    for path in paths {
        assert!(
            fs::symlink_metadata(path).is_ok(),
            "{} was deleted",
            path.display()
        );
    }
}

/// parity: XFER-015, OPS-039
#[test]
fn a_folder_only_root_can_change_is_deleted_with_everything_in_it() {
    let (_folder, root) = workspace();
    let (selected, _) = closed_tree(root.as_path());
    let outside = folder(&root.as_path().join("outside"), 0o755);
    let kept = file(&outside.join("kept.txt"));
    symlink(&outside, selected.join("link to outside")).expect("create the link");

    let result = delete(&selected, &on_ext4(root.as_path()));

    assert_eq!(result, Ok(()));
    assert!(fs::symlink_metadata(&selected).is_err());
    assert_kept(&[kept]);
}

/// parity: XFER-015, OPS-039
#[test]
fn a_folder_inside_that_others_can_write_to_is_refused_before_anything_is_deleted() {
    let (_folder, root) = workspace();
    let (selected, mut files) = closed_tree(root.as_path());
    let open = folder(&selected.join("sub/deeper/open"), 0o757);
    files.push(file(&open.join("in the open folder.txt")));

    let result = delete(&selected, &on_ext4(root.as_path()));

    assert_eq!(result, Err(others_can_change(&open)));
    assert_kept(&files);
}

/// The group-write bit is enough, and the sticky bit does not make a
/// world-writable folder safe: anybody can still create a name in it.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_group_writable_folder_and_a_sticky_world_writable_one_are_refused() {
    for mode in [0o770, 0o775, 0o2775, 0o1777, 0o1733] {
        let (_folder, root) = workspace();
        let (selected, mut files) = closed_tree(root.as_path());
        let shared = folder(&selected.join("shared"), mode);
        files.push(file(&shared.join("shared.txt")));

        let result = delete(&selected, &on_ext4(root.as_path()));

        assert_eq!(result, Err(others_can_change(&shared)), "mode {mode:o}");
        assert_kept(&files);
    }
}

/// parity: XFER-015, OPS-039
#[test]
fn the_selected_folder_itself_open_to_others_is_refused() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());
    set_mode(&selected, 0o777);

    let result = delete(&selected, &on_ext4(root.as_path()));

    assert_eq!(result, Err(others_can_change(&selected)));
    assert_kept(&files);
}

/// parity: XFER-015, OPS-039
#[test]
fn a_folder_above_that_others_can_change_is_refused() {
    let (_folder, root) = workspace();
    let above = folder(&root.as_path().join("above"), 0o777);
    let (selected, files) = closed_tree(&above);

    let result = delete(&selected, &on_ext4(root.as_path()));

    assert_eq!(result, Err(others_can_change(&above)));
    assert_kept(&files);
}

/// The owner half of the rule: a closed folder another account owns could
/// be opened up by that account at any time.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_another_account_owns_is_refused() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());
    let mut rules = on_ext4(root.as_path());
    rules.seams.owner = Some(me().wrapping_add(1));

    let result = delete(&selected, &rules);

    assert_eq!(result, Err(others_can_change(root.as_path())));
    assert_kept(&files);
}

/// The race the checks exist for. Part way through the walk, somebody who
/// can write to the selected folder swaps one of its folders for a link to
/// a folder outside it. The walk by path would then delete through the
/// link (see the next test); a folder that somebody else can change is
/// refused before the walk starts, so the swap never happens.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_swapped_for_a_link_part_way_through_cannot_reach_outside() {
    let (_folder, root) = workspace();
    let (selected, inner, bystander) = swap_fixture(root.as_path());
    set_mode(&selected, 0o777);
    let swapped = Arc::new(AtomicBool::new(false));
    let guard = swapping_guard(&inner, &bystander, &swapped);

    let result = delete_tree_with(
        &gio::File::for_path(&selected),
        None,
        &Cancellation::new(),
        Some(&guard),
        &on_ext4(root.as_path()),
    );

    assert_eq!(result, Err(others_can_change(&selected)));
    assert!(!swapped.load(Ordering::SeqCst), "the walk started");
    assert_kept(&[inner.join("c.txt"), bystander.join("c.txt")]);
}

/// The comparison for the test above: the swap really does redirect the
/// walk by path, which `remote_delete` accepts for shares and phones.
///
/// parity: XFER-015
#[test]
fn the_walk_by_path_alone_follows_a_folder_swapped_for_a_link() {
    let (_folder, root) = workspace();
    let (selected, inner, bystander) = swap_fixture(root.as_path());
    let swapped = Arc::new(AtomicBool::new(false));
    let guard = swapping_guard(&inner, &bystander, &swapped);

    let result = remote_delete::delete_tree(
        &gio::File::for_path(&selected),
        None,
        &Cancellation::new(),
        Some(&guard),
    );

    assert_eq!(result, Ok(()));
    assert!(swapped.load(Ordering::SeqCst));
    assert!(
        !bystander.join("c.txt").exists(),
        "the walk did not follow the link"
    );
}

/// `selected/inner/c.txt` and, outside, `bystander/c.txt`.
fn swap_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let selected = folder(&root.join("selected"), 0o755);
    let inner = folder(&selected.join("inner"), 0o755);
    file(&inner.join("c.txt"));
    let bystander = folder(&root.join("bystander"), 0o755);
    file(&bystander.join("c.txt"));
    (selected, inner, bystander)
}

/// A write guard that, when asked about `inner/c.txt`, moves `inner` away
/// and puts a link to `bystander` in its place.
fn swapping_guard(
    inner: &Path,
    bystander: &Path,
    swapped: &Arc<AtomicBool>,
) -> impl Fn(&str) -> Result<(), TransferError> + Send + Sync {
    let inner = inner.to_owned();
    let bystander = bystander.to_owned();
    let swapped = Arc::clone(swapped);
    let target = gio::File::for_path(inner.join("c.txt")).uri().to_string();
    move |uri: &str| {
        if uri == target && !swapped.swap(true, Ordering::SeqCst) {
            fs::rename(&inner, bystander.with_file_name("inner, moved away")).expect("move the folder away");
            symlink(&bystander, &inner).expect("put the link in its place");
        }
        Ok(())
    }
}

/// The mount table names mount points with octal escapes and with links
/// resolved; the selected folder is reached here through a link.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_mount_inside_the_folder_is_refused_before_anything_is_deleted() {
    let (_folder, root) = workspace();
    let real = folder(&root.as_path().join("real"), 0o755);
    symlink("real", root.as_path().join("link")).expect("create the link");
    let (_, files) = closed_tree(&real);
    let mount_point = folder(&real.join("selected/sub/my drive"), 0o755);
    let escaped = mount_point.to_str().expect("a UTF-8 path").replace(' ', "\\040");
    let table = format!(
        "22 1 8:2 / / rw shared:1 - ext4 /dev/sda2 rw\n\
         61 22 8:17 / {escaped} rw shared:2 - ext4 /dev/sdb1 rw\n"
    );
    let rules = rules(root.as_path(), mounts::parse_mounts(table.as_bytes()));
    let selected = root.as_path().join("link/selected");

    let result = delete(&selected, &rules);

    assert_eq!(result, Err(mount_inside_refusal(&selected.join("sub/my drive"))));
    assert_kept(&files);
}

/// parity: XFER-015, OPS-039
#[test]
fn a_mount_point_is_refused_as_the_folder() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());
    let rules = rules(
        root.as_path(),
        vec![mount(Path::new("/"), "ext4"), mount(&selected, "xfs")],
    );

    let result = delete(&selected, &rules);

    assert_eq!(result, Err(mount_point_refusal(&selected)));
    assert_kept(&files);
}

/// A drive mounted inside the folder after the checks: the walk asks
/// about each folder's device again before entering it.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_drive_mounted_part_way_through_is_not_entered() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());
    let sub = selected.join("sub");
    let walking = Arc::new(AtomicBool::new(false));
    let started = Arc::clone(&walking);
    // The guard is asked first about the selected folder, once the walk
    // that deletes has begun.
    let guard = move |_: &str| {
        started.store(true, Ordering::SeqCst);
        Ok(())
    };
    let mut rules = on_ext4(root.as_path());
    let mounted = sub.clone();
    let deleting = Arc::clone(&walking);
    rules.seams.device = Some(Box::new(move |path, device| {
        if path == mounted && deleting.load(Ordering::SeqCst) {
            device.wrapping_add(1)
        } else {
            device
        }
    }));

    let result = delete_tree_with(
        &gio::File::for_path(&selected),
        None,
        &Cancellation::new(),
        Some(&guard),
        &rules,
    );

    assert_eq!(result, Err(contained_mount_refusal(&sub)));
    assert_kept(&files[2..]);
}

/// A FUSE or network mount, or a drive without Unix owners, can claim any
/// owner it likes.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_on_a_filesystem_users_can_supply_is_refused() {
    for filesystem in ["fuse.sshfs", "fuse", "fuseblk", "nfs4", "cifs", "vfat", "9p"] {
        let (_folder, root) = workspace();
        let (selected, files) = closed_tree(root.as_path());
        let rules = rules(
            root.as_path(),
            vec![mount(Path::new("/"), "ext4"), mount(root.as_path(), filesystem)],
        );

        let result = delete(&selected, &rules);

        assert_eq!(result, Err(others_can_change(root.as_path())), "{filesystem}");
        assert_kept(&files);
    }
}

/// The kernel leaves a carriage return in a mount point unescaped: it is
/// part of the mount point, which must not be cut short at it.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_mount_point_with_a_carriage_return_in_its_name_is_still_found() {
    let (_folder, root) = workspace();
    let drive = folder(&root.join("disk\rcrafted"), 0o755);
    let (selected, files) = closed_tree(&drive);
    let table = format!(
        "22 1 8:2 / / rw shared:1 - ext4 /dev/sda2 rw\n\
         70 22 0:60 / {} rw - fuse.demo demo rw\n",
        drive.to_str().expect("a UTF-8 path")
    );
    let rules = rules(&root, mounts::parse_mounts(table.as_bytes()));

    let result = delete(&selected, &rules);

    assert_eq!(result, Err(others_can_change(&drive)));
    assert_kept(&files);
}

/// parity: XFER-015, OPS-039
#[test]
fn only_local_kernel_filesystems_mounted_by_the_administrator_are_trusted() {
    let on = |mount_point: &str, filesystem: &str, source: &[u8], options: &str| Mount {
        id: 22,
        device: (8, 2),
        point: mount_point.into(),
        filesystem: filesystem.into(),
        source: source.to_vec(),
        options: options.into(),
    };
    let trusted = [
        on("/", "ext4", b"/dev/nvme0n1p2", "rw"),
        on("/", "overlay", b"composefs", "ro"),
        on("/var", "btrfs", b"/dev/sda3", "rw,subvol=/var"),
        on("/srv", "xfs", b"/dev/sdb1", "rw"),
        on("/tank", "zfs", b"tank", "rw,xattr,posixacl"),
        on("/run", "tmpfs", b"tmpfs", "rw,mode=755"),
    ];
    let untrusted = [
        on("/run/media/demo/STICK", "ext4", b"/dev/sdc1", "rw"),
        on("/media/demo/STICK", "btrfs", b"/dev/sdc1", "rw"),
        on("/srv/image", "ext4", b"/dev/loop3", "rw"),
        on("/tank", "zfs", b"tank", "rw,xattr,nfs4acl"),
        on("/tank", "zfs", b"tank", "rw,xattr"),
        on("/mnt/share", "fuse.rclone", b"remote:", "rw"),
        on("/mnt/nas", "cifs", b"//nas/share", "rw"),
        on("/mnt/usb", "exfat", b"/dev/sdd1", "rw"),
    ];

    for mount in &trusted {
        assert!(is_trusted_filesystem(mount), "{mount:?}");
    }
    for mount in &untrusted {
        assert!(!is_trusted_filesystem(mount), "{mount:?}");
    }
}

/// Fedora Atomic's `/home` is a link to `var/home` in `/`: a link in a
/// folder only root can change is followed, relative or absolute, and
/// what it leads to is checked like any other path.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_link_on_the_path_in_a_folder_only_root_can_change_is_followed() {
    let (_folder, root) = workspace();
    let home = folder(&root.as_path().join("var/home"), 0o755);
    symlink("var/home", root.as_path().join("home")).expect("create the relative link");
    symlink(&home, root.as_path().join("absolute")).expect("create the absolute link");
    let rules = rules(
        root.as_path(),
        vec![
            mount(Path::new("/"), "ext4"),
            mount(&root.as_path().join("var"), "xfs"),
        ],
    );

    for through in ["home", "absolute"] {
        let (selected, _) = closed_tree(&home);

        let result = delete(&root.as_path().join(through).join("selected"), &rules);

        assert_eq!(result, Ok(()), "through {through}");
        assert!(fs::symlink_metadata(&selected).is_err(), "through {through}");
    }
}

/// parity: XFER-015, OPS-039
#[test]
fn a_link_on_the_path_in_a_folder_others_can_change_is_refused() {
    let (_folder, root) = workspace();
    let open = folder(&root.as_path().join("open"), 0o777);
    let home = folder(&root.as_path().join("var/home"), 0o755);
    symlink("../var/home", open.join("home")).expect("create the link");
    let (_, files) = closed_tree(&home);

    let result = delete(&open.join("home/selected"), &on_ext4(root.as_path()));

    assert_eq!(result, Err(others_can_change(&open)));
    assert_kept(&files);
}

/// A trusted link that leads into a folder others can change is judged
/// by where it leads.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_trusted_link_into_a_folder_others_can_change_is_refused() {
    let (_folder, root) = workspace();
    let open = folder(&root.as_path().join("open"), 0o777);
    let real = folder(&open.join("real"), 0o755);
    symlink("open/real", root.as_path().join("link")).expect("create the link");
    let (_, files) = closed_tree(&real);

    let result = delete(&root.as_path().join("link/selected"), &on_ext4(root.as_path()));

    assert_eq!(result, Err(others_can_change(&open)));
    assert_kept(&files);
}

/// parity: XFER-015, OPS-039
#[test]
fn a_loop_of_links_on_the_path_is_refused() {
    let (_folder, root) = workspace();
    symlink("b", root.as_path().join("a")).expect("create a link");
    symlink("a", root.as_path().join("b")).expect("create a link");

    // Found to be a folder by a listing: asked about first, the kernel
    // itself would give up on the loop before the check could.
    let result = delete_tree_with(
        &gio::File::for_path(root.join("a/selected")),
        Some(NodeKind::Directory),
        &Cancellation::new(),
        None,
        &on_ext4(&root),
    );

    // Link 41 is the 21st look at `a`.
    assert_eq!(result, Err(others_can_change(&root.as_path().join("a"))));
}

/// One removal of one name has nothing to redirect: a file and a link in
/// a folder others can change are still deleted, and the link's target
/// is kept.
///
/// parity: XFER-015, XFER-017, OPS-039
#[test]
fn a_file_or_a_link_is_deleted_without_the_folder_checks() {
    let (_folder, root) = workspace();
    let open = folder(&root.as_path().join("open"), 0o777);
    let lone = file(&open.join("lone.txt"));
    let target = folder(&root.as_path().join("target"), 0o755);
    let kept = file(&target.join("kept.txt"));
    let link = open.join("link");
    symlink(&target, &link).expect("create the link");
    let rules = on_ext4(root.as_path());

    assert_eq!(delete(&lone, &rules), Ok(()));
    let as_listed = delete_tree_with(
        &gio::File::for_path(&link),
        Some(NodeKind::Symlink),
        &Cancellation::new(),
        None,
        &rules,
    );
    assert_eq!(as_listed, Ok(()));

    assert!(fs::symlink_metadata(&lone).is_err());
    assert!(fs::symlink_metadata(&link).is_err());
    assert_kept(&[kept]);
}

/// A link that took a listed folder's place is deleted as a link.
///
/// parity: XFER-015, XFER-017, OPS-039
#[test]
fn a_link_in_place_of_a_folder_seen_earlier_is_removed_as_a_link() {
    let (_folder, root) = workspace();
    let target = folder(&root.as_path().join("target"), 0o755);
    let kept = file(&target.join("kept.txt"));
    let link = root.as_path().join("was a folder");
    symlink(&target, &link).expect("create the link");

    let result = delete_tree_with(
        &gio::File::for_path(&link),
        Some(NodeKind::Directory),
        &Cancellation::new(),
        None,
        &on_ext4(root.as_path()),
    );

    assert_eq!(result, Ok(()));
    assert!(fs::symlink_metadata(&link).is_err());
    assert_kept(&[kept]);
}

/// parity: XFER-015
#[test]
fn nesting_deeper_than_the_limit_is_refused_before_anything_is_deleted() {
    let (_folder, root) = workspace();
    let selected = folder(&root.as_path().join("selected"), 0o755);
    let top = file(&selected.join("top.txt"));
    let deepest = (0..=MAX_DEPTH).fold(selected.clone(), |path, _| path.join("d"));
    fs::create_dir_all(&deepest).expect("create the deep tree");

    let result = delete(&selected, &on_ext4(root.as_path()));

    assert_eq!(result, Err(nesting_error()));
    assert_kept(&[top, deepest]);
}

/// parity: XFER-015, XFER-020
#[test]
fn a_protected_descendant_stops_the_deletion_before_it_is_removed() {
    let (_folder, root) = workspace();
    let (selected, _) = closed_tree(root.as_path());
    let protected = folder(&selected.join("sub/.snapshot"), 0o755);
    let version = file(&protected.join("version"));
    let guard = |uri: &str| {
        if uri.contains(".snapshot") {
            Err(TransferError::failed("Protected snapshot."))
        } else {
            Ok(())
        }
    };

    let result = delete_tree_with(
        &gio::File::for_path(&selected),
        None,
        &Cancellation::new(),
        Some(&guard),
        &on_ext4(root.as_path()),
    );

    assert_eq!(result, Err(TransferError::failed("Protected snapshot.")));
    assert_kept(&[version]);
}

/// parity: XFER-015
#[test]
fn cancelling_stops_the_deletion_and_keeps_the_rest() {
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());
    let cancel = Cancellation::new();
    let requested = cancel.clone();
    let cancelling_guard = move |_: &str| {
        requested.cancel();
        Ok(())
    };

    let result = delete_tree_with(
        &gio::File::for_path(&selected),
        None,
        &cancel,
        Some(&cancelling_guard),
        &on_ext4(root.as_path()),
    );

    assert_eq!(result, Err(TransferError::Cancelled));
    assert_kept(&files);
}

/// What a real deletion trusts: root alone, every folder from `/` and this
/// process's own mount table. A folder of an ordinary user's is refused.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_real_deletion_trusts_only_root_and_judges_every_folder() {
    let production = Rules::default();
    assert_eq!(production.owner(), ROOT);
    assert_eq!(production.system(), Path::new("/"));
    assert!(production.judges(Path::new("/")));
    if rustix::process::getuid().is_root() {
        return;
    }
    let (_folder, root) = workspace();
    let (selected, files) = closed_tree(root.as_path());

    let result = delete_tree(&gio::File::for_path(&selected), None, &Cancellation::new(), None);

    assert!(
        matches!(&result, Err(TransferError::Failed(message)) if message.contains("can't be safely deleted")),
        "{result:?}"
    );
    assert_kept(&files);
}

mod user_mounts;
