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
        .into_existing_folder()
        .extract(&request, &fixture.cancel)
        .unwrap_err()
        .to_string();

    unlock(&fixture.destination);
    assert_names(&error, &fixture.destination);
    fixture.assert_no_output();
}

/// A new folder the user names like a private one is still named as is.
///
/// parity: ARC-013
#[test]
fn a_new_folder_named_like_a_private_one_keeps_its_name_in_errors() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    if !lock(&fixture.destination) {
        return;
    }
    let name = format!(".openxplorer-extract-{}", "d".repeat(32));

    let error = fixture.extract(&name).unwrap_err().to_string();

    unlock(&fixture.destination);
    let real = format!("{}: Permission denied", fixture.destination.join(&name).display());
    assert!(error.ends_with(&real), "{error}");
    assert!(!error.contains(".part"), "{error}");
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

/// A compression whose staging cannot be removed after it failed names
/// that staging exactly, so the user can find what was left behind.
///
/// parity: ARC-023
#[test]
fn a_compression_staging_left_behind_is_reported_where_it_is() {
    let temporary = tempfile::tempdir().expect("create a temporary folder");
    let sources = temporary.path().join("Sources");
    fs::create_dir(&sources).expect("create the sources");
    let first = sources.join("a.txt");
    let second = sources.join("b.txt");
    fs::write(&first, "a").expect("write a source");
    fs::write(&second, "b").expect("write a source");
    let destination = temporary.path().join("Destination");
    fs::create_dir(&destination).expect("create the destination");
    if !lock(&destination) {
        return;
    }
    unlock(&destination);
    let request = CompressionRequest {
        uris: vec![file_uri(&first), file_uri(&second)],
        destination_uri: file_uri(&destination),
        archive_name: "both.zip".into(),
    };
    let (folder, unreadable) = (destination.clone(), second.clone());
    // The share turns read-only while the second item is read, so neither
    // its write nor the cleanup succeeds.
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        if progress.label.ends_with("(2/2)") {
            fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmod");
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).expect("chmod");
        }
    });

    let error = compressor
        .compress(&request, &Cancellation::new())
        .unwrap_err()
        .to_string();

    unlock(&destination);
    let left: Vec<_> = fs::read_dir(&destination)
        .expect("list")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(error.contains(&file_uri(&left[0])), "{error}");
    assert!(error.contains("Inspect it before removing it."), "{error}");
}
