// SPDX-License-Identifier: AGPL-3.0-only
//! The checks the `archiveExtract` and `archiveInspect` branches of
//! `dispatch` in `v2.0.0:desktop/winspace.py` add around the extractor:
//! normalised locations, the write guard on the destination folder, no
//! server listing as a destination, and no merging into an existing
//! folder. Ports the archive cases of `DispatchTests` in
//! `v2.0.0:desktop/tests/test_rc2.py`; the dispatcher's busy check belongs to the
//! app, which runs one extraction at a time.

mod archive_support;
#[allow(
    dead_code,
    unused_imports,
    reason = "these tests use the write guard only, not the other transfer doubles"
)]
mod transfer_support;

use std::fs;

use ox_core::archive::{ArchiveError, ExtractionRequest};

use archive_support::{file_uri, ExtractionFixture, TestMember};
use transfer_support::versions::{PreviousVersions, READ_ONLY};

/// The `archiveExtract` branch of `dispatch` in `v2.0.0:desktop/winspace.py`
/// asks the guard about the destination folder itself first.
///
/// parity: ARC-020
#[test]
fn a_protected_destination_folder_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let snapshot = fixture.destination.join(".snapshot");
    fs::create_dir(&snapshot).expect("create a snapshot folder");
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: file_uri(&snapshot),
        folder_name: "Unpacked".to_owned(),
    };
    let mut extractor = fixture
        .extractor()
        .with_write_guard(PreviousVersions::new().guard());

    let error = extractor.extract(&request, &fixture.cancel).unwrap_err();

    assert_eq!(error.to_string(), READ_ONLY);
    assert_eq!(fs::read_dir(&snapshot).expect("list the snapshot").count(), 0);
}

/// The `archiveExtract` branch of `dispatch` in `v2.0.0:desktop/winspace.py`
/// refuses a server's list of shares before resolving it.
///
/// parity: ARC-010, OPS-036
#[test]
fn a_server_listing_is_not_a_destination() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: "smb://archive-nas/".to_owned(),
        folder_name: "Unpacked".to_owned(),
    };

    let error = fixture
        .extractor()
        .extract(&request, &fixture.cancel)
        .unwrap_err();

    assert_eq!(error, ArchiveError::ServerListingDestination);
    assert_eq!(
        error.to_string(),
        "Open a network share before choosing it as an extraction destination."
    );
}

/// The dispatcher normalises both locations before extracting; an address
/// it cannot normalise is refused with the location's message.
///
/// parity: ARC-010
#[test]
fn locations_are_normalised_first() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let spelled_with_dots = format!("{}/../destination/.", fixture.destination_uri());
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: spelled_with_dots,
        folder_name: "Unpacked".to_owned(),
    };

    let extracted = fixture
        .extractor()
        .extract(&request, &fixture.cancel)
        .expect("the sample extracts");

    assert_eq!(
        extracted.destination_uri,
        ox_core::location::file_uri(&fixture.destination)
    );
    let unsupported = ExtractionRequest {
        destination_uri: "http://example.com/".to_owned(),
        ..request
    };
    let error = fixture
        .extractor()
        .extract(&unsupported, &fixture.cancel)
        .unwrap_err();
    assert!(matches!(error, ArchiveError::Location(_)), "{error:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_rc2.py::DispatchTests::test_extract_dispatch_checks_member_destinations`.
///
/// parity: ARC-020
#[test]
fn member_destinations_are_checked_before_any_staging() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file(".snapshot/version.txt", b"saved")]);
    let mut extractor = fixture
        .extractor()
        .with_write_guard(PreviousVersions::new().guard());

    let error = fixture.extract_with(&mut extractor, "Extracted").unwrap_err();

    assert!(error.to_string().contains("read-only"), "{error}");
    assert!(!fixture.destination.join("Extracted").exists());
    fixture.assert_no_output();
}

/// Ported from `v2.0.0:desktop/tests/test_rc2.py::DispatchTests::test_inspect_and_extract_actual_actions`.
///
/// parity: ARC-008, ARC-014
#[test]
fn inspection_and_extraction_agree() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("Artwork/notes.txt", b"demo"),
        TestMember::file("readme.md", b"# example"),
    ]);

    let summary = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .expect("inspected");
    let extracted = fixture.extract("Unpacked").expect("the archive extracts");

    assert_eq!(
        (summary.file_count, summary.folder_count, summary.entry_count),
        (2, 1, 2)
    );
    assert_eq!(summary.unpacked_bytes, 13);
    let notes =
        fs::read_to_string(fixture.destination.join("Unpacked/Artwork/notes.txt")).expect("extracted");
    assert_eq!(notes, "demo");
    assert_eq!(extracted.summary, summary);
    assert_eq!(extracted.uri, file_uri(&fixture.destination.join("Unpacked")));
    assert!(fixture.archive.exists());
}

/// Ported from `v2.0.0:desktop/tests/test_rc2.py::DispatchTests::test_extract_collision_never_merges`.
///
/// parity: ARC-011, ARC-012
#[test]
fn an_existing_output_folder_fails_without_merging() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("data", b"x")]);
    let out = fixture.destination.join("out");
    fs::create_dir(&out).expect("create the folder");
    fs::write(out.join("sentinel"), "keep").expect("write the sentinel");

    let error = fixture.extract("out").unwrap_err();

    assert_eq!(error, ArchiveError::DestinationExists);
    assert_eq!(
        error.to_string(),
        "The destination already exists. Choose a new folder name; existing files are never overwritten."
    );
    assert_eq!(fs::read_to_string(out.join("sentinel")).expect("kept"), "keep");
    assert!(!out.join("data").exists());
}

/// Copying items out of a ZIP extracts only them, at their paths, after
/// the whole archive passed the checks (ARC-026).
///
/// parity: ARC-026
#[test]
fn a_selection_extracts_only_the_chosen_members() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("tidewater/maps/a.txt", b"a"),
        TestMember::file("tidewater/maps/deep/b.txt", b"bb"),
        TestMember::file("tidewater/readme.md", b"read"),
        TestMember::file("tidewater/other.txt", b"other"),
        TestMember::file("top.txt", b"top"),
    ]);
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: fixture.destination_uri(),
        folder_name: "copy".to_owned(),
    };
    let extracted = fixture
        .extractor()
        .with_selection(&["tidewater/maps/".to_owned(), "tidewater/readme.md".to_owned()])
        .extract(&request, &fixture.cancel)
        .expect("extracted");

    let copy = fixture.destination.join("copy");
    assert_eq!(fs::read(copy.join("tidewater/maps/a.txt")).expect("a"), b"a");
    assert_eq!(
        fs::read(copy.join("tidewater/maps/deep/b.txt")).expect("b"),
        b"bb"
    );
    assert_eq!(
        fs::read(copy.join("tidewater/readme.md")).expect("readme"),
        b"read"
    );
    assert!(!copy.join("tidewater/other.txt").exists());
    assert!(!copy.join("top.txt").exists());
    assert_eq!(extracted.summary.file_count, 3);
    assert_eq!(extracted.summary.unpacked_bytes, 7);
}
