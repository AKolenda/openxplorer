// SPDX-License-Identifier: AGPL-3.0-only
//! Extracting ZIP archives into a new folder with local files: what is
//! extracted, which destinations are refused, and previous-version
//! protection. Ports `ZipExtractTests` of
//! `v2.0.0:desktop/tests/test_zip_extract.py`. Rollback and cancellation are in
//! `archive_extract_rollback.rs`, the member checks and limits in
//! `archive_extract_checks.rs`, and the checks of the dispatcher in
//! `archive_extract_dispatch.rs`. Destinations that behave like a device
//! use the local provider of `transfer_support/`, the counterpart of
//! `v2.0.0:desktop/tests/local_provider.py`.

mod archive_support;
#[allow(
    dead_code,
    unused_imports,
    reason = "these tests use the local provider and the write guard only, not the other transfer doubles"
)]
mod transfer_support;

use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;

use ox_core::archive::{ArchiveError, ExtractionRequest};

use archive_support::{
    file_type, file_uri, mode_of, write_zip_with_python, CreatedMode, ExtractionFixture, LocalFileOutput,
    TestMember,
};
use transfer_support::local::{LocalNode, Provider};
use transfer_support::versions::{PreviousVersions, READ_ONLY};

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_round_trip`.
///
/// parity: ARC-012, ARC-013
#[test]
fn extracts_files_folders_and_empty_files_into_a_new_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();

    let extracted = fixture.extract("Unpacked").expect("the sample extracts");

    let folder = fixture.destination.join("Unpacked");
    let guide = fs::read(folder.join("Docs/Guide.txt")).expect("the guide was extracted");
    assert_eq!(guide, b"hello world");
    let empty = fs::metadata(folder.join("Zero.bin")).expect("the empty file was extracted");
    assert_eq!(empty.len(), 0);
    let summary = extracted.summary;
    assert_eq!(
        (summary.file_count, summary.folder_count, summary.unpacked_bytes),
        (2, 1, 11)
    );
    assert_eq!(extracted.uri, file_uri(&folder));
    assert_eq!(extracted.name, "Unpacked");
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_implied_directories`.
///
/// parity: ARC-013, ARC-019
#[test]
fn folders_implied_by_member_paths_are_created() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("a/b/c.txt", b"yes")]);

    fixture.extract("Unpacked").expect("the archive extracts");

    let text = fs::read_to_string(fixture.destination.join("Unpacked/a/b/c.txt")).expect("extracted");
    assert_eq!(text, "yes");
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_parent_after_child`.
///
/// parity: ARC-013, ARC-019
#[test]
fn a_folder_entry_after_its_contents_is_accepted() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("a/b.txt", b"data"), TestMember::folder("a/")]);

    fixture.extract("Unpacked").expect("the archive extracts");

    assert!(fixture.destination.join("Unpacked/a/b.txt").is_file());
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_empty_archive`.
///
/// parity: ARC-013, ARC-019
#[test]
fn an_empty_archive_extracts_to_an_empty_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[]);

    let extracted = fixture.extract("Unpacked").expect("an empty archive extracts");

    assert_eq!(extracted.summary.file_count, 0);
    assert!(fixture.destination.join("Unpacked").is_dir());
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_empty_folders`.
///
/// parity: ARC-013, ARC-019
#[test]
fn empty_folder_entries_are_created() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::folder("a/b/")]);

    fixture.extract("Unpacked").expect("the archive extracts");

    assert!(fixture.destination.join("Unpacked/a/b").is_dir());
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_unicode_spaces`.
///
/// parity: ARC-013, ARC-019
#[test]
fn unicode_and_spaces_in_names_are_kept() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("Design notes/Café.txt", b"fictional")]);

    fixture.extract("Unpacked").expect("the archive extracts");

    assert!(fixture
        .destination
        .join("Unpacked/Design notes/Café.txt")
        .is_file());
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_source_unchanged`.
///
/// parity: ARC-012, ARC-013
#[test]
fn the_archive_is_left_unchanged() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let before = fs::read(&fixture.archive).expect("read the archive");

    fixture.extract("Unpacked").expect("the sample extracts");

    assert_eq!(fs::read(&fixture.archive).expect("read the archive"), before);
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_existing_folder`.
///
/// parity: ARC-011, ARC-012
#[test]
fn an_existing_folder_is_never_merged_into() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let sentinel = fixture.destination.join("Unpacked/sentinel");
    fs::create_dir(fixture.destination.join("Unpacked")).expect("create the folder");
    fs::write(&sentinel, "keep").expect("write the sentinel");

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::DestinationExists);
    assert_eq!(fs::read_to_string(&sentinel).expect("the sentinel stays"), "keep");
    assert_eq!(fixture.destination_names(), ["Unpacked"]);
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_existing_file`.
///
/// parity: ARC-012
#[test]
fn an_existing_file_keeps_the_name() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let existing = fixture.destination.join("Unpacked");
    fs::write(&existing, "keep").expect("write the file");

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::DestinationExists);
    assert_eq!(fs::read_to_string(&existing).expect("the file stays"), "keep");
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_existing_symlink`.
///
/// parity: ARC-012
#[test]
fn an_existing_link_keeps_the_name_and_its_target_stays_empty() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let other = fixture.root.join("other");
    fs::create_dir(&other).expect("create the link target");
    let link = fixture.destination.join("Unpacked");
    symlink(&other, &link).expect("create the link");

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::DestinationExists);
    assert!(fs::symlink_metadata(&link).expect("the link stays").is_symlink());
    assert_eq!(fs::read_dir(&other).expect("list the target").count(), 0);
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_dangling_symlink`.
///
/// parity: ARC-012
#[test]
fn a_dangling_link_keeps_the_name() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let link = fixture.destination.join("Unpacked");
    symlink(fixture.root.join("missing"), &link).expect("create the link");

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::DestinationExists);
    assert!(fs::symlink_metadata(&link).expect("the link stays").is_symlink());
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_parent_cannot_be_symlink`.
///
/// parity: ARC-012, ARC-020
#[test]
fn a_linked_destination_folder_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let link = fixture.root.join("linked");
    symlink(&fixture.destination, &link).expect("create the link");
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: file_uri(&link),
        folder_name: "Test".to_owned(),
    };

    let result = fixture.extractor().extract(&request, &fixture.cancel);

    assert_eq!(result.unwrap_err(), ArchiveError::NotARealFolder);
    fixture.assert_no_output();
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_invalid_destination_names`.
///
/// parity: ARC-012, ARC-020
#[test]
fn invalid_folder_names_are_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();

    for name in ["", ".", "..", "../escape", "a/b", "a\\b", "\0bad"] {
        let result = fixture.extract(name);

        assert!(
            matches!(result, Err(ArchiveError::Location(_))),
            "{name:?}: {result:?}"
        );
    }
    fixture.assert_no_output();
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_never_restores_executable_or_setuid_mode`.
///
/// parity: ARC-013, ARC-018
#[test]
fn archive_permissions_are_never_applied() {
    let fixture = ExtractionFixture::new();
    let script = TestMember::with_unix_mode(
        "script.sh",
        file_type::REGULAR | 0o4755,
        b"#!/bin/sh\necho never run\n",
    );
    fixture.write_zip(&[script]);

    fixture.extract("Unpacked").expect("the archive extracts");

    let folder = fixture.destination.join("Unpacked");
    assert_eq!(mode_of(&folder.join("script.sh")), 0o600);
    assert_eq!(mode_of(&folder), 0o700);
}

/// A destination that reports `mtp://` URIs for local folders, like
/// `DeviceNode` in `test_device_destination_does_not_require_unix_chmod`.
struct DeviceStorage;

impl Provider for DeviceStorage {
    fn uri(&self, node: &LocalNode) -> String {
        format!("mtp://test-device{}", node.local_path().display())
    }
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_device_destination_does_not_require_unix_chmod`.
/// Python asserted that `chmod` is never called; here the new folder keeps
/// the mode a plain folder creation gives.
///
/// parity: ARC-018, ARC-020
#[test]
fn a_device_destination_gets_no_unix_modes() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let device_output = LocalFileOutput {
        created_mode: CreatedMode::Unchanged,
        ..LocalFileOutput::owner_only()
    };
    let mut extractor = fixture.extractor_with(
        LocalNode::factory(Arc::new(DeviceStorage)),
        Arc::new(device_output),
    );
    let plain_folder = fixture.root.join("plain");
    fs::create_dir(&plain_folder).expect("create a plain folder");

    let extracted = fixture
        .extract_with(&mut extractor, "Unpacked")
        .expect("the sample extracts");

    let folder = fixture.destination.join("Unpacked");
    assert_eq!(extracted.summary.file_count, 2);
    assert!(
        extracted.uri.starts_with("mtp://test-device/"),
        "{}",
        extracted.uri
    );
    let guide = fs::read_to_string(folder.join("Docs/Guide.txt")).expect("the guide was extracted");
    assert_eq!(guide, "hello world");
    assert_eq!(mode_of(&folder), mode_of(&plain_folder));
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_protected_extraction_descendant_fails_before_writing`.
///
/// parity: ARC-010, ARC-020
#[test]
fn a_protected_member_path_stops_the_extraction_before_writing() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("ordinary.txt", b"first"),
        TestMember::file(".snapshot/version.txt", b"backup"),
    ]);
    let versions = PreviousVersions::new();
    let mut extractor = fixture.extractor().with_write_guard(versions.guard());

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    assert_eq!(error.to_string(), READ_ONLY);
    assert!(error.to_string().contains("read-only"));
    fixture.assert_no_output();
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_configured_extraction_root_is_protected`.
///
/// parity: ARC-020
#[test]
fn a_new_folder_inside_a_configured_snapshot_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let versions = PreviousVersions::new();
    versions.configure(
        &fixture.destination_uri(),
        &file_uri(&fixture.destination.join("Unpacked")),
    );
    let mut extractor = fixture.extractor().with_write_guard(versions.guard());

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    assert_eq!(error.to_string(), READ_ONLY);
    fixture.assert_no_output();
}

/// Ported from `v2.0.0:desktop/tests/test_zip_extract.py::ZipExtractTests::test_supported_compression_round_trip`,
/// with archives written by Python's `zipfile`.
///
/// parity: ARC-013, ARC-016
#[test]
fn archives_python_writes_with_each_supported_method_round_trip() {
    let fixture = ExtractionFixture::new();
    for method in ["ZIP_STORED", "ZIP_DEFLATED", "ZIP_BZIP2", "ZIP_LZMA"] {
        write_zip_with_python(&fixture.archive, method, "a.txt", "data");

        fixture.extract(method).expect("each supported method extracts");

        let text = fs::read_to_string(fixture.destination.join(method).join("a.txt")).expect("extracted");
        assert_eq!(text, "data", "{method}");
    }
}
