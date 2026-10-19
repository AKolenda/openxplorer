// SPDX-License-Identifier: AGPL-3.0-only
//! Sources changing after compression's planning pass must not redirect reads.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use ox_core::archive::{ArchiveError, CompressionRequest, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

/// Replaces a planned source immediately before the writer opens it.
fn refuses_replacement(replace: fn(&Path, &Path)) {
    for archive_name in ["bundle.zip", "bundle.tar.xz"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("document.txt");
        let other = root.join("other.txt");
        fs::write(&source, "selected content").unwrap();
        fs::write(&other, "other example content").unwrap();
        let request = CompressionRequest {
            uris: vec![file_uri(&source)],
            destination_uri: file_uri(root),
            archive_name: archive_name.into(),
        };
        let changed_source = source.clone();
        let changed_other = other.clone();
        let mut compressor = ZipCompressor::new().with_progress(move |progress| {
            if progress.label.starts_with("Compressing ") {
                fs::remove_file(&changed_source).unwrap();
                replace(&changed_source, &changed_other);
            }
        });

        assert!(compressor.compress(&request, &Cancellation::new()).is_err());

        assert!(!root.join(archive_name).exists());
        assert_eq!(fs::read_to_string(other).unwrap(), "other example content");
        assert!(fs::read_dir(root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".openxplorer-compress-")
        }));
    }
}

#[test]
fn a_source_replaced_with_a_symlink_is_not_compressed() {
    refuses_replacement(|source, other| symlink(other, source).unwrap());
}

#[test]
fn a_source_replaced_with_a_fifo_does_not_wait_for_a_writer() {
    refuses_replacement(|source, _| {
        assert!(Command::new("mkfifo").arg(source).status().unwrap().success());
    });
}

#[test]
fn a_source_replaced_with_a_directory_is_not_compressed() {
    refuses_replacement(|source, _| fs::create_dir(source).unwrap());
}

#[test]
fn cancelling_before_opening_a_source_leaves_no_archive() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let source = root.join("document.txt");
    fs::write(&source, "selected content").unwrap();
    let request = CompressionRequest {
        uris: vec![file_uri(&source)],
        destination_uri: file_uri(root),
        archive_name: "bundle.zip".into(),
    };
    let cancel = Cancellation::new();
    let cancelling = cancel.clone();
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        if progress.label.starts_with("Compressing ") {
            cancelling.cancel();
        }
    });

    assert_eq!(
        compressor.compress(&request, &cancel),
        Err(ArchiveError::Cancelled)
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}
