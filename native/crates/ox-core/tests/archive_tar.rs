// SPDX-License-Identifier: AGPL-3.0-only
//! TAR archives, written by Python's `tarfile` as `tar` writes them, are
//! browsed and extracted by the ZIP rules (ARC-022, ARC-024).

mod archive_support;

use std::fs;
use std::path::Path;
use std::process::Command;

use ox_core::archive::{ArchiveBrowser, ArchiveError};

use archive_support::{file_uri, opener, ExtractionFixture};

/// Writes `path` with Python's `tarfile` in `mode` (`w:gz`, `w:xz`,
/// `w:bz2`, `w`): a folder `Docs` with `Guide.txt`, and with `link` a
/// symbolic link.
fn write_tar(path: &Path, mode: &str, link: bool) {
    let script = "import io, sys, tarfile\n\
                  with tarfile.open(sys.argv[1], sys.argv[2]) as archive:\n\
                  \x20   folder = tarfile.TarInfo('./Docs'); folder.type = tarfile.DIRTYPE; archive.addfile(folder)\n\
                  \x20   data = b'hello world'\n\
                  \x20   member = tarfile.TarInfo('./Docs/Guide.txt'); member.size = len(data)\n\
                  \x20   archive.addfile(member, io.BytesIO(data))\n\
                  \x20   if sys.argv[3] == 'link':\n\
                  \x20       link = tarfile.TarInfo('Docs/escape'); link.type = tarfile.SYMTYPE\n\
                  \x20       link.linkname = '/etc/passwd'; archive.addfile(link)\n";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(path)
        .args([mode, if link { "link" } else { "plain" }])
        .output()
        .expect("Python 3 is required to write reference archives");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// parity: ARC-024
#[test]
fn compressed_tars_extract_into_a_new_folder() {
    for mode in ["w", "w:gz", "w:bz2", "w:xz"] {
        let fixture = ExtractionFixture::new();
        write_tar(&fixture.archive, mode, false);

        let extracted = fixture.extract("Unpacked").expect("the TAR extracts");

        let guide = fixture.destination.join("Unpacked/Docs/Guide.txt");
        assert_eq!(fs::read(guide).expect("extracted"), b"hello world", "{mode}");
        assert_eq!(
            (extracted.summary.file_count, extracted.summary.folder_count),
            (1, 1)
        );
    }
}

/// parity: ARC-024
#[test]
fn a_tar_with_a_link_extracts_nothing() {
    let fixture = ExtractionFixture::new();
    write_tar(&fixture.archive, "w:gz", true);

    let refused = fixture.extract("Unpacked");

    assert_eq!(refused.unwrap_err(), ArchiveError::LinkOrSpecialFile);
    assert!(!fixture.destination.join("Unpacked").exists());
}

/// parity: ARC-022
#[test]
fn a_tar_is_browsed_like_a_zip() {
    let temporary = tempfile::tempdir().expect("a temporary folder");
    let archive = temporary.path().join("docs.tar.xz");
    write_tar(&archive, "w:xz", true);
    let browser = ArchiveBrowser::new(opener(), temporary.path().join("previews"));
    let cancel = ox_core::transfer::Cancellation::new();

    let listing = browser
        .list(&file_uri(&archive), "Docs/", &cancel)
        .expect("lists");
    let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Guide.txt"], "the link is hidden");
    let copy = browser
        .preview_member(&file_uri(&archive), "Docs/Guide.txt", &cancel)
        .expect("a text member previews");
    assert_eq!(fs::read(copy.path).expect("the private copy"), b"hello world");
}
