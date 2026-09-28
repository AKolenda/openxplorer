// SPDX-License-Identifier: AGPL-3.0-only
//! When a folder-size scan stops or refuses (PROP-026, PROP-027,
//! PROP-029): the scanned folder it refuses, its limits, cancellation,
//! progress, and errors on the scanned folder itself.
//!
//! Ports the rest of `FolderSizeTests` in `desktop/tests/test_v06.py`; the
//! totals and exclusions are in `sizes_scan.rs`. Where a Python test
//! subclasses `LocalSizeProvider`, the test here uses a `TestProvider`
//! from `sizes_support`. Every file is inside a temporary directory.

mod sizes_support;

use std::fs;
use std::os::unix::fs::symlink;
use std::time::Duration;

use ox_core::entry::EntryError;
use ox_core::location::file_uri;
use ox_core::sizes::{
    scan_folder_size, FolderSizeScan, LocalSizeProvider, PartialReason, ScanLimits, ScanStatus, SizeError,
    MAX_DURATION,
};
use ox_core::transfer::Cancellation;
use sizes_support::{scan, Folder, Listing, TestProvider};

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
