// SPDX-License-Identifier: AGPL-3.0-only
//! Links and special files inside a compressed folder (ARC-023).
//!
//! Windows' Compressed (zipped) Folder and its Create archive dialog with
//! "Retain symbolic links" clear store what a link points to, under the
//! link's name. Whatever cannot be stored (a FIFO, a socket, a dangling
//! link, a link that would loop) is named in the result, so the app can
//! say what the archive lacks instead of leaving it out silently.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;

use ox_core::archive::{CompressionRequest, LeftOutReason, ZipCompressor};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

/// `~/Documents` with `Meeting notes.txt`, `Projects/plan.txt` and a
/// folder `Special` holding a real file, links of every kind, a FIFO and
/// a socket, as the review's corpus has them.
fn documents(root: &Path) -> std::path::PathBuf {
    fs::write(root.join("Meeting notes.txt"), "Agenda for the planning meeting").unwrap();
    fs::create_dir(root.join("Projects")).unwrap();
    fs::write(root.join("Projects/plan.txt"), "Quarterly plan").unwrap();
    let special = root.join("Special");
    fs::create_dir(&special).unwrap();
    fs::write(special.join("hardlink notes.txt"), "kept").unwrap();
    symlink("../Meeting notes.txt", special.join("link to notes.txt")).unwrap();
    symlink("../Projects", special.join("link to Projects")).unwrap();
    symlink("missing file.txt", special.join("dangling.txt")).unwrap();
    symlink(".", special.join("self")).unwrap();
    symlink("loop-b", special.join("loop-a")).unwrap();
    symlink("loop-a", special.join("loop-b")).unwrap();
    assert!(Command::new("mkfifo")
        .arg(special.join("pipe.txt"))
        .status()
        .unwrap()
        .success());
    let _socket = UnixListener::bind(special.join("socket")).unwrap();
    special
}

/// The files of the ZIP at `path` with their contents, and its folders
/// with none, read back by Python's zipfile, an independent reader.
fn zip_contents(path: &Path) -> BTreeMap<String, Option<String>> {
    let script = "import json, sys, zipfile\n\
        archive = zipfile.ZipFile(sys.argv[1])\n\
        print(json.dumps({info.filename: None if info.is_dir() else archive.read(info).decode() \
        for info in archive.infolist()}))";
    let output = Command::new("python3")
        .args(["-I", "-c", script])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// parity: ARC-023
#[test]
fn a_compressed_folder_stores_what_its_links_point_to_and_names_what_it_cannot_store() {
    let temporary = tempfile::tempdir().unwrap();
    let special = documents(temporary.path());
    let request = CompressionRequest {
        uris: vec![file_uri(&special)],
        destination_uri: file_uri(temporary.path()),
        archive_name: "Special.zip".into(),
    };

    let created = ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .unwrap();

    let text = |value: &str| Some(value.to_owned());
    let expected: BTreeMap<String, Option<String>> = [
        ("Special/".to_owned(), None),
        ("Special/hardlink notes.txt".to_owned(), text("kept")),
        ("Special/link to Projects/".to_owned(), None),
        (
            "Special/link to Projects/plan.txt".to_owned(),
            text("Quarterly plan"),
        ),
        (
            "Special/link to notes.txt".to_owned(),
            text("Agenda for the planning meeting"),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(zip_contents(&temporary.path().join("Special.zip")), expected);
    let left_out: Vec<(&str, LeftOutReason)> = created
        .left_out
        .iter()
        .map(|item| (item.name.as_str(), item.reason))
        .collect();
    assert_eq!(
        left_out,
        [
            ("Special/dangling.txt", LeftOutReason::TargetMissing),
            ("Special/loop-a", LeftOutReason::TargetMissing),
            ("Special/loop-b", LeftOutReason::TargetMissing),
            ("Special/pipe.txt", LeftOutReason::SpecialFile),
            ("Special/self", LeftOutReason::Loop),
            ("Special/socket", LeftOutReason::SpecialFile),
        ]
    );
    // The links' targets are only read.
    assert_eq!(
        fs::read_to_string(temporary.path().join("Meeting notes.txt")).unwrap(),
        "Agenda for the planning meeting"
    );
}
