// SPDX-License-Identifier: AGPL-3.0-only
//! The ZIP structures Python's `zipfile` reads, which the Python app relied
//! on: archive comments, bytes in front of an archive (self-extracting
//! archives), ZIP64 end records and code page 437 names; and the damaged
//! or forged archives it refuses. These rules came with `zipfile`, so there
//! is no Python test to port; these tests hold the ZIP reader of
//! `ox_core::archive` to `zipfile`'s behaviour.

#[path = "archive_support.rs"]
mod support;

use std::fs;

use ox_core::archive::{ArchiveBrowser, ArchiveError, ZipFormatError};
use ox_core::transfer::Cancellation;

use support::{
    memory_opener, zip_bytes, zip_bytes_with, ArchiveLayout, Compression, EndRecords, ExtractionFixture,
    TestMember, UTF8_NAME_FLAG,
};

/// Lists the top of `archive`, held in memory, and returns the names.
fn top_names(archive: Vec<u8>) -> Result<Vec<String>, ArchiveError> {
    let previews = tempfile::tempdir().expect("create a temporary folder");
    let browser = ArchiveBrowser::new(memory_opener(archive), previews.path().to_path_buf());
    let listing = browser.list("file:///archive.zip", "", &Cancellation::new())?;
    Ok(listing.entries.into_iter().map(|entry| entry.name).collect())
}

/// Opens `member` of `archive`, held in memory, and returns its text.
fn opened_text(archive: Vec<u8>, member: &str) -> Result<String, ArchiveError> {
    let previews = tempfile::tempdir().expect("create a temporary folder");
    let browser = ArchiveBrowser::new(memory_opener(archive), previews.path().to_path_buf());
    let copy = browser.preview_member("file:///archive.zip", member, &Cancellation::new())?;
    Ok(fs::read_to_string(copy.path).expect("read the copy"))
}

/// A readable two-member archive laid out as `layout` says.
fn notes_archive(layout: &ArchiveLayout) -> Vec<u8> {
    let members = [
        TestMember::file("notes.txt", b"first notes"),
        TestMember::file("more.txt", b"more notes").compressed_with(Compression::Bzip2),
    ];
    zip_bytes_with(&members, layout)
}

/// parity: ARC-003
#[test]
fn an_archive_comment_is_skipped() {
    let layout = ArchiveLayout {
        comment: b"Made by the archive tests".repeat(100),
        ..ArchiveLayout::default()
    };

    let names = top_names(notes_archive(&layout)).expect("the archive lists");
    let text = opened_text(notes_archive(&layout), "more.txt").expect("the member opens");

    assert_eq!(names, ["more.txt", "notes.txt"]);
    assert_eq!(text, "more notes");
}

/// A self-extracting archive: a program in front of the ZIP, whose
/// recorded offsets do not count it.
///
/// parity: ARC-003
#[test]
fn bytes_in_front_of_the_archive_are_skipped() {
    let layout = ArchiveLayout {
        prefix: b"MZ a self-extracting stub".repeat(20),
        ..ArchiveLayout::default()
    };

    let text = opened_text(notes_archive(&layout), "notes.txt").expect("the member opens");

    assert_eq!(text, "first notes");
}

/// parity: ARC-003
#[test]
fn zip64_end_records_locate_the_directory() {
    let layout = ArchiveLayout {
        end_records: EndRecords::Zip64,
        ..ArchiveLayout::default()
    };

    let names = top_names(notes_archive(&layout)).expect("the archive lists");
    let text = opened_text(notes_archive(&layout), "notes.txt").expect("the member opens");

    assert_eq!(names, ["more.txt", "notes.txt"]);
    assert_eq!(text, "first notes");
}

/// `zipfile` finds a ZIP64 end record right before its locator when bytes
/// in front of the archive moved it from its recorded offset.
///
/// parity: ARC-003
#[test]
fn zip64_end_records_are_found_after_bytes_in_front() {
    let layout = ArchiveLayout {
        prefix: vec![0; 100],
        end_records: EndRecords::Zip64,
        ..ArchiveLayout::default()
    };

    let text = opened_text(notes_archive(&layout), "more.txt").expect("the member opens");

    assert_eq!(text, "more notes");
}

/// Names without the UTF-8 flag are code page 437, the historical ZIP
/// encoding, as `zipfile` reads them.
///
/// parity: ARC-003
#[test]
fn names_without_the_utf8_flag_are_code_page_437() {
    let archive = zip_bytes(&[TestMember::file("", b"x").named_raw(b"Caf\x82 \x9c.txt")]);

    let names = top_names(archive).expect("the archive lists");

    assert_eq!(names, ["Café £.txt"]);
}

/// parity: ARC-005
#[test]
fn a_name_flagged_as_utf8_that_is_not_is_a_damaged_zip() {
    let member = TestMember::file("", b"x")
        .named_raw(b"Caf\x82.txt")
        .with_flags(UTF8_NAME_FLAG);

    let error = top_names(zip_bytes(&[member])).unwrap_err();

    assert_eq!(error, ArchiveError::Format(ZipFormatError::NameNotUtf8));
}

/// parity: ARC-005
#[test]
fn a_member_needing_a_newer_zip_version_is_a_bad_zip() {
    let member = TestMember::file("new.txt", b"x").needing_version(64);

    let error = top_names(zip_bytes(&[member])).unwrap_err();

    assert_eq!(
        error,
        ArchiveError::Format(ZipFormatError::UnsupportedVersion(64))
    );
    assert_eq!(error.to_string(), "zip file version 6.4");
}

/// A local header that names another file than the central directory is
/// how an archive can show one file and extract another.
///
/// parity: ARC-006
#[test]
fn a_local_header_naming_another_file_is_refused() {
    let member = TestMember::file("report.txt", b"x").with_local_name("report.exe");

    let error = opened_text(zip_bytes(&[member]), "report.txt").unwrap_err();

    let expected = ZipFormatError::NameMismatch {
        directory: "report.txt".to_owned(),
        header: "report.exe".to_owned(),
    };
    assert_eq!(error, ArchiveError::Format(expected));
}

/// Member data that runs into the next member is the overlapping-members
/// ZIP bomb; `zipfile` refuses it and the extraction rolls back.
///
/// parity: ARC-013, ARC-017
#[test]
fn data_running_into_the_next_member_is_refused_as_a_zip_bomb() {
    let fixture = ExtractionFixture::new();
    let overlapping = TestMember::file("a.txt", b"0123456789")
        .compressed_with(Compression::Stored)
        .declaring_compressed_size(1000);
    fs::write(
        &fixture.archive,
        zip_bytes(&[overlapping, TestMember::file("b.txt", b"b")]),
    )
    .expect("write");

    let error = fixture.extract("Unpacked").unwrap_err();

    let expected = ZipFormatError::OverlappedEntries("a.txt".to_owned());
    assert_eq!(error, ArchiveError::Format(expected));
    assert!(error.to_string().contains("possible zip bomb"));
    fixture.assert_no_output();
}

/// Deflate data that cannot be decompressed names its member.
///
/// parity: ARC-013
#[test]
fn undecompressable_data_rolls_back_naming_the_member() {
    let fixture = ExtractionFixture::new();
    let mut archive = zip_bytes(&[TestMember::file("doc.txt", b"contents contents contents")]);
    let data_start = support::first_member_data_offset("doc.txt");
    archive[data_start] = 0xff;
    fs::write(&fixture.archive, archive).expect("write the damaged archive");

    let error = fixture.extract("Unpacked").unwrap_err();

    assert!(
        matches!(&error, ArchiveError::Format(ZipFormatError::CorruptData { name, .. }) if name == "doc.txt"),
        "{error:?}"
    );
    fixture.assert_no_output();
}
