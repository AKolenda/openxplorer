// SPDX-License-Identifier: AGPL-3.0-only
//! Browsing a ZIP read-only and opening one member as a private copy.
//! Ports `ZipTests` of `desktop/tests/test_v05.py` and the ZIP cases of
//! `AdditionalSecurityTests` in `desktop/tests/test_terminal_security.py`
//! (the name rules alone are unit tests of `member_names`).

mod archive_support;

use std::fs;
use std::path::{Path, PathBuf};

use ox_core::archive::{
    ArchiveBrowser, ArchiveEntryKind, ArchiveError, ArchiveListing, PreviewCopy, ZipFormatError,
    MAX_LISTED_ENTRIES, PREVIEW_NOTICE,
};
use ox_core::transfer::Cancellation;

use archive_support::{
    file_type, file_uri, memory_opener, mode_of, opener, write_archive_declaring_directory, zip_bytes,
    Compression, TestMember, ENCRYPTED_FLAG,
};

/// The archive and preview folder of `ZipTests.setUp` in
/// `desktop/tests/test_v05.py`.
struct BrowseFixture {
    _temporary: tempfile::TempDir,
    archive: PathBuf,
    previews: PathBuf,
    browser: ArchiveBrowser,
    cancel: Cancellation,
}

impl BrowseFixture {
    /// A fixture whose archive is `bank.zip` holding `members`.
    fn with_members(members: &[TestMember]) -> Self {
        let temporary = tempfile::tempdir().expect("create a temporary folder");
        let archive = temporary.path().join("bank.zip");
        let previews = temporary.path().join("previews");
        fs::write(&archive, zip_bytes(members)).expect("write the archive");
        Self {
            browser: ArchiveBrowser::new(opener(), previews.clone()),
            _temporary: temporary,
            archive,
            previews,
            cancel: Cancellation::new(),
        }
    }

    /// The archive of `setUp`, plus `extra` members `ZipTests` appends.
    fn bank(extra: &[TestMember]) -> Self {
        let mut members = vec![
            TestMember::file("Documents/bank.txt", b"old statement").compressed_with(Compression::Stored),
            TestMember::file("readme.txt", b"hello").compressed_with(Compression::Stored),
            TestMember::file("../escape.txt", b"bad").compressed_with(Compression::Stored),
        ];
        members.extend_from_slice(extra);
        Self::with_members(&members)
    }

    fn list(&self, prefix: &str) -> Result<ArchiveListing, ArchiveError> {
        self.browser.list(&file_uri(&self.archive), prefix, &self.cancel)
    }

    fn preview(&self, member: &str) -> Result<PreviewCopy, ArchiveError> {
        self.browser
            .preview_member(&file_uri(&self.archive), member, &self.cancel)
    }
}

/// A browser that reads `archive` from memory, like the `BytesIO` opener
/// of `test_zip_preview_rejects_special_members`.
fn in_memory_browser(archive: Vec<u8>, previews: &Path) -> ArchiveBrowser {
    ArchiveBrowser::new(memory_opener(archive), previews.to_path_buf())
}

/// Every file below `folder`.
fn files_below(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder).expect("list the folder") {
        let path = entry.expect("a folder entry").path();
        if path.is_dir() {
            files.extend(files_below(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_listing_extracts_nothing`.
///
/// parity: ARC-003
#[test]
fn listing_extracts_nothing() {
    let fixture = BrowseFixture::bank(&[]);

    fixture.list("").expect("the archive lists");

    assert!(!fixture.previews.exists());
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_nested_virtual_directory`.
///
/// parity: ARC-003
#[test]
fn a_folder_inside_the_archive_lists_its_members() {
    let fixture = BrowseFixture::bank(&[]);

    let listing = fixture.list("Documents/").expect("the folder lists");

    assert_eq!(listing.entries[0].name, "bank.txt");
    assert_eq!(listing.prefix, "Documents/");
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_dangerous_members_hidden`.
///
/// parity: ARC-004
#[test]
fn dangerous_members_are_hidden_and_counted() {
    let fixture = BrowseFixture::bank(&[]);

    let listing = fixture.list("").expect("the archive lists");

    assert_eq!(listing.hidden_unsafe_count, 1);
    assert!(listing.entries.iter().all(|entry| !entry.member.contains("..")));
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_selected_member_only_temp_copy`.
///
/// parity: ARC-006
#[test]
fn only_the_selected_member_is_copied_read_only() {
    let fixture = BrowseFixture::bank(&[]);

    let copy = fixture.preview("Documents/bank.txt").expect("the member opens");

    assert_eq!(
        fs::read_to_string(&copy.path).expect("read the copy"),
        "old statement"
    );
    assert_eq!(files_below(&fixture.previews), std::slice::from_ref(&copy.path));
    assert_eq!(mode_of(&copy.path), 0o400);
    let folder = copy.path.parent().expect("the copy is in its own folder");
    assert_eq!(mode_of(folder), 0o700);
    assert_eq!(copy.member, "Documents/bank.txt");
    assert_eq!(copy.uri(), file_uri(&copy.path));
    assert_eq!(
        PREVIEW_NOTICE,
        "Opened a read-only temporary copy. Changes are NOT saved back into the ZIP."
    );
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_traversal_rejected`.
///
/// parity: ARC-006
#[test]
fn a_traversal_member_is_not_opened() {
    let fixture = BrowseFixture::bank(&[]);

    let result = fixture.preview("../escape.txt");

    assert_eq!(result.unwrap_err(), ArchiveError::NotARegularMember);
    assert!(!fixture.previews.exists());
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_script_preview_rejected`.
///
/// parity: ARC-006
#[test]
fn a_script_is_not_opened() {
    let fixture = BrowseFixture::bank(&[TestMember::file("run.sh", b"bad")]);

    let result = fixture.preview("run.sh");

    assert_eq!(result.unwrap_err(), ArchiveError::UnsafePreviewType);
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_symlink_hidden_and_rejected`.
///
/// parity: ARC-004, ARC-006
#[test]
fn a_link_is_hidden_and_not_opened() {
    let link = TestMember::with_unix_mode("link.txt", file_type::SYMLINK | 0o777, b"/etc/passwd");
    let fixture = BrowseFixture::bank(&[link]);

    let listing = fixture.list("").expect("the archive lists");
    let result = fixture.preview("link.txt");

    assert_eq!(listing.hidden_unsafe_count, 2);
    assert_eq!(result.unwrap_err(), ArchiveError::MemberNotPreviewable);
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_duplicate_member_preview_rejected`.
///
/// parity: ARC-006
#[test]
fn a_duplicated_member_is_not_opened() {
    let fixture = BrowseFixture::bank(&[TestMember::file("readme.txt", b"different")]);

    let result = fixture.preview("readme.txt");

    assert_eq!(result.unwrap_err(), ArchiveError::MissingOrDuplicatedMember);
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_invalid_zip_reports_error`.
///
/// parity: ARC-005
#[test]
fn a_file_that_is_not_a_zip_reports_a_bad_zip() {
    let fixture = BrowseFixture::bank(&[]);
    fs::write(&fixture.archive, "not a zip").expect("overwrite the archive");

    let error = fixture.list("").unwrap_err();

    assert_eq!(error, ArchiveError::Format(ZipFormatError::NotAZip));
    assert_eq!(error.to_string(), "File is not a zip file");
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_central_directory_allocation_bounded`.
/// Python checked its bounded reader alone; here a whole listing stops
/// before the directory is read.
///
/// parity: ARC-005
#[test]
fn a_central_directory_over_32_mib_is_not_read() {
    let fixture = BrowseFixture::bank(&[]);
    write_archive_declaring_directory(&fixture.archive, 32 * 1024 * 1024 + 1);

    let error = fixture.list("").unwrap_err();

    assert_eq!(error, ArchiveError::DirectoryTooLarge);
    assert_eq!(
        error.to_string(),
        "ZIP directory is too large for the built-in viewer. Use an archive manager."
    );
}

/// ARC-005: `Archives.opened` in `desktop/archives.py` refuses more than
/// 100,000 members.
///
/// parity: ARC-005
#[test]
fn more_than_100000_members_are_left_to_an_archive_manager() {
    let temporary = tempfile::tempdir().expect("create a temporary folder");
    let members: Vec<TestMember> = (0..=100_000)
        .map(|index| TestMember::file(&format!("f{index}"), b"").compressed_with(Compression::Stored))
        .collect();
    let browser = in_memory_browser(zip_bytes(&members), temporary.path());

    let error = browser
        .list("file:///many.zip", "", &Cancellation::new())
        .unwrap_err();

    assert_eq!(error, ArchiveError::TooManyMembers);
    assert_eq!(
        error.to_string(),
        "The archive has more than 100,000 members. Use an archive manager."
    );
}

/// Ported from `desktop/tests/test_v05.py::ZipTests::test_empty_zip`.
///
/// parity: ARC-003
#[test]
fn an_empty_archive_lists_nothing() {
    let fixture = BrowseFixture::with_members(&[]);

    let listing = fixture.list("").expect("the archive lists");

    assert_eq!(listing.entries, []);
    assert!(!listing.is_truncated);
}

/// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_zip_preview_rejects_special_members`.
///
/// parity: ARC-004, ARC-006
#[test]
fn fifos_devices_and_links_are_hidden_and_not_opened() {
    let temporary = tempfile::tempdir().expect("create a temporary folder");
    let members = [
        TestMember::with_unix_mode("pipe.txt", file_type::FIFO | 0o600, b"contents"),
        TestMember::with_unix_mode("device.txt", file_type::CHARACTER_DEVICE | 0o600, b"contents"),
        TestMember::with_unix_mode("link.txt", file_type::SYMLINK | 0o600, b"contents"),
        TestMember::file("ordinary.txt", b"ordinary"),
    ];
    let browser = in_memory_browser(zip_bytes(&members), temporary.path());
    let cancel = Cancellation::new();

    let listing = browser
        .list("file:///sample.zip", "", &cancel)
        .expect("the archive lists");
    let result = browser.preview_member("file:///sample.zip", "pipe.txt", &cancel);

    let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["ordinary.txt"]);
    assert_eq!(listing.hidden_unsafe_count, 3);
    assert_eq!(result.unwrap_err(), ArchiveError::MemberNotPreviewable);
}

/// ARC-003: folders first, then files, each ignoring case, with the sizes
/// and member paths the browser shows and opens.
///
/// parity: ARC-003
#[test]
fn rows_list_folders_first_by_name_ignoring_case() {
    let fixture = BrowseFixture::with_members(&[
        TestMember::file("b.txt", b"12345"),
        TestMember::file("Apps/tool.txt", b"x"),
        TestMember::file("A.txt", b"123"),
        TestMember::folder("zeta/"),
    ]);

    let listing = fixture.list("").expect("the archive lists");

    let rows: Vec<(&str, &str)> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.member.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Apps", "Apps/"),
            ("zeta", "zeta/"),
            ("A.txt", "A.txt"),
            ("b.txt", "b.txt")
        ]
    );
    assert_eq!(listing.entries[0].kind, ArchiveEntryKind::Folder);
    let ArchiveEntryKind::File { size, .. } = listing.entries[3].kind else {
        panic!("b.txt is a file");
    };
    assert_eq!(size, 5);
    assert!(listing.entries[3].can_open && !listing.entries[0].can_open);
}

/// ARC-003: a listing stops at 5,000 rows and says so.
///
/// parity: ARC-003
#[test]
fn a_listing_stops_at_5000_rows() {
    let temporary = tempfile::tempdir().expect("create a temporary folder");
    let members: Vec<TestMember> = (0..=MAX_LISTED_ENTRIES)
        .map(|index| TestMember::file(&format!("file {index}.txt"), b"").compressed_with(Compression::Stored))
        .collect();
    let browser = in_memory_browser(zip_bytes(&members), temporary.path());

    let listing = browser
        .list("file:///many.zip", "", &Cancellation::new())
        .expect("lists");

    assert_eq!(listing.entries.len(), 5000);
    assert!(listing.is_truncated);
}

/// ARC-004: a name that hides text after a NUL is hidden and counted.
///
/// parity: ARC-004
#[test]
fn a_name_hiding_text_after_a_nul_is_hidden() {
    let hidden = TestMember::file("notes.txt", b"x").named_raw(b"notes.txt\0.exe");
    let fixture = BrowseFixture::with_members(&[hidden]);

    let listing = fixture.list("").expect("the archive lists");

    assert_eq!(listing.entries, []);
    assert_eq!(listing.hidden_unsafe_count, 1);
}

/// `Archives.list` refuses an unsafe folder to list.
///
/// parity: ARC-004
#[test]
fn an_unsafe_folder_is_not_listed() {
    let fixture = BrowseFixture::bank(&[]);

    for prefix in ["../", "/etc", "C:/", "a//b"] {
        assert_eq!(
            fixture.list(prefix).unwrap_err(),
            ArchiveError::InvalidFolder,
            "{prefix}"
        );
    }
}

/// ARC-006: encrypted members, and members too large or compressed too
/// much, are listed but cannot be opened.
///
/// parity: ARC-006
#[test]
fn encrypted_and_oversized_members_are_listed_but_not_opened() {
    let encrypted = TestMember::file("secret.txt", b"x").with_flags(ENCRYPTED_FLAG);
    let bomb = TestMember::file("bomb.txt", &vec![b'0'; 50_000]).declaring_size(256 * 1024 * 1024 + 1);
    let fixture = BrowseFixture::with_members(&[encrypted, bomb]);

    let listing = fixture.list("").expect("the archive lists");

    assert!(listing.entries.iter().all(|entry| !entry.can_open));
    assert!(listing.entries.iter().any(|entry| entry.is_encrypted));
    assert_eq!(
        fixture.preview("secret.txt").unwrap_err(),
        ArchiveError::EncryptedMember
    );
    assert_eq!(
        fixture.preview("bomb.txt").unwrap_err(),
        ArchiveError::MemberNotPreviewable
    );
    assert!(!fixture.previews.exists());
}

/// ARC-006: a folder or a missing member is not opened.
///
/// parity: ARC-006
#[test]
fn folders_and_missing_members_are_not_opened() {
    let fixture = BrowseFixture::bank(&[]);

    assert_eq!(
        fixture.preview("Documents/").unwrap_err(),
        ArchiveError::NotARegularMember
    );
    assert_eq!(
        fixture.preview("missing.txt").unwrap_err(),
        ArchiveError::MissingOrDuplicatedMember
    );
}

/// ARC-006: each opened member gets its own new private folder, so a
/// second copy never replaces the first.
///
/// parity: ARC-006
#[test]
fn opening_a_member_twice_makes_two_copies() {
    let fixture = BrowseFixture::bank(&[]);

    let first = fixture.preview("readme.txt").expect("the member opens");
    let second = fixture.preview("readme.txt").expect("the member opens again");

    assert_ne!(first.path, second.path);
    assert_eq!(files_below(&fixture.previews).len(), 2);
}
