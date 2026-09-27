// SPDX-License-Identifier: AGPL-3.0-only
//! Local copy, move, deletion and preflight invariants.

use std::fs;
use std::os::unix::fs::symlink;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{ConflictPolicy, TransferMode, MAX_DEPTH};

use crate::transfer_support::{local, versions::PreviousVersions, *};

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
    assert_eq!(
        fs::read_link(copied.join("loop")).unwrap(),
        std::path::Path::new("..")
    );
    assert_eq!(
        fs::read_link(copied.join("dangling")).unwrap(),
        std::path::Path::new("missing")
    );
    assert_eq!(mode_of(&fixture.dst.join("tree")), 0o750);
    assert_eq!(mode_of(&copied), 0o500);
    fixture.no_stage();
    // Restore owner access so TempDir cleanup works for unprivileged users.
    set_mode(&nested, 0o700);
    set_mode(&copied, 0o700);
}

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

#[test]
fn replacement_type_mismatch_preserves_both_items() {
    for source_is_directory in [true, false] {
        let fixture = Fixture::new();
        let source = fixture.src.join("conflict");
        let destination = fixture.dst.join("conflict");
        let (folder, file) = if source_is_directory {
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
}

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
    assert_eq!(
        updates
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.label.starts_with("Copying "))
            .count(),
        1
    );
}

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
