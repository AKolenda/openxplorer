// SPDX-License-Identifier: AGPL-3.0-only
//! Replace on local files: files are overwritten and folders merged, and
//! the existing item is never lost on the way. Ports the Replace cases of
//! `TransferTests` in `desktop/tests/test_operations.py`.

use std::fs;
use std::sync::Arc;

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, TransferError, MAX_DEPTH};

use crate::transfer_support::{
    local::{self, LocalNode, Provider},
    *,
};

/// Ported from `desktop/tests/test_operations.py::TransferTests::test_replace_move_is_native_and_removes_source`.
///
/// parity: XFER-009, XFER-011
#[test]
fn a_move_with_replace_overwrites_the_existing_file_and_removes_the_source() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "new");
    write(&fixture.destination_folder.join("a"), "old");
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Replace));

    assert_eq!(result.done, [file_uri(&source)]);
    assert!(!exists_without_following_links(&source));
    assert_eq!(read(&fixture.destination_folder.join("a")), "new");
}

/// Ports `test_replace_merges_directories_and_keeps_destination_only_files`,
/// for copies and moves.
///
/// parity: XFER-009
#[test]
fn replace_merges_folders_and_retains_destination_only_children() {
    for request in [
        Request::Copy(ConflictPolicy::Replace),
        Request::Move(ConflictPolicy::Replace),
    ] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("tree");
        let target = fixture.destination_folder.join("tree");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(target.join("nested")).unwrap();
        write(&source.join("nested/shared.txt"), "new");
        write(&target.join("nested/shared.txt"), "old");
        write(&target.join("keep.txt"), "keep");
        let mut engine = fixture.engine(local::local());

        let result = fixture.run(&mut engine, &[&source], request);

        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [file_uri(&source)]);
        assert_eq!(read(&target.join("nested/shared.txt")), "new");
        assert_eq!(read(&target.join("keep.txt")), "keep");
        let is_copy = matches!(request, Request::Copy(_));
        assert_eq!(source.exists(), is_copy, "{request:?}");
        fixture.assert_no_staging();
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

/// Ported from `desktop/tests/test_operations.py::TransferTests::test_replace_falls_back_to_reversible_rename_for_remote_backend`.
///
/// parity: XFER-010
#[test]
fn replace_without_direct_overwrite_renames_reversibly_and_leaves_no_backup() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "new");
    write(&fixture.destination_folder.join("a"), "old");

    let result = fixture.copy(Arc::new(NoDirectReplace), &[&source], ConflictPolicy::Replace);

    assert_eq!(result.done, [file_uri(&source)]);
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("a")), "new");
    assert_eq!(read(&source), "new");
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
}

/// Ported from `desktop/tests/test_operations.py::TransferTests::test_replace_type_mismatch_preserves_existing_folder`, in both
/// directions.
///
/// parity: XFER-009
#[test]
fn replacement_type_mismatch_preserves_both_items() {
    for source_kind in [NodeKind::Directory, NodeKind::File] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("conflict");
        let destination = fixture.destination_folder.join("conflict");
        let (folder, file) = if source_kind == NodeKind::Directory {
            (&source, &destination)
        } else {
            (&destination, &source)
        };
        fs::create_dir(folder).unwrap();
        write(&folder.join("retained"), "folder content");
        write(file, "file content");
        let mut engine = fixture.engine(local::local());

        let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

        assert!(result.done.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("file and folder"));
        assert_eq!(read(&folder.join("retained")), "folder content");
        assert_eq!(read(file), "file content");
        fixture.assert_no_staging();
    }
}

/// A Replace merge deeper than the nesting limit stops before anything is
/// moved, even when no write guard walked the tree first.
#[test]
fn a_deep_move_merge_is_bounded_even_without_a_write_guard() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("tree");
    let destination = fixture.destination_folder.join("tree");
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

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Replace));

    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("nesting"));
    assert_eq!(read(&source_nested.join("incoming")), "incoming");
    assert_eq!(read(&target_nested.join("original")), "original");
}
