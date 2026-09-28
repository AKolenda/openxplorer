// SPDX-License-Identifier: AGPL-3.0-only
//! An extraction is all or nothing: progress, cancellation, and the
//! rollback after damaged data, failed writes and a racing folder. Ports
//! those cases of `ZipExtractTests` in `desktop/tests/test_zip_extract.py`.
//! Destinations that race or cannot be cleaned up use the local provider
//! of `transfer_support/`, the counterpart of
//! `desktop/tests/local_provider.py`.

#[path = "archive_support.rs"]
mod support;
#[allow(
    dead_code,
    unused_imports,
    reason = "these tests use the local provider only, not the other transfer doubles"
)]
mod transfer_support;

use std::fs;
use std::sync::Arc;

use ox_core::archive::{ArchiveError, ZipFormatError};
use ox_core::transfer::{Cancellation, Node, TransferError};

use support::{
    file_uri, first_member_data_offset, gio_factory, incompressible_bytes, zip_bytes, Compression,
    ExtractionFixture, LocalFileOutput, TestMember, WriteBehaviour,
};
use transfer_support::local::{local_path_of, LocalNode, Provider};

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_progress_and_cleanup`.
///
/// parity: ARC-011, ARC-013
#[test]
fn progress_ends_complete_and_only_the_new_folder_remains() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();

    fixture.extract("Unpacked").expect("the sample extracts");

    let events = fixture.events();
    let last = events.last().expect("progress was reported");
    assert!((last.fraction - 1.0).abs() < f64::EPSILON);
    assert!(events.iter().any(|event| event.label.starts_with("Extracting ")));
    assert_eq!(fixture.destination_names(), ["Unpacked"]);
}

/// The labels of the Python extractor, word for word.
///
/// parity: ARC-011
#[test]
fn progress_labels_name_the_check_each_file_and_the_result() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();

    fixture.extract("Unpacked").expect("the sample extracts");

    let labels: Vec<String> = fixture.events().into_iter().map(|event| event.label).collect();
    assert_eq!(
        labels,
        [
            "Checking ZIP contents…",
            "Extracting Docs/Guide.txt · 1/2 files",
            "Extracted 2 files into Unpacked",
        ]
    );
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_cancel_before_start`.
///
/// parity: ARC-013
#[test]
fn cancelling_before_the_start_writes_nothing() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    fixture.cancel.cancel();

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::Cancelled);
    fixture.assert_no_output();
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_cancel_during_write`.
///
/// parity: ARC-011, ARC-013
#[test]
fn cancelling_while_writing_removes_the_staging_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("large.bin", &incompressible_bytes(150_000))]);
    let cancel = fixture.cancel.clone();
    let mut extractor = fixture.extractor().with_progress(move |progress| {
        if progress.label.starts_with("Extracting ") {
            cancel.cancel();
        }
    });

    let result = fixture.extract_with(&mut extractor, "Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::Cancelled);
    fixture.assert_no_output();
    assert!(fixture.archive.is_file());
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_crc_failure_rolls_back`.
///
/// parity: ARC-013
#[test]
fn a_crc_failure_rolls_back() {
    let fixture = ExtractionFixture::new();
    let member = TestMember::file("doc.txt", b"contents").compressed_with(Compression::Stored);
    let mut archive = zip_bytes(&[member]);
    archive[first_member_data_offset("doc.txt")] ^= 1;
    fs::write(&fixture.archive, archive).expect("write the damaged archive");

    let result = fixture.extract("Unpacked");

    let expected = ArchiveError::Format(ZipFormatError::BadCrc("doc.txt".to_owned()));
    assert_eq!(result.unwrap_err(), expected);
    fixture.assert_no_output();
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_disk_error_rolls_back`.
///
/// parity: ARC-013
#[test]
fn a_failed_write_rolls_back() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let output = LocalFileOutput::failing(WriteBehaviour::DiskFull);
    let mut extractor = fixture.extractor_with(gio_factory(), Arc::new(output));

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    assert_eq!(error.to_string(), "Disk full");
    fixture.assert_no_output();
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_short_write_rolls_back`.
///
/// parity: ARC-013
#[test]
fn a_short_write_rolls_back() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let output = LocalFileOutput::failing(WriteBehaviour::Short);
    let mut extractor = fixture.extractor_with(gio_factory(), Arc::new(output));

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    assert_eq!(error, ArchiveError::IncompleteWrite);
    assert_eq!(
        error.to_string(),
        "The destination did not accept all extracted bytes."
    );
    fixture.assert_no_output();
}

/// A destination where another program creates the output folder right
/// before the extraction publishes it, like `Racing` in
/// `test_racing_destination_is_not_replaced`.
struct RacingDestination;

impl Provider for RacingDestination {
    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        let competitor = local_path_of(target);
        fs::create_dir(&competitor)?;
        fs::write(competitor.join("competitor"), "keep")?;
        node.local_move_native(target, cancel)
    }
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_racing_destination_is_not_replaced`.
///
/// parity: ARC-012, ARC-013
#[test]
fn a_folder_created_meanwhile_is_never_replaced() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let factory = LocalNode::factory(Arc::new(RacingDestination));
    let mut extractor = fixture.extractor_with(factory, Arc::new(LocalFileOutput::owner_only()));

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    assert!(
        matches!(error, ArchiveError::Backend(TransferError::Exists(_))),
        "{error:?}"
    );
    let competitor = fs::read_to_string(fixture.destination.join("Unpacked/competitor")).expect("kept");
    assert_eq!(competitor, "keep");
    assert_eq!(fixture.destination_names(), ["Unpacked"]);
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_not_a_zip`.
///
/// parity: ARC-013, ARC-014
#[test]
fn a_file_that_is_not_a_zip_writes_nothing() {
    let fixture = ExtractionFixture::new();
    fs::write(&fixture.archive, "not a zip").expect("write the file");

    let result = fixture.extract("Unpacked");

    assert_eq!(result.unwrap_err(), ArchiveError::Format(ZipFormatError::NotAZip));
    fixture.assert_no_output();
}

/// A destination whose items cannot be removed.
struct UndeletableItems;

impl Provider for UndeletableItems {
    fn delete(&self, _node: &LocalNode) -> Result<(), TransferError> {
        Err(TransferError::failed("Permission denied"))
    }
}

/// The report of `ZipExtractor.extract` in `desktop/zip_extraction.py`
/// when the staging folder of a failed extraction cannot be removed.
///
/// parity: ARC-013
#[test]
fn a_staging_folder_that_cannot_be_removed_is_reported_where_it_is() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let output = LocalFileOutput::failing(WriteBehaviour::DiskFull);
    let factory = LocalNode::factory(Arc::new(UndeletableItems));
    let mut extractor = fixture.extractor_with(factory, Arc::new(output));

    let error = fixture.extract_with(&mut extractor, "Unpacked").unwrap_err();

    let names = fixture.destination_names();
    let [staging_name] = names.as_slice() else {
        panic!("only the staging folder remains: {names:?}");
    };
    let random_part = staging_name
        .strip_prefix(".openxplorer-extract-")
        .and_then(|rest| rest.strip_suffix(".part"));
    assert!(random_part.is_some(), "{staging_name}");
    let staging_uri = file_uri(&fixture.destination.join(staging_name));
    let expected = format!(
        "Disk full\nIncomplete extraction remains at {staging_uri}. Inspect it before removing it. Permission denied"
    );
    assert_eq!(error.to_string(), expected);
}
