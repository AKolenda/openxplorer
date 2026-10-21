// SPDX-License-Identifier: AGPL-3.0-only
//! An extraction or compression that fails names the folder or archive the
//! user asked for, never its private `.openxplorer-extract-<hex>.part` or
//! `.openxplorer-compress-<hex>.part` staging, and keeps the system's own
//! reason.

mod archive_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use ox_core::archive::{private_extraction_name, CompressionRequest, ExtractionRequest, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

use archive_support::ExtractionFixture;

/// Makes `folder` read-only (`chmod 555`). False when the test runs as
/// root, which writes anywhere; the folder is then writable again.
fn lock(folder: &Path) -> bool {
    fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).expect("chmod");
    let probe = folder.join("probe");
    if fs::write(&probe, "").is_ok() {
        let _ = fs::remove_file(probe);
        unlock(folder);
        return false;
    }
    true
}

/// Gives the owner write access to `folder` again.
fn unlock(folder: &Path) {
    fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// Asserts `error` names `real` and the system's reason, and nothing of
/// the staging.
fn assert_names(error: &str, real: &Path) {
    assert!(!error.contains(".openxplorer-"), "{error}");
    assert!(!error.contains(".part"), "{error}");
    assert!(error.contains(&real.display().to_string()), "{error}");
    assert!(error.contains("Permission denied"), "{error}");
}

/// parity: ARC-013
#[test]
fn an_extraction_into_a_read_only_folder_names_the_new_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    if !lock(&fixture.destination) {
        return;
    }

    let error = fixture.extract("Unpacked").unwrap_err().to_string();

    unlock(&fixture.destination);
    assert_names(&error, &fixture.destination.join("Unpacked"));
    fixture.assert_no_output();
}

/// "Extract all" into an existing folder unpacks into a private folder
/// inside it first (`private_extraction_name`). Its errors name the folder
/// the user chose, never that private name either.
///
/// parity: ARC-013
#[test]
fn an_extraction_into_a_read_only_existing_folder_names_that_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    if !lock(&fixture.destination) {
        return;
    }
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: fixture.destination_uri(),
        folder_name: private_extraction_name().expect("random"),
    };

    let error = fixture
        .extractor()
        .extract(&request, &fixture.cancel)
        .unwrap_err()
        .to_string();

    unlock(&fixture.destination);
    assert_names(&error, &fixture.destination);
    fixture.assert_no_output();
}

/// parity: ARC-023
#[test]
fn a_compression_into_a_read_only_folder_names_the_new_archive() {
    let temporary = tempfile::tempdir().expect("create a temporary folder");
    let source = temporary.path().join("one.txt");
    fs::write(&source, "one").expect("write the source");
    let locked = temporary.path().join("Locked");
    fs::create_dir(&locked).expect("create the destination");
    if !lock(&locked) {
        return;
    }
    let request = CompressionRequest {
        uris: vec![file_uri(&source)],
        destination_uri: file_uri(&locked),
        archive_name: "one.zip".into(),
    };

    let error = ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .unwrap_err()
        .to_string();

    unlock(&locked);
    assert_names(&error, &locked.join("one.zip"));
    assert_eq!(fs::read_dir(&locked).expect("list").count(), 0);
}
