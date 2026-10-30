// SPDX-License-Identifier: AGPL-3.0-only
//! The rules the privileged helper has to obey, checked on temporary
//! folders.
//!
//! The walk the helper runs is the walk these tests run, and the step
//! that elevates is replaced, so nothing here needs root and nothing
//! here is ever started through `pkexec`. They are written to say the
//! same thing whoever runs them: as an ordinary user every fixture is
//! already somebody root does not trust, and as root the fixtures that
//! stand for somebody else's are handed to `nobody`
//! ([`parent_path::make_untrusted`]) so that the rule under test is
//! still the one that fires.
//!
//! Nothing here writes outside the folder it made for itself.
//! `parent_path` does read paths of this machine's own - `/`, a
//! root-owned link such as `/bin`, and `/dev` - to measure the mount
//! rules against real mounts rather than against a fixture that only
//! claims to be one; it changes none of them, and it keeps its own
//! fixtures out of `/tmp` for the reason `parent_path::trusted_base`
//! gives.
//!
//! The matching test of the old behaviour, the walk by path that
//! `gvfsd-admin` deletions used to take, is
//! `gio_node::remote_delete::tests::a_folder_swapped_for_a_link_during_the_walk_is_followed`.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::report::{Record, Totals};
use super::walk::{self, kernel_mounts, Options};
use crate::safe_walk::Mount;
use crate::test_support::temporary_folder;
use crate::transfer::{Cancellation, TransferError};

/// Everything one run of the walk reported.
#[derive(Default)]
struct Reported {
    records: Vec<Record>,
}

impl Reported {
    /// The items the walk left in place, with the reason it gave.
    fn left_behind(&self) -> Vec<&PathBuf> {
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::Skipped { path, .. } | Record::Failed { path, .. } => Some(path),
                Record::Deleted(_) => None,
            })
            .collect()
    }
}

/// Runs the helper's walk on `target` and returns its totals and records.
fn run(target: &Path, cancel: &Cancellation, options: &Options) -> (Result<Totals, TransferError>, Reported) {
    let mut reported = Reported::default();
    let result = walk::delete_tree(target, cancel, options, &mut |record| {
        reported.records.push(record);
    });
    (result, reported)
}

/// Runs the helper's walk with the rules of a real deletion.
fn delete(target: &Path) -> (Result<Totals, TransferError>, Reported) {
    run(target, &Cancellation::new(), &Options::default())
}

/// A mount probe that puts one path, and everything the walk reads through
/// it, on a drive of its own.
fn another_drive_at(elsewhere: PathBuf) -> walk::MountProbe {
    Box::new(move |path, mount| {
        if path == elsewhere {
            Mount {
                id: Some(u64::MAX),
                device: (u32::MAX, u32::MAX),
            }
        } else {
            mount
        }
    })
}

/// A folder swapped for a symbolic link while the walk is inside it
/// cannot redirect the walk: every name below the selected folder is
/// resolved against a descriptor the walk already holds, so the items
/// removed are the ones that were really there, and the link's target is
/// untouched. The walk notices the swap and stops rather than finishing
/// on a tree that is no longer the one it was given.
///
/// The same tree deleted by path, the way `gvfsd-admin` deletions worked,
/// deletes the target instead; that is
/// `gio_node::remote_delete::tests::a_folder_swapped_for_a_link_during_the_walk_is_followed`.
///
/// parity: XFER-015
#[test]
fn a_folder_swapped_for_a_link_during_the_walk_is_not_followed() {
    let root = temporary_folder();
    let (selected, inner, outside) = swap_fixture(root.path());
    let swapped = Arc::new(Mutex::new(false));
    let swapping = Arc::clone(&swapped);
    let inner_for_swap = inner.clone();
    let options = Options {
        at_item: Some(Box::new(move |path| {
            if path != inner_for_swap.join("bait") {
                return;
            }
            // The walk is holding `inner` open and is about to remove the
            // item inside it. Someone who can write to `selected` puts a
            // link to another tree there under the same name.
            fs::remove_dir_all(&inner_for_swap).expect("the folder is removed");
            symlink(inner_for_swap.parent().unwrap().join("outside"), &inner_for_swap)
                .expect("the link replaces it");
            *swapping.lock().expect("the lock is not poisoned") = true;
        })),
        ..Options::default()
    };

    let (result, _reported) = run(&selected, &Cancellation::new(), &options);

    assert!(
        *swapped.lock().expect("the lock is not poisoned"),
        "the swap never happened"
    );
    assert_eq!(
        result,
        Err(crate::safe_walk::folder_changed()),
        "the walk finished on a tree that had been swapped under it"
    );
    assert_eq!(
        fs::read(outside.join("bait")).expect("the link's target is untouched"),
        b"not yours"
    );
}

/// The tree both walks are tried on: a selected folder with one item
/// inside a nested folder, and another tree outside it holding an item of
/// the same name. Returns the selected folder, its nested folder and the
/// tree outside.
pub(super) fn swap_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let outside = root.join("outside");
    fs::create_dir(&outside).expect("the tree outside is created");
    fs::write(outside.join("bait"), b"not yours").expect("its item is written");
    let selected = root.join("selected");
    let inner = selected.join("inner");
    fs::create_dir_all(&inner).expect("the selected tree is created");
    fs::write(inner.join("bait"), b"mine").expect("its item is written");
    (selected, inner, outside)
}

/// parity: XFER-015
#[test]
fn a_deep_chain_of_folders_with_items_at_several_depths_is_deleted() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    let mut folder = selected.clone();
    for level in 0..12 {
        folder = folder.join(format!("level {level}"));
        fs::create_dir_all(&folder).expect("the level is created");
        fs::write(folder.join("data.txt"), b"delete me").expect("its item is written");
    }
    fs::create_dir(selected.join("empty")).expect("an empty folder is created");
    fs::write(selected.join(".hidden"), b"hidden").expect("a hidden item is written");

    let (result, _reported) = delete(&selected);

    let totals = result.expect("the whole tree is deleted");
    assert!(totals.is_complete(), "{totals:?}");
    // 12 folders, 12 items, the empty folder, the hidden item and the
    // selected folder itself.
    assert_eq!(totals.deleted, 27);
    assert!(!selected.exists());
}

/// parity: XFER-015, XFER-017
#[test]
fn a_link_inside_is_removed_as_a_link_and_its_target_outside_survives() {
    let root = temporary_folder();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).expect("the target folder is created");
    fs::write(outside.join("kept.txt"), b"kept").expect("its item is written");
    let kept_file = root.path().join("kept-file.txt");
    fs::write(&kept_file, b"kept too").expect("the target file is written");
    let selected = root.path().join("selected");
    fs::create_dir(&selected).expect("the selected folder is created");
    symlink(&outside, selected.join("link to a folder")).expect("the folder link is created");
    symlink(&kept_file, selected.join("link to a file")).expect("the file link is created");

    let (result, _reported) = delete(&selected);

    assert!(result.expect("the tree is deleted").is_complete());
    assert!(!selected.exists());
    assert_eq!(
        fs::read(outside.join("kept.txt")).expect("the target folder is kept"),
        b"kept"
    );
    assert_eq!(
        fs::read(&kept_file).expect("the target file is kept"),
        b"kept too"
    );
}

/// A drive, share or bind mount inside the selected folder is never
/// entered and never removed, and the deletion says so rather than
/// reporting a clean sweep.
///
/// parity: XFER-015
#[test]
fn a_folder_on_another_mount_is_not_entered_and_is_reported() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    let mounted = selected.join("mounted");
    fs::create_dir_all(&mounted).expect("the tree is created");
    fs::write(mounted.join("on the drive.txt"), b"keep me").expect("the drive's item is written");
    fs::write(selected.join("mine.txt"), b"delete me").expect("the selected item is written");
    let options = Options {
        mounts: another_drive_at(mounted.clone()),
        ..Options::default()
    };

    let (result, reported) = run(&selected, &Cancellation::new(), &options);

    let totals = result.expect("the walk runs");
    assert!(!totals.is_complete(), "{totals:?}");
    assert_eq!(totals.skipped, 1);
    assert_eq!(
        fs::read(mounted.join("on the drive.txt")).expect("the drive's item is kept"),
        b"keep me"
    );
    assert!(!selected.join("mine.txt").exists());
    // The folder that holds it cannot go either, and the deletion names
    // both rather than claiming to have emptied the tree.
    assert_eq!(reported.left_behind(), vec![&mounted, &selected]);
}

/// The item itself being on another mount means it is a mount point: the
/// drive, not an item on it.
///
/// parity: XFER-015
#[test]
fn a_mount_point_is_refused_and_nothing_is_deleted() {
    let root = temporary_folder();
    let selected = root.path().join("Data");
    fs::create_dir(&selected).expect("the mount point is created");
    fs::write(selected.join("on the drive.txt"), b"keep me").expect("the item is written");
    let options = Options {
        mounts: another_drive_at(selected.clone()),
        ..Options::default()
    };

    let (result, reported) = run(&selected, &Cancellation::new(), &options);

    assert_eq!(result, Err(walk::mount_point_refusal(&selected)));
    assert!(reported.records.is_empty(), "{:?}", reported.records);
    assert_eq!(
        fs::read(selected.join("on the drive.txt")).expect("the drive's item is kept"),
        b"keep me"
    );
}

/// parity: XFER-015
#[test]
fn a_filesystem_root_a_relative_path_and_a_path_through_dotdot_are_refused() {
    // Each path gets the refusal that names what is wrong with it, so
    // that a check which stopped working and let the path through to
    // fail later, on a missing name or a kind it will not open, cannot
    // be mistaken here for the check still doing its job.
    let refusals = [
        (Path::new("/"), walk::filesystem_root_refusal()),
        (Path::new("relative/folder"), walk::own_absolute_path_refusal()),
        (Path::new("/srv/a/../b"), walk::own_absolute_path_refusal()),
        (Path::new("/srv/."), walk::own_absolute_path_refusal()),
        (
            Path::new("/srv/data/"),
            walk::trailing_slash_refusal(Path::new("/srv/data/")),
        ),
    ];

    for (refused, expected) in refusals {
        let (result, reported) = run(refused, &Cancellation::new(), &Options::default());

        assert_eq!(result, Err(expected), "{}", refused.display());
        assert!(
            reported.records.is_empty(),
            "{} reported {:?}",
            refused.display(),
            reported.records
        );
    }
}

/// Resolving the selected item's own folder is the step a privileged
/// helper cannot take by path: a link anywhere above the item could have
/// been planted by whoever can write to one of those folders. With the
/// rules a real deletion runs with, where root is the only owner the walk
/// trusts, a link the test user made is refused rather than resolved.
/// `tests::parent_path` holds the rest of that rule, including the links
/// that are followed.
///
/// parity: XFER-015
#[test]
fn a_link_in_the_folders_above_the_item_is_refused_rather_than_followed() {
    let root = temporary_folder();
    let real = root.path().join("real");
    let item = real.join("selected");
    fs::create_dir_all(&item).expect("the real tree is created");
    fs::write(item.join("data.txt"), b"keep me").expect("its item is written");
    let through_a_link = root.path().join("link").join("selected");
    symlink(&real, root.path().join("link")).expect("the link is created");
    // Run as root the link would belong to root, which a real deletion
    // trusts; give it away so the rule being tested is the same one.
    parent_path::make_untrusted(&root.path().join("link"));

    let (result, reported) = delete(&through_a_link);

    assert_eq!(
        result,
        Err(super::walk::untrusted_link_refusal(&root.path().join("link")))
    );
    assert!(reported.records.is_empty(), "{:?}", reported.records);
    assert_eq!(
        fs::read(item.join("data.txt")).expect("nothing was deleted"),
        b"keep me"
    );
}

/// parity: XFER-015
#[test]
fn cancelling_stops_the_walk_and_keeps_what_it_has_not_reached() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    let later = selected.join("later");
    fs::create_dir_all(&later).expect("the tree is created");
    fs::write(later.join("kept.txt"), b"kept").expect("the later item is written");
    let cancel = Cancellation::new();
    let cancelling = cancel.clone();
    let options = Options {
        // The walk asks about the selected folder first; cancel there, so
        // nothing inside it is reached.
        at_item: Some(Box::new(move |_path| cancelling.cancel())),
        ..Options::default()
    };

    let (result, reported) = run(&selected, &cancel, &options);

    assert_eq!(result, Err(TransferError::Cancelled));
    assert!(reported.records.is_empty(), "{:?}", reported.records);
    assert_eq!(
        fs::read(later.join("kept.txt")).expect("the item is kept"),
        b"kept"
    );
}

/// parity: XFER-015
#[test]
fn the_default_walk_asks_the_kernel_which_mount_a_folder_is_on() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    fs::create_dir(&selected).expect("the folder is created");
    let unchanged = Mount {
        id: Some(7),
        device: (1, 2),
    };

    assert_eq!(kernel_mounts()(&selected, unchanged), unchanged);
}

/// The folder being emptied is looked at again between its items, so a
/// folder whose name somebody takes over part way through stops the
/// walk rather than carrying on inside whatever now answers to the
/// name.
///
/// The hook fires while the walk is on the first item it found, which
/// is before the check that sits at the top of each further item, and
/// moves the folder away to put another one in its place. Everything
/// the walk does still goes through the descriptor it holds, so the
/// swap cannot redirect a deletion; what it must not do is leave the
/// walk reporting a clean sweep of a tree that is no longer the one it
/// was given.
///
/// parity: XFER-015
#[test]
fn a_folder_whose_name_is_taken_over_part_way_through_stops_the_walk() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    fs::create_dir(&selected).expect("the folder is created");
    for item in ["one.txt", "two.txt"] {
        fs::write(selected.join(item), b"mine").expect("the item is written");
    }
    let stand_in = root.path().join("stand-in");
    fs::create_dir(&stand_in).expect("the stand-in is created");
    fs::write(stand_in.join("not yours.txt"), b"not yours").expect("its item is written");
    let moved_to = root.path().join("moved");
    let taken_over = Arc::new(Mutex::new(false));
    let taking_over = Arc::clone(&taken_over);
    let (selected_name, stand_in_name, moved_name) = (selected.clone(), stand_in.clone(), moved_to.clone());
    let options = Options {
        at_item: Some(Box::new(move |path| {
            if path.parent() != Some(selected_name.as_path()) {
                return;
            }
            let mut taken_over = taking_over.lock().expect("the lock is not poisoned");
            if *taken_over {
                return;
            }
            fs::rename(&selected_name, &moved_name).expect("the folder is moved aside");
            fs::rename(&stand_in_name, &selected_name).expect("the stand-in takes the name");
            *taken_over = true;
        })),
        ..Options::default()
    };

    let (result, _reported) = run(&selected, &Cancellation::new(), &options);

    assert!(
        *taken_over.lock().expect("the lock is not poisoned"),
        "the name was never taken over"
    );
    assert_eq!(result, Err(crate::safe_walk::folder_changed()));
    // The stand-in is untouched, and so is everything the walk had not
    // reached inside the folder it was really given.
    assert_eq!(
        fs::read(selected.join("not yours.txt")).expect("the stand-in's item is kept"),
        b"not yours"
    );
    assert_eq!(
        fs::read_dir(&moved_to)
            .expect("the folder the walk held is still there")
            .count(),
        1,
        "the walk went on after the swap"
    );
}

/// A bind mount of a single file inside the selected folder is left in
/// place too. The mount check in front of every item is what stops it:
/// the one in front of every folder cannot, because this is not a
/// folder.
///
/// parity: XFER-015
#[test]
fn a_file_on_another_mount_inside_the_tree_is_not_removed() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    fs::create_dir(&selected).expect("the folder is created");
    let mounted = selected.join("mounted.txt");
    fs::write(&mounted, b"keep me").expect("the mounted file is written");
    fs::write(selected.join("mine.txt"), b"delete me").expect("the selected item is written");
    let options = Options {
        mounts: another_drive_at(mounted.clone()),
        ..Options::default()
    };

    let (result, reported) = run(&selected, &Cancellation::new(), &options);

    let totals = result.expect("the walk runs");
    assert!(!totals.is_complete(), "{totals:?}");
    assert_eq!(totals.skipped, 1);
    assert_eq!(fs::read(&mounted).expect("the mounted file is kept"), b"keep me");
    assert!(!selected.join("mine.txt").exists());
    assert_eq!(reported.left_behind(), vec![&mounted, &selected]);
}

/// A tree deeper than the limit the engine uses everywhere else stops
/// the walk, with the same refusal, rather than descending as far as
/// the kernel will let it.
///
/// parity: XFER-015
#[test]
fn a_tree_deeper_than_the_limit_is_refused() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    let deepest = selected.join("one").join("two");
    fs::create_dir_all(&deepest).expect("the tree is created");
    fs::write(deepest.join("deep.txt"), b"keep me").expect("the deep item is written");
    let options = Options {
        max_depth: 1,
        ..Options::default()
    };

    let (result, _reported) = run(&selected, &Cancellation::new(), &options);

    assert_eq!(result, Err(crate::transfer::nesting_error()));
    assert!(
        deepest.join("deep.txt").exists(),
        "nothing below the limit was deleted"
    );
}

mod parent_path;
mod routing;
