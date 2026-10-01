// SPDX-License-Identifier: AGPL-3.0-only
//! Folder-size scans of real temporary folders: what a scan counts and what
//! it leaves out (PROP-026, PROP-028).
//!
//! Ports the totals and exclusions of `FolderSizeTests` in
//! `v2.0.0:desktop/tests/test_v06.py`. Its limits, cancellation, progress and the
//! refusals and errors of the scanned folder are in `sizes_scan_limits.rs`;
//! `test_max_seconds`, which needs a simulated clock, is a unit test in
//! `sizes::scan`. Where a Python test subclasses `LocalSizeProvider`, the
//! test here uses a `TestProvider` from `sizes_support`, which reads
//! through the real local provider unless the test changes how folders are
//! listed. Every file is inside a temporary directory.

mod sizes_support;

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::process::Command;

use ox_core::entry::EntryError;
use ox_core::location::file_uri;
use ox_core::sizes::{LocalSizeProvider, ScanStatus, SizeEntry, SizeEntryKind};
use sizes_support::{scan, simulated_entry, Folder, Listing, TestProvider, EXCLUDED};

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_empty_is_real_zero`
///
/// parity: PROP-026, PROP-028
#[test]
fn an_empty_folder_is_a_real_zero() {
    let folder = Folder::new();

    let size = folder.scan();

    assert_eq!((size.bytes, size.status), (0, ScanStatus::Complete));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_recursive_file_bytes`
///
/// parity: PROP-026, PROP-028
#[test]
fn file_bytes_are_totalled_recursively() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    folder.write("sub/b", b"12345");

    let size = folder.scan();

    assert_eq!((size.bytes, size.files, size.folders), (8, 2, 1));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_hidden_files_counted`
///
/// parity: PROP-026, PROP-028
#[test]
fn hidden_files_are_counted() {
    let folder = Folder::new();
    folder.write(".hidden/private", b"1234");

    assert_eq!(folder.scan().bytes, 4);
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_files_unchanged`
///
/// parity: PROP-026, PROP-028
#[test]
fn scanning_leaves_files_unchanged() {
    let folder = Folder::new();
    let path = folder.write("a", b"abc");
    let before = fs::metadata(&path).unwrap();

    folder.scan();

    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (before.len(), before.mtime(), before.mtime_nsec()),
        (after.len(), after.mtime(), after.mtime_nsec())
    );
    assert_eq!(fs::read(&path).unwrap(), b"abc");
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_symbolic_link_not_followed`
///
/// parity: PROP-028
#[test]
fn a_symbolic_link_is_not_followed() {
    let folder = Folder::new();
    let actual = folder.write("actual", b"abc");
    symlink(&actual, folder.path().join("link")).unwrap();

    let size = folder.scan();

    assert_eq!((size.bytes, size.skipped, size.status), (3, 1, EXCLUDED));
    assert_eq!(
        size.status.reason(),
        "Some links, mounts, snapshot collections or unreadable entries were excluded"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_cycle_does_not_recurse`
///
/// parity: PROP-028
#[test]
fn a_link_cycle_does_not_recurse() {
    let folder = Folder::new();
    folder.write("sub/a", b"xyz");
    symlink(folder.path(), folder.path().join("sub/loop")).unwrap();

    let size = folder.scan();

    assert_eq!(size.bytes, 3);
    assert_eq!(size.folders, 1);
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_snapshot_collection_excluded`
///
/// parity: PROP-028
#[test]
fn a_snapshot_collection_is_excluded() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    folder.write(".zfs/snapshot/a/secret", b"not duplicated");

    let size = folder.scan();

    assert_eq!(size.bytes, 3);
    assert_eq!(size.status, EXCLUDED);
}

/// Every snapshot collection name of `EXCLUDED` in `folder_sizes.py` is
/// left out, not only `.zfs`.
///
/// parity: PROP-028
#[test]
fn every_snapshot_collection_name_is_excluded() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    for collection in [".zfs", ".snapshot", "#snapshot", ".snapshots"] {
        folder.write(&format!("{collection}/daily/copy"), b"not duplicated");
    }

    let size = folder.scan();

    assert_eq!((size.bytes, size.folders, size.skipped), (3, 0, 4));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_explicit_snapshot_root_can_be_scanned`
///
/// parity: PROP-028
#[test]
fn an_explicitly_chosen_snapshot_folder_can_be_scanned() {
    let folder = Folder::new();
    let file = folder.write(".zfs/snapshot/dated/a", b"abc");
    let snapshot = file.parent().expect("the file is in a snapshot");

    let size = scan(&file_uri(snapshot)).expect("the scan starts");

    assert_eq!(size.bytes, 3);
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_hardlinks_count_once`
///
/// parity: PROP-028
#[test]
fn hard_links_count_once() {
    let folder = Folder::new();
    let file = folder.write("a", b"abc");
    fs::hard_link(&file, folder.path().join("b")).unwrap();

    let size = folder.scan();

    assert_eq!((size.bytes, size.files), (3, 1));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_fifo_excluded_not_opened`
///
/// parity: PROP-028
#[test]
fn a_fifo_is_excluded_and_never_opened() {
    let folder = Folder::new();
    let status = Command::new("mkfifo")
        .arg(folder.path().join("pipe"))
        .status()
        .expect("mkfifo (GNU coreutils) is required for the FIFO test");
    assert!(status.success(), "mkfifo failed: {status}");

    let size = folder.scan();

    assert_eq!((size.bytes, size.skipped), (0, 1));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_nested_mount_excluded`
///
/// parity: PROP-028
#[test]
fn a_nested_mount_is_excluded() {
    let folder = Folder::new();
    folder.write("other/a", b"abc");
    let provider = LocalSizeProvider::with_mount_points([folder.path().join("other")]);

    let size = folder.scan_through(&provider).unwrap();

    assert_eq!((size.bytes, size.status), (0, EXCLUDED));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_unreadable_entries_reported`
///
/// parity: PROP-028
#[test]
fn unreadable_entries_are_reported_as_errors() {
    let folder = Folder::new();
    let provider = TestProvider::listing(Listing::Only(vec![SizeEntry::unreadable("unreadable")]));

    let size = folder.scan_through(&provider).unwrap();

    assert_eq!((size.errors, size.status), (1, EXCLUDED));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_failed_subtree_reported`
///
/// parity: PROP-028
#[test]
fn a_failed_subfolder_is_reported_and_not_shown_as_empty() {
    let folder = Folder::new();
    folder.write("sub/a", b"abc");
    let provider = TestProvider::listing(Listing::FailFolder {
        suffix: "/sub",
        error: EntryError::PermissionDenied("test refusal".into()),
    });

    let size = folder.scan_through(&provider).unwrap();

    assert_eq!((size.errors, size.status), (1, EXCLUDED));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_unknown_size_is_not_fabricated_zero`
///
/// parity: PROP-027, PROP-028
#[test]
fn an_unknown_size_is_not_counted_as_zero() {
    let folder = Folder::new();
    let unknown_size = simulated_entry("file:///unknown", SizeEntryKind::File);
    let provider = TestProvider::listing(Listing::Only(vec![unknown_size]));

    let size = folder.scan_through(&provider).unwrap();

    assert_eq!(size.status, EXCLUDED);
    assert_eq!((size.files, size.skipped), (0, 1));
}

/// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_spaces_in_path`
///
/// parity: PROP-026
#[test]
fn a_folder_with_spaces_and_a_hash_in_its_name_is_scanned() {
    let folder = Folder::new();
    let file = folder.write("spaces and #/file.txt", b"abc");
    let named = file.parent().expect("the file is in the named folder");

    let size = scan(&file_uri(named)).unwrap();

    assert_eq!(size.bytes, 3);
}
