// SPDX-License-Identifier: AGPL-3.0-only
//! Compress to ZIP writes ZIP64 records when an archive needs them: more
//! than 65,535 items, or a file, offset or directory past 4 GiB. Before,
//! such a compression stopped with "ZIP files over 4 GiB or with more than
//! 65,535 items need an archive manager."

use std::fs;
use std::process::Command;

use ox_core::archive::{ArchiveBrowser, CompressionRequest, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

/// A folder of 65,536 empty files, one more than a ZIP without ZIP64 can
/// list, compresses into a ZIP that Python's `zipfile` and the app's own
/// reader read back in full.
///
/// parity: ARC-023
#[test]
fn a_folder_of_more_than_65535_files_compresses_with_zip64() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let folder = root.join("Soundings");
    fs::create_dir(&folder).unwrap();
    for index in 0..65_536 {
        fs::write(folder.join(format!("{index:05}.txt")), b"").unwrap();
    }
    let request = CompressionRequest {
        uris: vec![file_uri(&folder)],
        destination_uri: file_uri(root),
        archive_name: "Soundings.zip".into(),
    };

    ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .expect("the ZIP is written");

    let archive = root.join("Soundings.zip");
    let checked = Command::new("python3")
        .args([
            "-I",
            "-c",
            "import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; print(len(z.namelist()))",
        ])
        .arg(&archive)
        .output()
        .expect("run python3");
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&checked.stdout).trim(), "65537");
    let browser = ArchiveBrowser::new(
        std::sync::Arc::new(ox_core::archive::GioArchiveOpener),
        root.join("previews"),
    );
    let listing = browser
        .list(&file_uri(&archive), "", &Cancellation::new())
        .expect("the app reads it back");
    assert_eq!(listing.entries.len(), 1);
}
