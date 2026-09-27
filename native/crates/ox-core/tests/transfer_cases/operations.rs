// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move and permanent delete on local files: staging, links, modes,
//! special files, self-descendant destinations and cancellation. Ports the
//! cases of `TransferTests` in `desktop/tests/test_operations.py` that the
//! other case files do not.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, TransferError, TransferMode, MAX_DEPTH};

use crate::transfer_support::{
    local::{self, local_path_of, LocalNode, Provider},
    *,
};

/// Port of `test_copy_file`.
///
/// parity: XFER-001
#[test]
fn a_copied_file_arrives_complete_and_leaves_no_staging() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("data.bin");
    let content = random_bytes(35_000);
    fs::write(&source, &content).unwrap();

    let result = fixture.copy(local::local(), &[&source], ConflictPolicy::Skip);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [uri(&source)]);
    assert_eq!(
        fs::read(fixture.destination_folder.join("data.bin")).unwrap(),
        content
    );
    fixture.assert_no_staging();
}

/// Ports `test_recursive_copy_includes_hidden`,
/// `test_symlink_copied_not_followed` and
/// `test_nested_symlink_loop_not_traversed`.
///
/// parity: XFER-001, XFER-005, XFER-017
#[test]
fn recursive_copy_preserves_sources_hidden_files_links_and_modes() {
    let fixture = Fixture::new();
    let _access = RestoreOwnerAccess::new(&fixture.root);
    let folder = fixture.source_folder.join("tree");
    let nested = folder.join("nested");
    fs::create_dir_all(&nested).unwrap();
    write(&nested.join(".hidden"), "hidden content");
    symlink("..", nested.join("loop")).unwrap();
    symlink("missing", nested.join("dangling")).unwrap();
    set_mode(&folder, 0o750);
    set_mode(&nested, 0o500);
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&folder],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [uri(&folder)]);
    assert_eq!(read(&nested.join(".hidden")), "hidden content");
    let copied = fixture.destination_folder.join("tree/nested");
    assert_eq!(read(&copied.join(".hidden")), "hidden content");
    assert_eq!(fs::read_link(copied.join("loop")).unwrap(), Path::new(".."));
    assert_eq!(
        fs::read_link(copied.join("dangling")).unwrap(),
        Path::new("missing")
    );
    assert_eq!(mode_of(&fixture.destination_folder.join("tree")), 0o750);
    assert_eq!(mode_of(&copied), 0o500);
    fixture.assert_no_staging();
}

/// Port of `test_duplicate_sources_deduplicated`.
///
/// parity: XFER-019
#[test]
fn a_source_selected_twice_is_copied_once() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "a");

    let result = fixture.copy(local::local(), &[&source, &source], ConflictPolicy::Skip);

    assert_eq!(result.done, [uri(&source)]);
    assert!(
        result.errors.is_empty() && result.skipped.is_empty(),
        "{result:?}"
    );
    assert_eq!(list(&fixture.destination_folder), ["a"]);
}

/// Port of `test_move_native`.
///
/// parity: XFER-011
#[test]
fn a_move_takes_the_item_out_of_its_folder() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "a");
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.done, [uri(&source)]);
    assert!(!lexists(&source));
    assert_eq!(read(&fixture.destination_folder.join("a")), "a");
}

/// Ports `test_reject_self_descendant` and
/// `test_reject_symlink_destination_inside_source`.
///
/// parity: XFER-016
#[test]
fn self_and_descendant_destinations_are_rejected_including_symlink_aliases() {
    for mode in [TransferMode::Copy, TransferMode::Move] {
        let fixture = Fixture::new();
        let folder = fixture.source_folder.join("tree");
        let nested = folder.join("nested");
        fs::create_dir_all(&nested).unwrap();
        write(&folder.join("original"), "untouched");
        let alias = fixture.root.join("alias");
        symlink(&nested, &alias).unwrap();
        for destination in [&folder, &nested, &alias] {
            let mut engine = fixture.engine(local::local());

            let result = fixture.run(
                &mut engine,
                &[&folder],
                mode,
                ConflictPolicy::Replace,
                Some(destination),
            );

            assert!(result.done.is_empty());
            assert!(result.errors[0].contains("inside itself"));
            assert_eq!(read(&folder.join("original")), "untouched");
            assert!(list(&nested).is_empty());
        }
    }
}

/// Port of `test_trash_unsupported_no_delete`; deleting a link removes the
/// link and never its target.
///
/// parity: XFER-014, XFER-015, XFER-017
#[test]
fn delete_does_not_follow_symlinks_and_trash_never_falls_back_to_delete() {
    let fixture = Fixture::new();
    let original = fixture.source_folder.join("original");
    write(&original, "keep");
    let link = fixture.source_folder.join("link");
    symlink(&original, &link).unwrap();
    let mut engine = fixture.engine(local::local());

    let trash = fixture.run(
        &mut engine,
        &[&original],
        TransferMode::Trash,
        ConflictPolicy::Skip,
        None,
    );

    assert!(trash.done.is_empty());
    assert!(trash.errors[0].contains("no delete fallback"));

    let deleted = fixture.run(
        &mut engine,
        &[&link],
        TransferMode::Delete,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(deleted.done, [uri(&link)]);
    assert!(!lexists(&link));
    assert_eq!(read(&original), "keep");
}

/// Ports `test_delete_removes_tree_permanently` and
/// `test_delete_does_not_need_a_destination`.
///
/// parity: XFER-015
#[test]
fn permanent_delete_removes_folders_and_files_without_a_destination() {
    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    write(&tree.join("a"), "a");
    write(&tree.join("sub/b"), "b");
    let loose = fixture.source_folder.join("loose");
    write(&loose, "x");
    let mut engine = fixture.engine(local::local());

    let result = engine
        .run(
            TransferMode::Delete,
            &[uri(&tree), uri(&loose)],
            None,
            ConflictPolicy::Skip,
            &fixture.cancel,
        )
        .expect("a permanent delete needs no destination");

    assert_eq!(result.done.len(), 2, "{result:?}");
    assert!(result.errors.is_empty(), "{result:?}");
    assert!(!lexists(&tree));
    assert!(!lexists(&loose));
}

/// Ports `test_special_file_rejected_cleanup`; a tree deeper than the
/// nesting limit is refused the same way.
///
/// parity: XFER-018
#[test]
fn deep_trees_and_special_files_are_not_published() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("tree");
    let mut nested = source.clone();
    for _ in 0..=MAX_DEPTH + 1 {
        nested.push("d");
    }
    fs::create_dir_all(nested).unwrap();
    let fifo = fixture.source_folder.join("pipe");
    mkfifo(&fifo);
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&source, &fifo],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 2);
    assert!(result.errors[0].contains("nesting"));
    assert!(result.errors[1].contains("special files"));
    assert!(list(&fixture.destination_folder).is_empty());
    assert!(lexists(&fifo));
}

/// Port of `test_copy_cancel_removes_partial_stage`: the user's
/// cancellation stops the copy between blocks, removes its staging and
/// starts no later item.
///
/// parity: OPS-022, XFER-001
#[test]
fn cancellation_during_copy_removes_partial_stage_and_stops_the_batch() {
    let fixture = Fixture::new();
    let first = fixture.source_folder.join("large");
    let later = fixture.source_folder.join("later");
    let content = random_bytes(32_768);
    fs::write(&first, &content).unwrap();
    write(&later, "later");
    let cancel = fixture.cancel.clone();
    let updates = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&updates);
    let mut engine = fixture.engine(local::local()).with_progress(move |progress| {
        if progress.label.starts_with("Copying ") {
            cancel.cancel();
        }
        recorded.lock().unwrap().push(progress);
    });

    let result = fixture.run(
        &mut engine,
        &[&first, &later],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.cancelled);
    assert!(result.done.is_empty());
    assert!(result.errors.is_empty(), "{result:?}");
    assert!(list(&fixture.destination_folder).is_empty());
    assert_eq!(fs::read(first).unwrap(), content);
    assert_eq!(read(&later), "later");
    let byte_updates = updates
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.label.starts_with("Copying "))
        .count();
    assert_eq!(byte_updates, 1);
}

/// Port of `test_failure_inside_tree_leaves_source`: one special file deep
/// inside a folder fails the whole folder. Nothing is published, the stage
/// is removed and the source is untouched.
///
/// parity: XFER-018
#[test]
fn a_special_file_inside_a_folder_fails_the_whole_folder() {
    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    fs::create_dir(&tree).unwrap();
    write(&tree.join("a"), "hello");
    mkfifo(&tree.join("pipe"));
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&tree],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("special files"), "{result:?}");
    assert!(list(&fixture.destination_folder).is_empty());
    assert_eq!(read(&tree.join("a")), "hello");
}

/// Port of `test_cancel_before_start`: a run cancelled before it starts is
/// refused while the destination is checked, before anything is touched.
///
/// parity: OPS-022
#[test]
fn a_run_cancelled_before_it_starts_changes_nothing() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "a");
    fixture.cancel.cancel();
    let mut engine = fixture.engine(local::local());

    let refused = fixture.try_run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(refused, Err(TransferError::Cancelled));
    assert!(list(&fixture.destination_folder).is_empty());
    assert_eq!(read(&source), "a");
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
    let source = fixture.source_folder.join("a");
    write(&source, "a");
    let watched = Arc::new(WatchedLocal::default());

    let result = fixture.copy(watched.clone(), &[&source], ConflictPolicy::Skip);

    assert!(result.errors.is_empty(), "{result:?}");
    let targets = watched.targets.lock().expect("target log");
    assert_eq!(targets[0].file_name(), Some("payload".as_ref()));
    let folder = targets[0].parent().expect("payload has a parent");
    assert!(is_staging_path(folder), "{}", folder.display());
    assert_eq!(folder.parent(), Some(fixture.destination_folder.as_path()));
    assert_eq!(read(&fixture.destination_folder.join("a")), "a");
}
