// SPDX-License-Identifier: AGPL-3.0-only
//! Snapshot and backup locations are read-only (PROP-024), through the
//! transfer engine too (XFER-020).
//!
//! Ports `test_snapshot_guard` from `v2.0.0:desktop/tests/test_v05.py` and runs
//! the cases of `ProtectedTransferTests` in
//! `v2.0.0:desktop/tests/test_operations.py` and of
//! `test_recursive_replace_and_delete_preserve_backup_descendant` in
//! `v2.0.0:desktop/tests/gio_integration.py` with the real
//! [`PreviousVersions::write_guard`] and the GIO engine; the transfer
//! tests in `transfer_cases/snapshots.rs` run them with a test double of
//! the guard. The read-only rule is also compared with
//! `PreviousVersions.protected` in `v2.0.0:desktop/previous_versions.py`. Every
//! file is inside a temporary directory.

mod python_support;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::location::file_uri;
use ox_core::transfer::{Cancellation, ConflictPolicy, Node, Operation, TransferEngine, TransferResult};
use ox_core::versions::{PreviousVersions, SnapshotLayout, VersionsError};
use python_support::run_python;
use tempfile::TempDir;

/// Prints whether the Python app protects each URI read from standard
/// arguments after the settings directory `sys.argv[1]`, as JSON.
const PYTHON_PRINTS_PROTECTION: &str = r"
import json, sys
from pathlib import Path
from previous_versions import PreviousVersions
versions = PreviousVersions(Path(sys.argv[1]))
print(json.dumps([versions.protected(uri) for uri in sys.argv[2:]]))
";

/// The read-only message of the Python app.
const READ_ONLY: &str =
    "Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first.";

/// Source and destination folders and the service guarding them.
struct Fixture {
    temporary: TempDir,
    source_folder: PathBuf,
    destination_folder: PathBuf,
    versions: Arc<PreviousVersions>,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let source_folder = temporary.path().join("source");
        let destination_folder = temporary.path().join("destination");
        fs::create_dir(&source_folder).unwrap();
        fs::create_dir(&destination_folder).unwrap();
        let versions = Arc::new(PreviousVersions::new(&temporary.path().join("settings")));
        Self {
            temporary,
            source_folder,
            destination_folder,
            versions,
        }
    }

    /// Runs `operation` on `items` with the GIO engine and the real guard.
    fn run(&self, operation: Operation<'_>, items: &[&Path]) -> TransferResult {
        let factory = Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>));
        let mut engine = TransferEngine::new(factory).with_write_guard(self.versions.write_guard());
        let uris: Vec<String> = items.iter().map(|path| file_uri(path)).collect();
        engine
            .run(operation, &uris, &Cancellation::new())
            .expect("the request is accepted")
    }

    /// Copies `items` into `folder`.
    fn copy_into(&self, items: &[&Path], folder: &Path, policy: ConflictPolicy) -> TransferResult {
        let destination_folder = file_uri(folder);
        let operation = Operation::Copy {
            destination_folder: &destination_folder,
            policy,
        };
        self.run(operation, items)
    }

    /// No staging folder or other leftover next to the destination items.
    fn assert_only_items_in_destination(&self, names: &[&str]) {
        let mut found: Vec<String> = fs::read_dir(&self.destination_folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, names);
    }
}

/// Creates `folder/ordinary.txt` and `folder/.snapshot/version.txt`, both
/// holding `contents`.
fn folder_with_snapshot(folder: &Path, contents: &str) {
    fs::create_dir_all(folder.join(".snapshot")).unwrap();
    fs::write(folder.join("ordinary.txt"), contents).unwrap();
    fs::write(folder.join(".snapshot/version.txt"), contents).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_snapshot_guard`
///
/// parity: PROP-024, XFER-020
#[test]
fn a_location_inside_a_snapshot_folder_is_refused_as_read_only() {
    let temporary = tempfile::tempdir().unwrap();
    let versions = PreviousVersions::new(temporary.path());

    let refusal = versions.check_writable("smb://nas/share/.snapshot/old/file");

    let refusal = refusal.unwrap_err();
    assert!(matches!(refusal, VersionsError::ReadOnly));
    assert_eq!(refusal.to_string(), READ_ONLY);
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::ProtectedTransferTests::test_replace_cannot_overwrite_nested_snapshot`
///
/// parity: XFER-020, PROP-024
#[test]
fn replace_cannot_overwrite_nested_snapshot() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    let target = fixture.destination_folder.join("project");
    folder_with_snapshot(&source, "incoming");
    folder_with_snapshot(&target, "original");

    let result = fixture.copy_into(&[&source], &fixture.destination_folder, ConflictPolicy::Replace);

    assert!(!result.errors.is_empty(), "{result:?}");
    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(read(&target.join("ordinary.txt")), "original");
    assert_eq!(read(&target.join(".snapshot/version.txt")), "original");
    fixture.assert_only_items_in_destination(&["project"]);
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::ProtectedTransferTests::test_removal_or_move_preserves_whole_tree_containing_snapshot`
///
/// parity: XFER-020, PROP-024
#[test]
fn removal_or_move_preserves_whole_tree_containing_snapshot() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    folder_with_snapshot(&source, "keep");
    let destination_folder = file_uri(&fixture.destination_folder);
    let operations = [
        Operation::Delete,
        Operation::Trash,
        Operation::Move {
            destination_folder: &destination_folder,
            policy: ConflictPolicy::Skip,
        },
    ];
    for operation in operations {
        let result = fixture.run(operation, &[&source]);

        assert!(
            result.errors[0].contains("read-only"),
            "{operation:?}: {result:?}"
        );
        assert!(result.done.is_empty(), "{operation:?}: {result:?}");
        assert_eq!(read(&source.join("ordinary.txt")), "keep");
        assert_eq!(read(&source.join(".snapshot/version.txt")), "keep");
        assert!(!fixture.destination_folder.join("project").exists());
    }
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::ProtectedTransferTests::test_configured_backup_descendant_is_protected`
///
/// parity: XFER-020, PROP-024, PROP-023
#[test]
fn configured_backup_descendant_is_protected() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    let backup = source.join("history");
    fs::create_dir_all(&backup).unwrap();
    fs::write(backup.join("version.txt"), "backup").unwrap();
    fixture
        .versions
        .configure(&file_uri(&source), &file_uri(&backup), SnapshotLayout::Direct)
        .unwrap();

    let result = fixture.run(Operation::Delete, &[&source]);

    assert!(!result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&backup.join("version.txt")), "backup");
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::ProtectedTransferTests::test_snapshot_file_can_be_restored_to_another_folder`
///
/// parity: XFER-020, PROP-025
#[test]
fn snapshot_file_can_be_restored_to_another_folder() {
    let fixture = Fixture::new();
    let snapshot = fixture.source_folder.join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    let saved = snapshot.join("document.txt");
    fs::write(&saved, "saved").unwrap();

    let result = fixture.copy_into(&[&saved], &fixture.destination_folder, ConflictPolicy::Skip);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("document.txt")), "saved");
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::ProtectedTransferTests::test_symlink_to_snapshot_is_removed_without_traversal`
///
/// parity: XFER-020
#[test]
fn symlink_to_snapshot_is_removed_without_traversal() {
    let fixture = Fixture::new();
    let snapshot = fixture.source_folder.join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    fs::write(snapshot.join("version.txt"), "backup").unwrap();
    let link = fixture.source_folder.join("shortcut");
    symlink(&snapshot, &link).unwrap();

    let result = fixture.run(Operation::Delete, &[&link]);

    assert!(result.errors.is_empty(), "{result:?}");
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(read(&snapshot.join("version.txt")), "backup");
}

/// Ported from `v2.0.0:desktop/tests/gio_integration.py::GioLocalIntegration::test_recursive_replace_and_delete_preserve_backup_descendant`
///
/// parity: XFER-020
#[test]
fn recursive_replace_and_delete_preserve_backup_descendant() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("project");
    let target = fixture.destination_folder.join("project");
    folder_with_snapshot(&source, "incoming");
    folder_with_snapshot(&target, "backup");

    let replaced = fixture.copy_into(&[&source], &fixture.destination_folder, ConflictPolicy::Replace);
    let removed = fixture.run(Operation::Delete, &[&target]);

    assert!(!replaced.errors.is_empty(), "{replaced:?}");
    assert!(!removed.errors.is_empty(), "{removed:?}");
    assert_eq!(read(&target.join(".snapshot/version.txt")), "backup");
}

/// Copying into a configured backup folder is refused, while restoring a
/// copy out of it is allowed.
///
/// parity: XFER-020, PROP-024, PROP-025
#[test]
fn a_configured_backup_folder_can_be_copied_from_but_not_into() {
    let fixture = Fixture::new();
    let backup = &fixture.source_folder;
    let saved = backup.join("photo.jpg");
    fs::write(&saved, "saved version").unwrap();
    let live = fixture.destination_folder.join("photo.jpg");
    fs::write(&live, "current version").unwrap();
    fixture
        .versions
        .configure(
            &file_uri(&fixture.destination_folder),
            &file_uri(backup),
            SnapshotLayout::Direct,
        )
        .unwrap();

    let into_backup = fixture.copy_into(&[&live], backup, ConflictPolicy::Replace);
    let restored = fixture.copy_into(&[&saved], &fixture.destination_folder, ConflictPolicy::KeepBoth);

    assert!(into_backup.errors[0].contains(READ_ONLY), "{into_backup:?}");
    assert_eq!(read(&saved), "saved version");
    assert!(restored.errors.is_empty(), "{restored:?}");
    assert_eq!(read(&live), "current version");
    assert_eq!(
        read(&fixture.destination_folder.join("photo - Copy.jpg")),
        "saved version"
    );
}

/// "Restore a copy" (`restoreVersion` in `v2.0.0:desktop/ui/app.js`) refuses a
/// destination inside a snapshot or backup folder before anything is
/// copied, and copies into a live folder with Keep both.
///
/// parity: PROP-025
#[test]
fn a_version_is_restored_as_a_copy_only_outside_snapshots() {
    let fixture = Fixture::new();
    let snapshot = fixture.source_folder.join(".snapshot/monday");
    fs::create_dir_all(&snapshot).unwrap();
    let version = snapshot.join("report.txt");
    fs::write(&version, "monday").unwrap();
    let live = fixture.destination_folder.join("report.txt");
    fs::write(&live, "today").unwrap();

    let into_snapshot = fixture.versions.restore_destination(&file_uri(&snapshot));
    let destination = fixture
        .versions
        .restore_destination(&fixture.destination_folder.to_string_lossy())
        .expect("a live folder is a valid destination");
    let restored = fixture.run(
        Operation::Copy {
            destination_folder: &destination,
            policy: ConflictPolicy::KeepBoth,
        },
        &[&version],
    );

    let refusal = into_snapshot.unwrap_err();
    assert!(matches!(refusal, VersionsError::RestoreIntoSnapshot));
    assert_eq!(
        refusal.to_string(),
        "Choose a folder outside the snapshot collection."
    );
    assert_eq!(destination, file_uri(&fixture.destination_folder));
    assert!(restored.errors.is_empty(), "{restored:?}");
    assert_eq!(read(&live), "today");
    assert_eq!(read(&version), "monday");
    assert_eq!(
        read(&fixture.destination_folder.join("report - Copy.txt")),
        "monday"
    );
}

/// The read-only rule agrees with `PreviousVersions.protected` in the
/// Python app for conventional markers, encoded markers, configured roots
/// and look-alike names.
///
/// parity: PROP-021, PROP-024
#[test]
fn locations_are_protected_exactly_as_in_the_python_app() {
    let temporary = tempfile::tempdir().unwrap();
    let settings = temporary.path().join("winspace");
    let versions = PreviousVersions::new(&settings);
    versions
        .configure("smb://nas/share", "smb://nas/backup", SnapshotLayout::Direct)
        .unwrap();
    let uris = [
        "smb://nas/share/.snapshot/old/file",
        "smb://nas/share/%23snapshot/manual",
        "smb://nas/share/%2Esnapshots/1/snapshot/a",
        "smb://nas/share/@GMT-2026.09.05-18.00.00/a",
        "file:///tank/.zfs/snapshot/daily/a",
        "file:///tank/.zfs/snapshots-not/a",
        "smb://nas/backup",
        "smb://nas/backup/nightly/a",
        "smb://nas/backup-old/a",
        "smb://nas/share/folder.snapshot",
        "file:///home/demo/Documents",
    ];

    let from_rust: Vec<bool> = uris
        .iter()
        .map(|uri| matches!(versions.check_writable(uri), Err(VersionsError::ReadOnly)))
        .collect();
    let printed = run_python(PYTHON_PRINTS_PROTECTION, &python_arguments(&settings, &uris));
    let from_python: Vec<bool> = serde_json::from_str(&printed).expect("Python printed JSON");

    assert_eq!(from_rust, from_python);
    assert_eq!(
        from_rust,
        [true, true, true, true, true, false, true, true, false, false, false]
    );
}

/// The settings directory followed by `uris`, as the Python script reads
/// its arguments.
fn python_arguments<'a>(settings: &'a Path, uris: &'a [&'a str]) -> Vec<&'a Path> {
    let mut arguments = vec![settings];
    arguments.extend(uris.iter().map(Path::new));
    arguments
}

/// One read of the sources marks a whole listing, as Python's `annotate`
/// does for each batch of rows.
///
/// parity: PROP-024
#[test]
fn listing_rows_are_marked_read_only_with_one_snapshot_of_the_rule() {
    let fixture = Fixture::new();
    let backup = fixture.temporary.path().join("backup");
    fixture
        .versions
        .configure(
            &file_uri(&fixture.source_folder),
            &file_uri(&backup),
            SnapshotLayout::Direct,
        )
        .unwrap();
    let rows = [
        file_uri(&backup.join("monday")),
        file_uri(&fixture.source_folder.join(".snapshots")),
        file_uri(&fixture.source_folder.join("report.pdf")),
    ];

    let protected = fixture.versions.protected_locations();

    let read_only: Vec<bool> = rows.iter().map(|uri| protected.is_protected(uri)).collect();
    assert_eq!(read_only, [true, true, false]);
}
