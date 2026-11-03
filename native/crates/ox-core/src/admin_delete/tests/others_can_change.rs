// SPDX-License-Identifier: AGPL-3.0-only
//! Folders 2.0.12 refused to delete as administrator, which the pinned
//! walk deletes.
//!
//! Until this walk, a whole `admin://` folder was deleted by path, each
//! step resolved again from `/` by `gvfsd-admin` as root, and the only
//! safe answer was to refuse, before anything was deleted, every folder
//! that another account could change (`gio_node/admin_delete.rs` in
//! 2.0.12): a folder inside the tree, the selected folder or a folder on
//! the way to it that was group- or other-writable or not owned by root,
//! and anything on a drive whose owners and modes could not be believed.
//!
//! The walk resolves every name below the selected folder against a
//! descriptor it already holds, so none of those folders can redirect it.
//! Each test here takes one shape that used to be refused and checks two
//! things: that the walk deletes exactly the selected tree and nothing
//! outside it, and that the race the advisory describes, a folder inside
//! the tree swapped for a symbolic link to a folder outside while the walk
//! is inside it, by whoever can write to the folder that holds it, cannot
//! reach outside either.
//!
//! What 2.0.12 refused and the walk still refuses keeps its test
//! elsewhere: a link on the path in a folder others can change
//! (`parent_path`), a mount point as the item (`tests`), a loop of links
//! (`parent_path`) and a Flatpak (`elevation`).

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::parent_path::make_untrusted;
use super::{run, swap_fixture, walk, Options};
use crate::safe_walk::{folder_changed, Mount};
use crate::test_support::temporary_folder;
use crate::transfer::Cancellation;

/// What a test reads back about a tree: every item below it, with the
/// contents of each file, in a fixed order.
type Snapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

/// Every item below `folder`, read without following links.
fn snapshot(folder: &Path) -> Snapshot {
    let mut items = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("the folder is listed") {
            let path = entry.expect("the entry is read").path();
            let status = fs::symlink_metadata(&path).expect("the item is inspected");
            let contents = if status.is_dir() {
                pending.push(path.clone());
                None
            } else if status.is_symlink() {
                Some(
                    fs::read_link(&path)
                        .expect("the link is read")
                        .into_os_string()
                        .into_encoded_bytes(),
                )
            } else {
                Some(fs::read(&path).expect("the file is read"))
            };
            items.push((path, contents));
        }
    }
    items.sort();
    items
}

/// Gives `path` the mode `bits`.
fn set_mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).expect("the mode is set");
}

/// The tree each test deletes, made under `holder`: `holder/selected`
/// with `selected/<writable>` given `mode` (the selected folder itself
/// when `writable` is empty), a file and a folder `inner` with a file in
/// it inside that folder, and a folder `outside` beside the selected one
/// with a file of the same name and one deeper. Returns the selected
/// folder, the folder others can write to, and the tree outside.
fn tree(holder: &Path, writable: &str, mode: u32) -> (PathBuf, PathBuf, PathBuf) {
    let (selected, _, outside) = swap_fixture(holder);
    fs::create_dir(outside.join("deeper")).expect("the deeper folder outside is created");
    fs::write(outside.join("deeper/kept.txt"), b"not yours either").expect("its item is written");
    let open = if writable.is_empty() {
        selected.clone()
    } else {
        selected.join(writable)
    };
    let inner = open.join("inner");
    fs::create_dir_all(&inner).expect("the folder others can write to is created");
    fs::write(inner.join("bait"), b"mine").expect("its item is written");
    fs::write(open.join("shared.txt"), b"mine too").expect("the shared item is written");
    set_mode(&open, mode);
    (selected, open, outside)
}

/// Deletes `selected` with `options` and asserts that it is gone, that the
/// walk reported nothing left behind, and that nothing in `outside`
/// changed.
fn assert_deletes_exactly(selected: &Path, outside: &Path, options: &Options, case: &str) {
    let before = snapshot(outside);

    let (result, reported) = run(selected, &Cancellation::new(), options);

    let totals = result.unwrap_or_else(|error| panic!("{case}: refused: {error}"));
    assert!(totals.is_complete(), "{case}: {totals:?}");
    assert!(
        reported.left_behind().is_empty(),
        "{case}: {:?}",
        reported.left_behind()
    );
    assert!(
        fs::symlink_metadata(selected).is_err(),
        "{case}: the selected folder is still there"
    );
    assert_eq!(
        snapshot(outside),
        before,
        "{case}: something outside the selected tree changed"
    );
}

/// Deletes `selected` with `options` while, just before the walk removes
/// `open/inner/bait`, somebody who can write to `open` removes `inner` and
/// puts a link to `outside` under its name. Asserts that the swap happened,
/// that the walk stopped on it rather than finishing, and that nothing in
/// `outside` changed.
fn assert_the_race_stays_inside(selected: &Path, open: &Path, outside: &Path, options: Options, case: &str) {
    let before = snapshot(outside);
    let inner = open.join("inner");
    let swapped = Arc::new(Mutex::new(false));
    let swapping = Arc::clone(&swapped);
    let (bait, target) = (inner.join("bait"), outside.to_path_buf());
    let options = Options {
        at_item: Some(Box::new(move |path| {
            if path != bait {
                return;
            }
            fs::remove_dir_all(&inner).expect("the folder is removed");
            symlink(&target, &inner).expect("the link replaces it");
            *swapping.lock().expect("the lock is not poisoned") = true;
        })),
        ..options
    };

    let (result, _reported) = run(selected, &Cancellation::new(), &options);

    assert!(
        *swapped.lock().expect("the lock is not poisoned"),
        "{case}: the swap never happened"
    );
    assert_eq!(
        result,
        Err(folder_changed()),
        "{case}: the walk finished on a swapped tree"
    );
    assert_eq!(
        snapshot(outside),
        before,
        "{case}: the swap reached outside the selected tree"
    );
}

/// 2.0.12 refused a folder inside the tree that the group or others can
/// write to, sticky or not (`a_folder_inside_that_others_can_write_to_is_refused_before_anything_is_deleted`,
/// `a_group_writable_folder_and_a_sticky_world_writable_one_are_refused`).
/// `0o775` is the shape of `/var/log` on several distributions, `0o1777`
/// and `0o1733` the shapes of `/tmp` and of a spool folder.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_inside_that_others_can_write_to_is_deleted_and_nothing_outside() {
    for mode in [0o757, 0o770, 0o775, 0o2775, 0o777, 0o1777, 0o1733] {
        let case = format!("mode {mode:o}");
        let root = temporary_folder();
        let (selected, _, outside) = tree(root.path(), "sub/deeper/open", mode);
        assert_deletes_exactly(&selected, &outside, &Options::default(), &case);

        let root = temporary_folder();
        let (selected, open, outside) = tree(root.path(), "sub/deeper/open", mode);
        assert_the_race_stays_inside(&selected, &open, &outside, Options::default(), &case);
    }
}

/// 2.0.12 refused the selected folder itself when others could write to
/// it (`the_selected_folder_itself_open_to_others_is_refused`), and that
/// refusal was what kept its walk by path from the swap
/// (`a_folder_swapped_for_a_link_part_way_through_cannot_reach_outside`).
///
/// parity: XFER-015, OPS-039
#[test]
fn the_selected_folder_open_to_others_is_deleted_and_nothing_outside() {
    for mode in [0o775, 0o777, 0o1777] {
        let case = format!("mode {mode:o}");
        let root = temporary_folder();
        let (selected, _, outside) = tree(root.path(), "", mode);
        assert_deletes_exactly(&selected, &outside, &Options::default(), &case);

        let root = temporary_folder();
        let (selected, open, outside) = tree(root.path(), "", mode);
        assert_the_race_stays_inside(&selected, &open, &outside, Options::default(), &case);
    }
}

/// 2.0.12 refused a folder with a folder on the way to it that others can
/// change (`a_folder_above_that_others_can_change_is_refused`): anything
/// in `/tmp`, `/var/tmp` or a group-writable system folder. Every test
/// here already runs below the machine's own temporary folder, so this
/// one adds such a folder directly above the selected one, and checks
/// that the rest of that folder is left as it was.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_folder_on_the_way_that_others_can_change_does_not_stop_the_deletion() {
    for mode in [0o775, 0o777, 0o1777] {
        let case = format!("mode {mode:o}");
        let root = temporary_folder();
        let above = root.path().join("above");
        fs::create_dir(&above).expect("the folder above is created");
        let (selected, _, outside) = tree(&above, "sub", 0o755);
        let neighbour = above.join("neighbour.txt");
        fs::write(&neighbour, b"not yours").expect("the neighbour is written");
        set_mode(&above, mode);
        assert_deletes_exactly(&selected, &outside, &Options::default(), &case);
        assert_eq!(
            fs::read(&neighbour).expect("the neighbour is kept"),
            b"not yours",
            "{case}"
        );

        let root = temporary_folder();
        let above = root.path().join("above");
        fs::create_dir(&above).expect("the folder above is created");
        let (selected, open, outside) = tree(&above, "sub", 0o755);
        set_mode(&above, mode);
        assert_the_race_stays_inside(&selected, &open, &outside, Options::default(), &case);
    }
}

/// 2.0.12 refused a folder that root does not own
/// (`a_folder_another_account_owns_is_refused`), which is every folder in
/// a user's home and the home folder itself. Run as an ordinary user the
/// fixture already belongs to somebody who is not root; run as root it is
/// handed to `nobody`, so the walk deletes a tree root does not own
/// either way, with the rules of a real deletion.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_tree_another_account_owns_is_deleted_and_nothing_outside() {
    let give_away = |selected: &Path| {
        let mut pending = vec![selected.to_path_buf()];
        while let Some(next) = pending.pop() {
            make_untrusted(&next);
            if fs::symlink_metadata(&next).is_ok_and(|status| status.is_dir()) {
                pending.extend(
                    fs::read_dir(&next)
                        .expect("listed")
                        .map(|entry| entry.expect("read").path()),
                );
            }
        }
    };
    let root = temporary_folder();
    let (selected, _, outside) = tree(root.path(), "sub", 0o700);
    give_away(&selected);
    assert_deletes_exactly(
        &selected,
        &outside,
        &Options::default(),
        "owned by another account",
    );

    let root = temporary_folder();
    let (selected, open, outside) = tree(root.path(), "sub", 0o700);
    give_away(&selected);
    assert_the_race_stays_inside(
        &selected,
        &open,
        &outside,
        Options::default(),
        "owned by another account",
    );
}

/// 2.0.12 refused a folder on a filesystem whose owners and modes an
/// ordinary user could have written (`a_folder_on_a_filesystem_users_can_supply_is_refused`,
/// `only_local_kernel_filesystems_mounted_by_the_administrator_are_trusted`):
/// FUSE, network shares, FAT and exFAT, loop images and drives under
/// `/media` or `/run/media`. The walk believes nothing such a drive says
/// about owners and modes, because it never needs to: here the whole
/// fixture is on a drive of its own, and the walk still deletes exactly
/// the selected tree and does not follow the swap.
///
/// parity: XFER-015, OPS-039
#[test]
fn a_tree_on_a_drive_whose_permissions_cannot_be_checked_is_deleted_and_nothing_outside() {
    const USB_STICK: Mount = Mount {
        id: Some(u64::MAX),
        device: (u32::MAX, u32::MAX),
    };
    let on_the_stick = |stick: PathBuf| -> walk::MountProbe {
        Box::new(move |path, mount| {
            if path.starts_with(&stick) {
                USB_STICK
            } else {
                mount
            }
        })
    };
    let root = temporary_folder();
    let (selected, _, outside) = tree(root.path(), "sub", 0o777);
    let options = Options {
        mounts: on_the_stick(root.path().to_path_buf()),
        ..Options::default()
    };
    assert_deletes_exactly(&selected, &outside, &options, "on another drive");

    let root = temporary_folder();
    let (selected, open, outside) = tree(root.path(), "sub", 0o777);
    let options = Options {
        mounts: on_the_stick(root.path().to_path_buf()),
        ..Options::default()
    };
    assert_the_race_stays_inside(&selected, &open, &outside, options, "on another drive");
}
