// SPDX-License-Identifier: AGPL-3.0-only
//! Folder-size scans of real temporary folders (PROP-026 to PROP-029).
//!
//! Ports `FolderSizeTests` in `desktop/tests/test_v06.py`, except
//! `test_max_seconds`, which needs a simulated clock and is a unit test in
//! `sizes::scan`. Where a Python test subclasses `LocalSizeProvider`, the
//! test here uses a [`TestProvider`], which reads through the real local
//! provider unless the test changes how folders are listed. Every file is
//! inside a temporary directory.

mod sizes_support;

use std::fs;
use std::ops::ControlFlow;
use std::os::unix::fs::{symlink, MetadataExt};
use std::process::Command;
use std::time::Duration;

use ox_core::entry::EntryError;
use ox_core::location::file_uri;
use ox_core::sizes::{
    scan_folder_size, FolderSizeScan, LocalSizeProvider, PartialReason, ScanLimits, ScanStatus, SizeEntry,
    SizeEntryKind, SizeError, SizeProvider, MAX_DURATION,
};
use ox_core::transfer::Cancellation;
use sizes_support::{scan, simulated_entry, visit_each, Folder, EXCLUDED};

/// How a [`TestProvider`] lists folders. Each variant stands in for one of
/// the `LocalSizeProvider` subclasses of the Python tests.
#[derive(Debug)]
enum Listing {
    /// The real listing.
    Real,
    /// The real listing, cancelling the scan once this many items were
    /// passed on.
    CancelAfter(usize),
    /// These items, whatever the folder.
    Only(Vec<SizeEntry>),
    /// The real listing, except that the folder whose URI ends with
    /// `suffix` fails with `error`.
    FailFolder { suffix: &'static str, error: EntryError },
    /// Every listing fails with this error.
    FailAlways(EntryError),
}

/// Reads through a real [`LocalSizeProvider`], except where `listing` or
/// `inspect_error` say otherwise.
#[derive(Debug)]
struct TestProvider {
    local: LocalSizeProvider,
    listing: Listing,
    /// Returned instead of the scanned folder's metadata.
    inspect_error: Option<EntryError>,
}

impl TestProvider {
    fn listing(listing: Listing) -> Self {
        Self {
            local: LocalSizeProvider::new().expect("the mount table is readable"),
            listing,
            inspect_error: None,
        }
    }

    fn failing_inspect(error: EntryError) -> Self {
        Self {
            inspect_error: Some(error),
            ..Self::listing(Listing::Real)
        }
    }

    /// The real listing of `folder_uri`, cancelling the scan once `limit`
    /// items were passed on.
    fn visit_then_cancel(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
        limit: usize,
    ) -> Result<(), EntryError> {
        let mut passed_on = 0;
        self.local.visit_children(folder_uri, cancel, &mut |entry| {
            let flow = visit(entry);
            passed_on += 1;
            if passed_on == limit {
                cancel.cancel();
            }
            flow
        })
    }
}

impl SizeProvider for TestProvider {
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<SizeEntry, EntryError> {
        match &self.inspect_error {
            Some(error) => Err(error.clone()),
            None => self.local.inspect(uri, cancel),
        }
    }

    fn visit_children(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError> {
        match &self.listing {
            Listing::Real => self.local.visit_children(folder_uri, cancel, visit),
            Listing::CancelAfter(limit) => self.visit_then_cancel(folder_uri, cancel, visit, *limit),
            Listing::Only(entries) => {
                visit_each(entries, visit);
                Ok(())
            }
            Listing::FailFolder { suffix, error } => {
                if folder_uri.ends_with(suffix) {
                    return Err(error.clone());
                }
                self.local.visit_children(folder_uri, cancel, visit)
            }
            Listing::FailAlways(error) => Err(error.clone()),
        }
    }
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_empty_is_real_zero`
///
/// parity: PROP-026, PROP-028
#[test]
fn an_empty_folder_is_a_real_zero() {
    let folder = Folder::new();

    let size = folder.scan();

    assert_eq!((size.bytes, size.status), (0, ScanStatus::Complete));
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_recursive_file_bytes`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_hidden_files_counted`
///
/// parity: PROP-026, PROP-028
#[test]
fn hidden_files_are_counted() {
    let folder = Folder::new();
    folder.write(".hidden/private", b"1234");

    assert_eq!(folder.scan().bytes, 4);
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_files_unchanged`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_symbolic_link_not_followed`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_cycle_does_not_recurse`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_root_symlink_refused`
///
/// parity: PROP-028, PROP-029
#[test]
fn a_link_to_a_folder_is_refused_as_the_scanned_folder() {
    let folder = Folder::new();
    let target = folder.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = folder.path().join("link");
    symlink(&target, &link).unwrap();

    let refusal = scan(&file_uri(&link)).unwrap_err();

    assert_eq!(refusal, SizeError::NotAFolder);
    assert_eq!(
        refusal.to_string(),
        "Select a directory, not a file or symbolic link."
    );
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_regular_file_refused`
///
/// parity: PROP-028, PROP-029
#[test]
fn a_file_is_refused_as_the_scanned_folder() {
    let folder = Folder::new();
    let file = folder.write("a", b"x");

    assert_eq!(scan(&file_uri(&file)), Err(SizeError::NotAFolder));
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_snapshot_collection_excluded`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_explicit_snapshot_root_can_be_scanned`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_hardlinks_count_once`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_fifo_excluded_not_opened`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_entry_limit_marks_partial`
///
/// parity: PROP-027, PROP-029
#[test]
fn the_entry_limit_marks_the_result_partial() {
    let folder = Folder::new();
    for name in ["0", "1", "2", "3", "4"] {
        folder.write(name, b"x");
    }
    let provider = LocalSizeProvider::new().unwrap();
    let limits = ScanLimits::new(2, MAX_DURATION).unwrap();

    let size = FolderSizeScan::new(&provider)
        .with_limits(limits)
        .run(&folder.uri(), &Cancellation::new(), |_| {})
        .unwrap();

    assert_eq!(
        (size.bytes, size.status),
        (2, ScanStatus::Partial(PartialReason::ScanLimitReached))
    );
    assert_eq!(size.status.reason(), "Scan limit reached");
}

/// `scan_folder` in `folder_sizes.py` refuses limits below one entry or
/// second before it reads anything.
///
/// parity: PROP-029
#[test]
fn limits_of_zero_are_refused() {
    let refusals = [
        ScanLimits::new(0, MAX_DURATION),
        ScanLimits::new(1, Duration::ZERO),
    ];

    for refusal in refusals {
        assert_eq!(refusal, Err(SizeError::InvalidLimits));
    }
    assert_eq!(
        SizeError::InvalidLimits.to_string(),
        "Scan limits must be positive."
    );
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_cancellation_returns_partial_progress`
///
/// parity: PROP-029, PROP-026
#[test]
fn cancelling_returns_the_totals_counted_so_far() {
    let folder = Folder::new();
    for number in 0..10 {
        folder.write(&number.to_string(), b"x");
    }
    let provider = TestProvider::listing(Listing::CancelAfter(3));

    let size = folder
        .scan_through(&provider)
        .expect("a cancelled scan still returns its totals");

    assert_eq!(size.status, ScanStatus::Cancelled);
    assert!(size.bytes < 10, "{size:?}");
    assert_eq!(size.bytes, 3);
    assert_eq!(size.status.reason(), "Cancelled by user");
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_progress_and_completion_published`
///
/// parity: PROP-026
#[test]
fn progress_and_completion_are_published() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    let mut published = Vec::new();

    let size = scan_folder_size(&folder.uri(), &Cancellation::new(), |size| {
        published.push(size.clone());
    })
    .unwrap();

    let first = published.first().expect("the start is published");
    let last = published.last().expect("the end is published");
    assert_eq!(first.status, ScanStatus::Scanning);
    assert_eq!(last.status, ScanStatus::Complete);
    assert_eq!(last.bytes, 3);
    assert_eq!(*last, size);
    assert!(size.finished_at.is_some());
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_server_root_refused`
///
/// parity: PROP-026, PROP-029
#[test]
fn a_whole_smb_server_is_refused() {
    let refusal = scan("smb://nas/").unwrap_err();

    assert_eq!(refusal, SizeError::ServerRoot);
    assert_eq!(
        refusal.to_string(),
        "Open or select a share first, not the whole SMB server."
    );
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_nested_mount_excluded`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_unreadable_entries_reported`
///
/// parity: PROP-028
#[test]
fn unreadable_entries_are_reported_as_errors() {
    let folder = Folder::new();
    let provider = TestProvider::listing(Listing::Only(vec![SizeEntry::unreadable("unreadable")]));

    let size = folder.scan_through(&provider).unwrap();

    assert_eq!((size.errors, size.status), (1, EXCLUDED));
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_failed_subtree_reported`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_root_enumeration_error_propagates_for_native_retry`
///
/// parity: PROP-029
#[test]
fn a_listing_error_on_the_scanned_folder_is_returned_for_a_retry() {
    let folder = Folder::new();
    let refusal = EntryError::PermissionDenied("mount or authentication required".into());
    let provider = TestProvider::listing(Listing::FailAlways(refusal.clone()));

    assert_eq!(folder.scan_through(&provider), Err(SizeError::Read(refusal)));
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_root_inspect_error_is_not_an_empty_directory`
///
/// parity: PROP-029
#[test]
fn an_error_reading_the_scanned_folder_is_not_an_empty_folder() {
    let folder = Folder::new();
    let missing = EntryError::NotFound("offline or missing".into());
    let provider = TestProvider::failing_inspect(missing.clone());

    assert_eq!(folder.scan_through(&provider), Err(SizeError::Read(missing)));
}

/// An unmounted share reaches the caller, which mounts it and scans again,
/// as `start_worker` with `mount_retry` does in `desktop/winspace.py`.
///
/// parity: PROP-029
#[test]
fn an_unmounted_scanned_folder_asks_for_a_mount() {
    let folder = Folder::new();
    let provider = TestProvider::failing_inspect(EntryError::NotMounted("not mounted".into()));

    let refusal = folder.scan_through(&provider).unwrap_err();

    assert!(
        matches!(&refusal, SizeError::Read(error) if error.needs_mount()),
        "{refusal:?}"
    );
}

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_unknown_size_is_not_fabricated_zero`
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

/// Ported from `desktop/tests/test_v06.py::FolderSizeTests::test_spaces_in_path`
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
