// SPDX-License-Identifier: AGPL-3.0-only
//! The engine over the production GIO adapter, on temporary local files.
//! Ports the engine cases of `GioLocalIntegration` in
//! `desktop/tests/gio_integration.py`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{ConflictPolicy, Node, TransferEngine, TransferMode};

use crate::transfer_support::{versions::PreviousVersions, *};

/// An engine resolving every URI with [`GioNode`].
fn gio_engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri: &str| {
        Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>)
    }))
}

/// Gives `path` owner access again, so the temporary folder can be removed
/// even after a test failed midway.
struct RestoreOwnerAccess<'a>(&'a Path);

impl Drop for RestoreOwnerAccess<'_> {
    fn drop(&mut self) {
        if lexists(self.0) {
            set_mode(self.0, 0o700);
        }
    }
}

/// Port of `test_keep_both`.
///
/// parity: XFER-008
#[test]
fn keep_both_through_gio_adds_a_copy_name() {
    let fixture = Fixture::new();
    let source = fixture.src.join("payload.txt");
    write(&source, "new");
    write(&fixture.dst.join("payload.txt"), "old");

    let result = fixture.run(
        &mut gio_engine(),
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::KeepBoth,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.dst.join("payload.txt")), "old");
    assert_eq!(read(&fixture.dst.join("payload (copy 2).txt")), "new");
    fixture.no_stage();
}

/// Port of `test_replace_existing_file`.
///
/// parity: XFER-009
#[test]
fn replace_through_gio_overwrites_the_existing_file() {
    let fixture = Fixture::new();
    let source = fixture.src.join("payload.txt");
    write(&source, "new");
    write(&fixture.dst.join("payload.txt"), "old");

    let result = fixture.run(
        &mut gio_engine(),
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(list(&fixture.dst), ["payload.txt"]);
    assert_eq!(read(&fixture.dst.join("payload.txt")), "new");
    assert_eq!(read(&source), "new");
}

/// Port of `test_copy_preserves_private_directory_modes` and
/// `test_copy_read_only_directory_preserves_mode`.
///
/// parity: XFER-005
#[test]
fn copied_folders_keep_private_and_read_only_modes() {
    let fixture = Fixture::new();
    let private = fixture.src.join("private");
    let restricted = private.join("restricted");
    fs::create_dir_all(&restricted).expect("create the folders");
    write(
        &restricted.join("payload.txt"),
        "private through parent permissions",
    );
    set_mode(&private, 0o700);
    set_mode(&restricted, 0o750);
    let read_only = fixture.src.join("read-only");
    fs::create_dir(&read_only).expect("create the read-only folder");
    write(&read_only.join("payload"), "contents");
    set_mode(&read_only, 0o500);
    let _source_access = RestoreOwnerAccess(&read_only);
    let copied_read_only = fixture.dst.join("read-only");
    let _copy_access = RestoreOwnerAccess(&copied_read_only);

    let result = fixture.run(
        &mut gio_engine(),
        &[&private, &read_only],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(mode_of(&fixture.dst.join("private")), 0o700);
    assert_eq!(mode_of(&fixture.dst.join("private/restricted")), 0o750);
    let copied_payload = fixture.dst.join("private/restricted/payload.txt");
    assert_eq!(read(&copied_payload), "private through parent permissions");
    assert_eq!(mode_of(&copied_read_only), 0o500);
    assert_eq!(read(&copied_read_only.join("payload")), "contents");
    fixture.no_stage();
}

/// Port of `test_merge_read_only_source_keeps_existing_destination_permissions`.
///
/// parity: XFER-005, XFER-009
#[test]
fn merging_a_read_only_folder_keeps_the_destination_folders_mode() {
    let fixture = Fixture::new();
    let source = fixture.src.join("project");
    fs::create_dir(&source).expect("create the source folder");
    write(&source.join("incoming"), "new");
    set_mode(&source, 0o500);
    let _source_access = RestoreOwnerAccess(&source);
    let target = fixture.dst.join("project");
    fs::create_dir(&target).expect("create the existing folder");
    set_mode(&target, 0o700);
    write(&target.join("keep"), "existing");

    let result = fixture.run(
        &mut gio_engine(),
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(mode_of(&target), 0o700);
    assert_eq!(read(&target.join("incoming")), "new");
    assert_eq!(read(&target.join("keep")), "existing");
    fixture.no_stage();
}

/// Port of `test_failed_publish_cleans_restricted_staging_tree`. Another
/// program takes the final name while the copy runs, so publishing fails
/// after the staged folder got its read-only mode back; cleanup must still
/// remove the whole staging tree.
///
/// parity: XFER-002, XFER-007
#[test]
fn a_failed_publish_removes_staging_that_holds_a_read_only_folder() {
    let fixture = Fixture::new();
    let source = fixture.src.join("project");
    fs::create_dir(&source).expect("create the source folder");
    write(&source.join("payload"), "contents");
    set_mode(&source, 0o500);
    let _source_access = RestoreOwnerAccess(&source);
    let racer = fixture.dst.join("project");
    let racer_path = racer.clone();
    let mut engine = gio_engine().with_progress(move |progress| {
        if progress.label.starts_with("Copying ") && !lexists(&racer_path) {
            write(&racer_path, "another program");
        }
    });

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(list(&fixture.dst), ["project"]);
    assert_eq!(read(&racer), "another program");
    assert_eq!(read(&source.join("payload")), "contents");
}

/// Port of `test_recursive_replace_and_delete_preserve_backup_descendant`,
/// which is also the engine half of
/// `test_operate_dispatch_protects_backup_descendants` in
/// `desktop/tests/test_rc2.py` (its bridge dispatch has no native port yet).
///
/// parity: XFER-015, XFER-020
#[test]
fn replace_and_delete_through_gio_keep_a_snapshot_inside_the_folder() {
    let fixture = Fixture::new();
    let source = fixture.src.join("project");
    let target = fixture.dst.join("project");
    for (folder, content) in [(&source, "incoming"), (&target, "backup")] {
        fs::create_dir_all(folder.join(".snapshot")).expect("create the snapshot folder");
        write(&folder.join(".snapshot/version.txt"), content);
    }
    let versions = PreviousVersions::new();
    let mut engine = gio_engine().with_write_guard(versions.guard());

    let replaced = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Replace,
        None,
    );
    let removed = fixture.run(
        &mut engine,
        &[&target],
        TransferMode::Delete,
        ConflictPolicy::Skip,
        None,
    );

    assert!(replaced.errors[0].contains("read-only"), "{replaced:?}");
    assert!(removed.errors[0].contains("read-only"), "{removed:?}");
    assert_eq!(read(&target.join(".snapshot/version.txt")), "backup");
}
