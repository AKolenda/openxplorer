// SPDX-License-Identifier: AGPL-3.0-only
//! Local copy, move, deletion and preflight invariants. Ports the cases of
//! `TransferTests` and `ProtectedTransferTests` in
//! `desktop/tests/test_operations.py` that the other case files do not.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{
    Cancellation, ConflictPolicy, Node, NodeKind, TransferError, TransferMode, MAX_DEPTH,
};

use crate::transfer_support::{
    local::{self, LocalNode, Provider},
    versions::PreviousVersions,
    *,
};

/// Port of `test_copy_file`.
///
/// parity: XFER-001
#[test]
fn a_copied_file_arrives_complete_and_leaves_no_staging() {
    let fixture = Fixture::new();
    let source = fixture.src.join("data.bin");
    let content = random_bytes(35_000);
    fs::write(&source, &content).unwrap();

    let result = fixture.copy(local::local(), &[&source], ConflictPolicy::Skip);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [uri(&source)]);
    assert_eq!(fs::read(fixture.dst.join("data.bin")).unwrap(), content);
    fixture.no_stage();
}

/// Ports `test_recursive_copy_includes_hidden`,
/// `test_symlink_copied_not_followed` and
/// `test_nested_symlink_loop_not_traversed`.
///
/// parity: XFER-001, XFER-005, XFER-017
#[test]
fn recursive_copy_preserves_sources_hidden_files_links_and_modes() {
    let fixture = Fixture::new();
    let folder = fixture.src.join("tree");
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
    let copied = fixture.dst.join("tree/nested");
    assert_eq!(read(&copied.join(".hidden")), "hidden content");
    assert_eq!(fs::read_link(copied.join("loop")).unwrap(), Path::new(".."));
    assert_eq!(
        fs::read_link(copied.join("dangling")).unwrap(),
        Path::new("missing")
    );
    assert_eq!(mode_of(&fixture.dst.join("tree")), 0o750);
    assert_eq!(mode_of(&copied), 0o500);
    fixture.no_stage();
    // Restore owner access so TempDir cleanup works for unprivileged users.
    set_mode(&nested, 0o700);
    set_mode(&copied, 0o700);
}

/// Ports `test_skip_never_overwrites`, `test_keep_both` and
/// `test_replace_file_after_staging_copy_completes`. Each run selects the
/// source twice, which must not make a second copy.
///
/// parity: XFER-006, XFER-008, XFER-009
#[test]
fn conflict_policies_never_overwrite_without_replace() {
    for policy in [
        ConflictPolicy::Skip,
        ConflictPolicy::KeepBoth,
        ConflictPolicy::Replace,
    ] {
        let fixture = Fixture::new();
        let source = fixture.src.join("notes.txt");
        write(&source, "new");
        write(&fixture.dst.join("notes.txt"), "old");
        let mut engine = fixture.engine(local::local());
        let result = fixture.run(&mut engine, &[&source, &source], TransferMode::Copy, policy, None);
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(read(&source), "new");
        match policy {
            ConflictPolicy::Skip => {
                assert_eq!(result.skipped, [uri(&source)]);
                assert_eq!(read(&fixture.dst.join("notes.txt")), "old");
            }
            ConflictPolicy::KeepBoth => {
                assert_eq!(result.done, [uri(&source)]);
                assert_eq!(read(&fixture.dst.join("notes.txt")), "old");
                assert_eq!(read(&fixture.dst.join("notes (copy 2).txt")), "new");
            }
            ConflictPolicy::Replace => {
                assert_eq!(result.done, [uri(&source)]);
                assert_eq!(read(&fixture.dst.join("notes.txt")), "new");
            }
        }
        assert!(fixture.leftovers().is_empty());
    }
}

/// Port of `test_duplicate_sources_deduplicated`.
///
/// parity: XFER-019
#[test]
fn a_source_selected_twice_is_copied_once() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
    write(&source, "a");

    let result = fixture.copy(local::local(), &[&source, &source], ConflictPolicy::Skip);

    assert_eq!(result.done, [uri(&source)]);
    assert!(
        result.errors.is_empty() && result.skipped.is_empty(),
        "{result:?}"
    );
    assert_eq!(list(&fixture.dst), ["a"]);
}

/// Port of `test_move_native`.
///
/// parity: XFER-011
#[test]
fn a_move_takes_the_item_out_of_its_folder() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
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
    assert_eq!(read(&fixture.dst.join("a")), "a");
}

/// Port of `test_replace_move_is_native_and_removes_source`.
///
/// parity: XFER-009, XFER-011
#[test]
fn a_move_with_replace_overwrites_the_existing_file_and_removes_the_source() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
    write(&source, "new");
    write(&fixture.dst.join("a"), "old");
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::Replace,
        None,
    );

    assert_eq!(result.done, [uri(&source)]);
    assert!(!lexists(&source));
    assert_eq!(read(&fixture.dst.join("a")), "new");
}

/// Ports `test_replace_merges_directories_and_keeps_destination_only_files`,
/// for copies and moves.
///
/// parity: XFER-009
#[test]
fn replace_merges_folders_and_retains_destination_only_children() {
    for mode in [TransferMode::Copy, TransferMode::Move] {
        let fixture = Fixture::new();
        let source = fixture.src.join("tree");
        let target = fixture.dst.join("tree");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(target.join("nested")).unwrap();
        write(&source.join("nested/shared.txt"), "new");
        write(&target.join("nested/shared.txt"), "old");
        write(&target.join("keep.txt"), "keep");
        let mut engine = fixture.engine(local::local());
        let result = fixture.run(&mut engine, &[&source], mode, ConflictPolicy::Replace, None);
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [uri(&source)]);
        assert_eq!(read(&target.join("nested/shared.txt")), "new");
        assert_eq!(read(&target.join("keep.txt")), "keep");
        assert_eq!(source.exists(), mode == TransferMode::Copy);
        fixture.no_stage();
    }
}

/// A backend that cannot overwrite in one step, like `NoDirectReplace` in
/// `desktop/tests/test_operations.py`.
struct NoDirectReplace;

impl Provider for NoDirectReplace {
    fn replace_native(
        &self,
        _node: &LocalNode,
        _target: &dyn Node,
        _cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        Err(TransferError::ReplaceUnsupported(
            "overwrite flag unsupported".into(),
        ))
    }
}

/// Port of `test_replace_falls_back_to_reversible_rename_for_remote_backend`.
///
/// parity: XFER-010
#[test]
fn replace_without_direct_overwrite_renames_reversibly_and_leaves_no_backup() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
    write(&source, "new");
    write(&fixture.dst.join("a"), "old");

    let result = fixture.copy(Arc::new(NoDirectReplace), &[&source], ConflictPolicy::Replace);

    assert_eq!(result.done, [uri(&source)]);
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.dst.join("a")), "new");
    assert_eq!(read(&source), "new");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Port of `test_replace_type_mismatch_preserves_existing_folder`, in both
/// directions.
///
/// parity: XFER-009
#[test]
fn replacement_type_mismatch_preserves_both_items() {
    for source_kind in [NodeKind::Directory, NodeKind::File] {
        let fixture = Fixture::new();
        let source = fixture.src.join("conflict");
        let destination = fixture.dst.join("conflict");
        let (folder, file) = if source_kind == NodeKind::Directory {
            (&source, &destination)
        } else {
            (&destination, &source)
        };
        fs::create_dir(folder).unwrap();
        write(&folder.join("retained"), "folder content");
        write(file, "file content");
        let mut engine = fixture.engine(local::local());
        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Copy,
            ConflictPolicy::Replace,
            None,
        );
        assert!(result.done.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("file and folder"));
        assert_eq!(read(&folder.join("retained")), "folder content");
        assert_eq!(read(file), "file content");
        fixture.no_stage();
    }
}

/// Ports `test_reject_self_descendant` and
/// `test_reject_symlink_destination_inside_source`.
///
/// parity: XFER-016
#[test]
fn self_and_descendant_destinations_are_rejected_including_symlink_aliases() {
    for mode in [TransferMode::Copy, TransferMode::Move] {
        let fixture = Fixture::new();
        let folder = fixture.src.join("tree");
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

/// Port of `test_removal_or_move_preserves_whole_tree_containing_snapshot`.
///
/// parity: XFER-020
#[test]
fn protected_descendants_stop_mutations_before_any_item_changes() {
    for mode in [TransferMode::Move, TransferMode::Trash, TransferMode::Delete] {
        let fixture = Fixture::new();
        let folder = fixture.src.join("tree");
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
        assert!(list(&fixture.dst).is_empty());
    }
}

/// A configured snapshot folder is read-only as a destination, while a copy
/// out of it is allowed.
///
/// parity: XFER-020
#[test]
fn configured_snapshot_destination_is_protected_but_restoring_a_copy_is_allowed() {
    let fixture = Fixture::new();
    let source = fixture.src.join("photo.jpg");
    write(&source, "photo");
    let versions = PreviousVersions::new();
    versions.configure(&uri(&fixture.dst), &uri(&fixture.src));
    let mut engine = fixture.engine(local::local()).with_write_guard(versions.guard());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );
    assert_eq!(result.done, [uri(&source)]);
    let reverse = fixture.run(
        &mut engine,
        &[&fixture.dst.join("photo.jpg")],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        Some(&fixture.src),
    );
    assert!(reverse.done.is_empty());
    assert!(reverse.errors[0].contains("read-only"));
    assert_eq!(read(&source), "photo");
}

/// Port of `test_trash_unsupported_no_delete`; deleting a link removes the
/// link and never its target.
///
/// parity: XFER-014, XFER-015, XFER-017
#[test]
fn delete_does_not_follow_symlinks_and_trash_never_falls_back_to_delete() {
    let fixture = Fixture::new();
    let original = fixture.src.join("original");
    write(&original, "keep");
    let link = fixture.src.join("link");
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
    let tree = fixture.src.join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    write(&tree.join("a"), "a");
    write(&tree.join("sub/b"), "b");
    let loose = fixture.src.join("loose");
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
    let source = fixture.src.join("tree");
    let mut nested = source.clone();
    for _ in 0..=MAX_DEPTH + 1 {
        nested.push("d");
    }
    fs::create_dir_all(nested).unwrap();
    let fifo = fixture.src.join("pipe");
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
    assert!(list(&fixture.dst).is_empty());
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
    let first = fixture.src.join("large");
    let later = fixture.src.join("later");
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
    assert!(list(&fixture.dst).is_empty());
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

/// A Replace merge deeper than the nesting limit stops before anything is
/// moved, even when no write guard walked the tree first.
#[test]
fn a_deep_move_merge_is_bounded_even_without_a_write_guard() {
    let fixture = Fixture::new();
    let source = fixture.src.join("tree");
    let destination = fixture.dst.join("tree");
    let mut source_nested = source.clone();
    let mut target_nested = destination.clone();
    for _ in 0..=MAX_DEPTH {
        source_nested.push("d");
        target_nested.push("d");
    }
    fs::create_dir_all(&source_nested).unwrap();
    fs::create_dir_all(&target_nested).unwrap();
    write(&source_nested.join("incoming"), "incoming");
    write(&target_nested.join("original"), "original");
    let mut engine = fixture.engine(local::local());
    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::Replace,
        None,
    );
    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("nesting"));
    assert_eq!(read(&source_nested.join("incoming")), "incoming");
    assert_eq!(read(&target_nested.join("original")), "original");
}

/// Port of `test_failure_inside_tree_leaves_source`: one special file deep
/// inside a folder fails the whole folder. Nothing is published, the stage
/// is removed and the source is untouched.
///
/// parity: XFER-018
#[test]
fn a_special_file_inside_a_folder_fails_the_whole_folder() {
    let fixture = Fixture::new();
    let tree = fixture.src.join("tree");
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
    assert!(list(&fixture.dst).is_empty());
    assert_eq!(read(&tree.join("a")), "hello");
}

/// Port of `test_cancel_before_start`: a run cancelled before it starts is
/// refused while the destination is checked, before anything is touched.
///
/// parity: OPS-022
#[test]
fn a_run_cancelled_before_it_starts_changes_nothing() {
    let fixture = Fixture::new();
    let source = fixture.src.join("a");
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
    assert!(list(&fixture.dst).is_empty());
    assert_eq!(read(&source), "a");
}
