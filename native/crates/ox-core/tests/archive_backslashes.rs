// SPDX-License-Identifier: AGPL-3.0-only
//! ZIPs whose member names separate folders with backslashes, as some
//! Windows tools write them (`Compress-Archive` of Windows `PowerShell` 5.1
//! among them), although the ZIP specification asks for forward slashes
//! (APPNOTE.TXT 4.4.17). A backslash cannot be part of a Windows file
//! name, so it is read as a folder separator, as Info-ZIP's `unzip` and
//! 7-Zip read it; every rule for unsafe paths then applies to the folders
//! it gives.

mod archive_support;

use std::fs;

use ox_core::archive::{ArchiveBrowser, ArchiveError};

use archive_support::{opener, ExtractionFixture, TestMember};

/// A browser over the fixture's archive.
fn browser(fixture: &ExtractionFixture) -> ArchiveBrowser {
    ArchiveBrowser::new(opener(), fixture.root.join("previews"))
}

/// A ZIP written with backslashes shows its folders and files, and a file
/// in it opens.
///
/// parity: ARC-004, ARC-006
#[test]
fn backslashes_separate_folders_when_browsing() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("Harbor\\Charts\\north.txt", b"North channel"),
        TestMember::file("Harbor\\tides.txt", b"High water at noon."),
    ]);
    let uri = fixture.archive_uri();

    let top = browser(&fixture).list(&uri, "", &fixture.cancel).expect("lists");
    let inside = browser(&fixture)
        .list(&uri, "Harbor/", &fixture.cancel)
        .expect("lists");

    assert_eq!(top.hidden_unsafe_count, 0);
    let names = |listing: &ox_core::archive::ArchiveListing| -> Vec<String> {
        listing.entries.iter().map(|entry| entry.name.clone()).collect()
    };
    assert_eq!(names(&top), ["Harbor"]);
    assert_eq!(names(&inside), ["Charts", "tides.txt"]);
    let copy = browser(&fixture)
        .preview_member(&uri, "Harbor/Charts/north.txt", &fixture.cancel)
        .expect("the file opens");
    assert_eq!(fs::read_to_string(copy.path).unwrap(), "North channel");
}

/// Extracting it creates the folders the backslashes name.
///
/// parity: ARC-014, ARC-019
#[test]
fn backslashes_separate_folders_when_extracting() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("Harbor\\Charts\\north.txt", b"North channel"),
        TestMember::file("Harbor\\tides.txt", b"High water at noon."),
    ]);

    fixture.extract("Harbor copy").expect("extracts");

    let folder = fixture.destination.join("Harbor copy/Harbor");
    assert_eq!(
        fs::read_to_string(folder.join("Charts/north.txt")).unwrap(),
        "North channel"
    );
    assert_eq!(
        fs::read_to_string(folder.join("tides.txt")).unwrap(),
        "High water at noon."
    );
}

/// The folders a backslash gives are checked like any others: a parent
/// folder, an absolute or drive path and a UNC path are still refused, and
/// a name written both ways is a duplicate.
///
/// parity: ARC-004, ARC-014, ARC-015
#[test]
fn unsafe_paths_written_with_backslashes_are_still_refused() {
    for (name, refusal) in [
        ("..\\..\\outside.txt", ArchiveError::PathUnsafeForShares),
        ("Harbor\\..\\..\\outside.txt", ArchiveError::PathUnsafeForShares),
        ("\\outside.txt", ArchiveError::UnsafeMemberPath),
        ("\\\\server\\share\\outside.txt", ArchiveError::UnsafeMemberPath),
        ("C:\\outside.txt", ArchiveError::PathUnsafeForShares),
    ] {
        let fixture = ExtractionFixture::new();
        fixture.write_zip(&[TestMember::file(name, b"x")]);

        let listing = browser(&fixture)
            .list(&fixture.archive_uri(), "", &fixture.cancel)
            .expect("lists");
        assert_eq!(
            (listing.entries.len(), listing.hidden_unsafe_count),
            (0, 1),
            "{name}"
        );
        let error = fixture.extract("Out").unwrap_err();
        assert_eq!(error, refusal, "{name}");
        fixture.assert_no_output();
    }

    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("Harbor\\tides.txt", b"a"),
        TestMember::file("Harbor/tides.txt", b"b"),
    ]);
    assert_eq!(fixture.extract("Out").unwrap_err(), ArchiveError::DuplicateNames);
    fixture.assert_no_output();
}
