// SPDX-License-Identifier: AGPL-3.0-only
//! The checks every member passes before an extraction writes anything:
//! unsafe paths, ambiguous names, links and special files, encryption,
//! compression methods and the limits against ZIP bombs. Ports those cases
//! of `ZipExtractTests` in `desktop/tests/test_zip_extract.py`; the
//! archives that `test_encrypted_flag`, `test_nul_truncation` and
//! `test_unsupported_compression` changed in memory are written with the
//! changed fields instead.

mod archive_support;

use ox_core::archive::{ArchiveError, ExtractionLimits};

use archive_support::{file_type, Compression, ExtractionFixture, TestMember, ENCRYPTED_FLAG};

/// Member paths that all meet the same refusal.
struct UnsafePaths {
    names: &'static [&'static str],
    expected: ArchiveError,
}

/// Extracts the fixture's archive as `Unpacked` and returns the refusal,
/// after checking that nothing was written.
fn refusal(fixture: &ExtractionFixture) -> ArchiveError {
    let error = fixture.extract("Unpacked").expect_err("the archive is refused");
    fixture.assert_no_output();
    error
}

/// Extracts the fixture's archive with `limits` and returns the refusal,
/// after checking that nothing was written.
fn refusal_with(fixture: &ExtractionFixture, limits: ExtractionLimits) -> ArchiveError {
    let mut extractor = fixture.extractor().with_limits(limits);
    let error = fixture
        .extract_with(&mut extractor, "Unpacked")
        .expect_err("the archive is refused");
    fixture.assert_no_output();
    error
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_unsafe_paths`.
/// Python checked only that each is refused; here each refusal is named.
///
/// parity: ARC-014
#[test]
fn unsafe_member_paths_are_refused_with_nothing_written() {
    let groups = [
        UnsafePaths {
            names: &["/etc/x", "//server/share", "a\\evil", "a\u{1}b"],
            expected: ArchiveError::UnsafeMemberPath,
        },
        UnsafePaths {
            names: &[
                "../outside",
                "a/../../outside",
                "C:/file",
                "a/./b",
                "a//b",
                "a:stream",
                "a/..",
                "a.",
                "a ",
            ],
            expected: ArchiveError::PathUnsafeForShares,
        },
        UnsafePaths {
            names: &["CON", "a/NUL.txt"],
            expected: ArchiveError::ReservedDeviceName,
        },
    ];
    let fixture = ExtractionFixture::new();
    for group in groups {
        for name in group.names {
            fixture.write_zip(&[
                TestMember::file("good.txt", b"good"),
                TestMember::file(name, b"bad"),
            ]);

            assert_eq!(refusal(&fixture), group.expected, "{name:?}");
        }
    }
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_filename_size_limit`.
///
/// parity: ARC-014, ARC-017
#[test]
fn a_segment_over_255_bytes_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file(&"x".repeat(256), b"data")]);

    assert_eq!(refusal(&fixture), ArchiveError::PathUnsafeForShares);
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_nul_truncation`.
///
/// parity: ARC-014
#[test]
fn a_name_hiding_text_after_a_nul_is_invalid() {
    let fixture = ExtractionFixture::new();
    let hidden = TestMember::folder("Docs/").named_raw(b"Docs/\0suffix");
    fixture.write_zip(&[hidden, TestMember::file("Docs/Guide.txt", b"hello world")]);

    let error = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .unwrap_err();

    assert_eq!(error, ArchiveError::InvalidMemberName);
    assert!(error.to_string().contains("invalid"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_duplicate_names`.
///
/// parity: ARC-014, ARC-015
#[test]
fn duplicate_names_are_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::file("a.txt", b"first"),
        TestMember::file("a.txt", b"second"),
    ]);

    let error = refusal(&fixture);

    assert_eq!(error, ArchiveError::DuplicateNames);
    assert!(error.to_string().contains("duplicate"));
}

/// Two member paths and the refusal they meet together.
struct AliasCase {
    first: &'static str,
    second: &'static str,
    expected: ArchiveError,
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_case_and_unicode_aliases`.
///
/// parity: ARC-015
#[test]
fn names_differing_only_in_case_or_unicode_form_are_refused() {
    let cases = [
        AliasCase {
            first: "Note.txt",
            second: "note.txt",
            expected: ArchiveError::DuplicateNames,
        },
        AliasCase {
            first: "Docs/a",
            second: "docs/b",
            expected: ArchiveError::AmbiguousPaths,
        },
        AliasCase {
            first: "Café/a",
            second: "Cafe\u{301}/b",
            expected: ArchiveError::AmbiguousPaths,
        },
    ];
    let fixture = ExtractionFixture::new();
    for case in cases {
        fixture.write_zip(&[
            TestMember::file(case.first, b"a"),
            TestMember::file(case.second, b"b"),
        ]);

        assert_eq!(
            refusal(&fixture),
            case.expected,
            "{} and {}",
            case.first,
            case.second
        );
    }
}

/// Two file members, one of whose paths is a folder of the other.
struct ConflictCase {
    first: &'static str,
    second: &'static str,
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_file_directory_conflicts`.
///
/// parity: ARC-015
#[test]
fn a_path_used_as_file_and_folder_is_refused_in_either_order() {
    let cases = [
        ConflictCase {
            first: "a",
            second: "a/b",
        },
        ConflictCase {
            first: "a/b",
            second: "a",
        },
    ];
    let fixture = ExtractionFixture::new();
    for case in cases {
        fixture.write_zip(&[
            TestMember::file(case.first, b"f"),
            TestMember::file(case.second, b"g"),
        ]);

        assert_eq!(
            refusal(&fixture),
            ArchiveError::AmbiguousPaths,
            "{} and {}",
            case.first,
            case.second
        );
    }
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_archive_symlink`.
///
/// parity: ARC-014, ARC-016
#[test]
fn a_symbolic_link_member_is_refused() {
    let fixture = ExtractionFixture::new();
    let link = TestMember::with_unix_mode("link", file_type::SYMLINK | 0o777, b"../../outside");
    fixture.write_zip(&[link]);

    let error = refusal(&fixture);

    assert_eq!(error, ArchiveError::LinkOrSpecialFile);
    assert!(error.to_string().contains("symbolic link"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_special_files`.
///
/// parity: ARC-016
#[test]
fn fifos_devices_and_sockets_are_refused() {
    let fixture = ExtractionFixture::new();
    let special_types = [
        file_type::FIFO,
        file_type::CHARACTER_DEVICE,
        file_type::BLOCK_DEVICE,
        file_type::SOCKET,
    ];
    for special_type in special_types {
        fixture.write_zip(&[TestMember::with_unix_mode("special", special_type | 0o600, b"")]);

        let error = refusal(&fixture);

        assert_eq!(error, ArchiveError::LinkOrSpecialFile, "{special_type:o}");
        assert!(error.to_string().contains("special file"));
    }
}

/// A file entry with a folder's mode is refused like a special file, as
/// `member_parts` in `desktop/zip_extraction.py` does.
///
/// parity: ARC-016
#[test]
fn a_file_entry_with_a_folder_mode_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::with_unix_mode(
        "folder",
        file_type::DIRECTORY | 0o755,
        b"",
    )]);

    assert_eq!(refusal(&fixture), ArchiveError::LinkOrSpecialFile);
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_encrypted_flag`.
///
/// parity: ARC-016
#[test]
fn an_encrypted_member_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[
        TestMember::folder("Docs/"),
        TestMember::file("Docs/Guide.txt", b"hello world").with_flags(ENCRYPTED_FLAG),
    ]);

    let error = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .unwrap_err();

    assert_eq!(error, ArchiveError::PasswordProtected);
    assert!(error.to_string().starts_with("Password-protected"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_directory_payload`.
///
/// parity: ARC-016
#[test]
fn a_folder_entry_with_data_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("bad/", b"payload")]);

    assert_eq!(refusal(&fixture), ArchiveError::InconsistentSizes);
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_unsupported_compression`.
///
/// parity: ARC-016
#[test]
fn an_unsupported_compression_method_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::folder("Docs/").compressed_with(Compression::Unsupported(99))]);

    let error = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .unwrap_err();

    assert_eq!(error, ArchiveError::UnsupportedCompression);
    assert!(error.to_string().contains("compression"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_inspection_writes_nothing`.
///
/// parity: ARC-008, ARC-009
#[test]
fn inspection_summarises_and_writes_nothing() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();

    let summary = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .expect("inspected");

    assert_eq!(summary.file_count, 2);
    assert_eq!(
        (summary.folder_count, summary.unpacked_bytes, summary.entry_count),
        (1, 11, 3)
    );
    fixture.assert_no_output();
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_entries_limit`.
///
/// parity: ARC-017
#[test]
fn more_entries_than_the_limit_are_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let limits = ExtractionLimits {
        max_entries: 2,
        ..ExtractionLimits::default()
    };

    let error = refusal_with(&fixture, limits);

    assert_eq!(error, ArchiveError::TooManyEntries);
    assert!(error.to_string().contains("entries"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_paths_limit`.
///
/// parity: ARC-017
#[test]
fn implied_folders_count_toward_the_paths_limit() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("a/b/c/d.txt", b"data")]);
    let limits = ExtractionLimits {
        max_paths: 3,
        ..ExtractionLimits::default()
    };

    let error = refusal_with(&fixture, limits);

    assert_eq!(error, ArchiveError::TooManyPaths);
    assert!(error.to_string().contains("paths"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_depth_limit`.
///
/// parity: ARC-017
#[test]
fn nesting_deeper_than_128_levels_is_refused() {
    let fixture = ExtractionFixture::new();
    let deep = format!("{}/a", vec!["folder"; 129].join("/"));
    fixture.write_zip(&[TestMember::file(&deep, b"")]);

    let error = refusal(&fixture);

    assert_eq!(error, ArchiveError::NestingTooDeep);
    assert!(error.to_string().contains("nesting"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_member_bytes_limit`.
///
/// parity: ARC-017
#[test]
fn a_member_over_the_size_limit_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("large", b"0123456789")]);
    let limits = ExtractionLimits {
        max_member_bytes: 5,
        ..ExtractionLimits::default()
    };

    let error = refusal_with(&fixture, limits);

    assert_eq!(error, ArchiveError::MemberTooLarge);
    assert!(error.to_string().contains("decompression"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_total_bytes_limit`.
///
/// parity: ARC-017
#[test]
fn members_over_the_total_limit_are_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("a", b"1234"), TestMember::file("b", b"1234")]);
    let limits = ExtractionLimits {
        max_total_bytes: 7,
        ..ExtractionLimits::default()
    };

    let error = refusal_with(&fixture, limits);

    assert_eq!(error, ArchiveError::ArchiveTooLarge);
    assert!(error.to_string().contains("extraction limit"));
}

/// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_ratio_limit`.
///
/// parity: ARC-017
#[test]
fn a_member_compressed_beyond_the_ratio_limit_is_refused() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("large", &vec![b'0'; 50_000])]);
    let limits = ExtractionLimits {
        max_ratio: 3,
        ..ExtractionLimits::default()
    };

    let error = refusal_with(&fixture, limits);

    assert_eq!(error, ArchiveError::MemberTooLarge);
    assert!(error.to_string().contains("decompression"));
}

/// A member whose data ends before its declared size, with a matching
/// checksum, stops the extraction (`written != item.file_size` in
/// `desktop/zip_extraction.py`).
///
/// parity: ARC-017
#[test]
fn a_member_shorter_than_it_declares_stops_the_extraction() {
    let fixture = ExtractionFixture::new();
    let short = TestMember::file("short.txt", b"abcd")
        .compressed_with(Compression::Stored)
        .declaring_size(8);
    fixture.write_zip(&[short]);

    let error = refusal(&fixture);

    assert_eq!(error, ArchiveError::TruncatedMember);
    assert_eq!(
        error.to_string(),
        "ZIP member has a truncated size. Extraction stopped."
    );
}
