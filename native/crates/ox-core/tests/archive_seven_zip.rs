// SPDX-License-Identifier: AGPL-3.0-only
//! 7z archives are browsed, opened and extracted by the rules of a ZIP or
//! a TAR, as Windows 11 reads 7z itself ("Native support for 7z, tar and
//! rar archive formats", Windows 11 version 23H2 and later).

mod archive_support;

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ox_core::archive::{
    is_supported_archive, suggested_folder_name, ArchiveBrowser, ArchiveEntryKind, ArchiveError,
    ArchiveOpener, ArchiveStream, ZipExtractor,
};
use ox_core::transfer::Cancellation;

use archive_support::{
    coded_seven_zip, encoded_header_seven_zip, gio_factory, memory_opener, one_file_seven_zip, raw_seven_zip,
    seven_zip_bytes, seven_zip_claiming_files, seven_zip_with_empty_file_between,
    seven_zip_without_sub_streams, ExtractionFixture, LocalFileOutput, SevenEntry, TestCoder, AES, BCJ2,
    BZIP2, COPY, DEFLATE, LZMA2, PPMD, SEVEN_ZIP_MODIFIED,
};

/// A folder, a file in it, a folder in that and an empty file, with a
/// file at the top.
fn sample_entries() -> Vec<SevenEntry> {
    vec![
        SevenEntry::folder("Docs"),
        SevenEntry::file("Docs/Guide.txt", b"hello world"),
        SevenEntry::folder("Docs/Deep"),
        SevenEntry::file("Docs/Deep/b.txt", b"deep data"),
        SevenEntry::file("Docs/Empty.txt", b""),
        SevenEntry::file("readme.txt", b"read me first"),
    ]
}

/// A browser over `bytes` held in memory, copying opened members into
/// `root`.
fn browser_of(bytes: Vec<u8>, root: &std::path::Path) -> ArchiveBrowser {
    ArchiveBrowser::new(memory_opener(bytes), root.to_path_buf())
}

/// The names and kinds of the listing of `prefix`.
fn listed(browser: &ArchiveBrowser, prefix: &str) -> Vec<(String, bool)> {
    browser
        .list("file:///fixture/Bundle.7z", prefix, &Cancellation::new())
        .expect("the 7z lists")
        .entries
        .into_iter()
        .map(|entry| (entry.name, entry.kind == ArchiveEntryKind::Folder))
        .collect()
}

/// A fixture whose archive is `sample.7z` holding `bytes`.
fn fixture_with(bytes: &[u8]) -> ExtractionFixture {
    let mut fixture = ExtractionFixture::new();
    fixture.archive = fixture.root.join("sample.7z");
    fs::write(&fixture.archive, bytes).expect("the 7z");
    fixture
}

/// parity: ARC-022
#[test]
fn a_7z_name_or_type_is_browsed_by_the_app_and_its_folder_drops_the_ending() {
    assert!(is_supported_archive("Bundle.7z", None));
    assert!(is_supported_archive("BUNDLE.7Z", None));
    assert!(is_supported_archive(
        "Bundle",
        Some("application/x-7z-compressed")
    ));
    assert_eq!(suggested_folder_name("Bundle.7z").expect("a name"), "Bundle");
    assert_eq!(suggested_folder_name("Site.7Z").expect("a name"), "Site");
}

/// parity: ARC-022
#[test]
fn a_7z_lists_its_folders_and_files_like_a_zip_solid_or_not() {
    for solid in [false, true] {
        let root = tempfile::tempdir().expect("a folder");
        let browser = browser_of(seven_zip_bytes(&sample_entries(), solid), root.path());

        let top = browser
            .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
            .expect("the 7z lists");
        assert_eq!(
            listed(&browser, ""),
            [("Docs".to_owned(), true), ("readme.txt".to_owned(), false)],
            "solid: {solid}"
        );
        let readme = &top.entries[1];
        assert!(
            matches!(readme.kind, ArchiveEntryKind::File { size: 13, compressed_size } if compressed_size > 0)
        );
        assert_eq!(readme.modified, Some(SEVEN_ZIP_MODIFIED));
        assert!(readme.can_open && !readme.is_encrypted);
        assert_eq!(top.hidden_unsafe_count, 0);
        assert_eq!(
            listed(&browser, "Docs/"),
            [
                ("Deep".to_owned(), true),
                ("Empty.txt".to_owned(), false),
                ("Guide.txt".to_owned(), false)
            ]
        );
        assert_eq!(listed(&browser, "Docs/Deep"), [("b.txt".to_owned(), false)]);
    }
}

/// A folder only named in a member's path is listed as a folder, and a
/// backslash separates folders, as 7-Zip reads it.
///
/// parity: ARC-022
#[test]
fn folders_named_only_in_paths_and_backslashes_are_folders() {
    let root = tempfile::tempdir().expect("a folder");
    let entries = [
        SevenEntry::file("Photos/2026/harbor.jpg", b"jpeg"),
        SevenEntry::file("Notes\\plan.txt", b"plan"),
    ];
    let browser = browser_of(seven_zip_bytes(&entries, true), root.path());

    assert_eq!(
        listed(&browser, ""),
        [("Notes".to_owned(), true), ("Photos".to_owned(), true)]
    );
    assert_eq!(listed(&browser, "Notes"), [("plan.txt".to_owned(), false)]);
}

/// parity: ARC-024
#[test]
fn a_7z_extracts_into_a_new_folder_solid_or_not() {
    for solid in [false, true] {
        let fixture = fixture_with(&seven_zip_bytes(&sample_entries(), solid));

        let extracted = fixture.extract("Unpacked").expect("the 7z extracts");

        let folder = fixture.destination.join("Unpacked");
        assert_eq!(
            fs::read(folder.join("Docs/Guide.txt")).expect("a file"),
            b"hello world"
        );
        assert_eq!(
            fs::read(folder.join("Docs/Deep/b.txt")).expect("a file"),
            b"deep data"
        );
        assert_eq!(fs::read(folder.join("Docs/Empty.txt")).expect("a file"), b"");
        assert_eq!(
            fs::read(folder.join("readme.txt")).expect("a file"),
            b"read me first"
        );
        assert_eq!(
            (extracted.summary.file_count, extracted.summary.folder_count),
            (4, 2),
            "solid: {solid}"
        );
    }
}

/// Members are read in any order: going back in a solid block starts it
/// again.
///
/// parity: ARC-006
#[test]
fn files_of_a_solid_7z_open_as_private_read_only_copies_in_any_order() {
    let root = tempfile::tempdir().expect("a folder");
    let browser = browser_of(seven_zip_bytes(&sample_entries(), true), root.path());
    let cancel = Cancellation::new();

    for (member, data) in [
        ("readme.txt", b"read me first".as_slice()),
        ("Docs/Guide.txt", b"hello world"),
        ("readme.txt", b"read me first"),
    ] {
        let copy = browser
            .preview_member("file:///fixture/Bundle.7z", member, &cancel)
            .expect("a copy");
        assert_eq!(fs::read(&copy.path).expect("the copy"), data);
        let mode = fs::metadata(&copy.path).expect("the copy").permissions().mode();
        assert_eq!(mode & 0o222, 0, "read-only");
    }
}

/// parity: ARC-004, ARC-014
#[test]
fn unsafe_7z_names_are_hidden_and_refuse_extraction() {
    for unsafe_name in [
        "../evil.txt",
        "/abs.txt",
        "C:/drive.txt",
        "..\\up.txt",
        "Docs/../../x.txt",
    ] {
        let entries = [
            SevenEntry::file("ok.txt", b"ok"),
            SevenEntry::file(unsafe_name, b"bad"),
        ];
        let bytes = seven_zip_bytes(&entries, true);
        let root = tempfile::tempdir().expect("a folder");
        let listing = browser_of(bytes.clone(), root.path())
            .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
            .expect("lists");
        let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["ok.txt"], "{unsafe_name}");
        assert_eq!(listing.hidden_unsafe_count, 1, "{unsafe_name}");

        let fixture = fixture_with(&bytes);
        assert!(fixture.extract("Unpacked").is_err(), "{unsafe_name}");
        fixture.assert_no_output();
    }
}

/// A symbolic link stored as p7zip stores it is hidden and refuses the
/// extraction, as a ZIP's link does.
///
/// parity: ARC-004, ARC-016
#[test]
fn a_7z_symbolic_link_is_hidden_and_refuses_extraction() {
    let entries = [
        SevenEntry::file("ok.txt", b"ok"),
        SevenEntry::link("escape", "/etc/passwd"),
    ];
    let bytes = seven_zip_bytes(&entries, false);
    let root = tempfile::tempdir().expect("a folder");
    let listing = browser_of(bytes.clone(), root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("lists");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.hidden_unsafe_count, 1);

    let fixture = fixture_with(&bytes);
    let error = fixture.extract("Unpacked").expect_err("refused");
    assert!(matches!(error, ArchiveError::LinkOrSpecialFile), "{error:?}");
    fixture.assert_no_output();
}

/// parity: ARC-017
#[test]
fn a_cut_or_damaged_7z_is_refused_and_leaves_nothing() {
    let bytes = seven_zip_bytes(&sample_entries(), true);
    let root = tempfile::tempdir().expect("a folder");
    for length in [3, 20, 40, bytes.len() / 2, bytes.len() - 1] {
        let cut = bytes[..length].to_vec();
        let error = browser_of(cut.clone(), root.path())
            .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
            .expect_err("a cut 7z");
        assert!(
            matches!(error, ArchiveError::DamagedSevenZip),
            "{length}: {error:?}"
        );
        let fixture = fixture_with(&cut);
        assert!(fixture.extract("Unpacked").is_err());
        fixture.assert_no_output();
    }

    // Damaged data under intact headers stops at the checksum.
    let mut damaged = bytes.clone();
    damaged[40] ^= 0x55;
    let fixture = fixture_with(&damaged);
    let error = fixture.extract("Unpacked").expect_err("damaged data");
    assert!(matches!(error, ArchiveError::DamagedSevenZip), "{error:?}");
    fixture.assert_no_output();
}

/// An encrypted 7z whose names are readable is listed; its files then
/// say plainly that the encryption isn't supported.
///
/// parity: ARC-016
#[test]
fn an_encrypted_7z_lists_and_says_its_encryption_is_not_supported() {
    let bytes = one_file_seven_zip("secret.txt", &AES, &[0x13, 0x00], &[0x42; 32]);
    let root = tempfile::tempdir().expect("a folder");
    let browser = browser_of(bytes.clone(), root.path());
    let listing = browser
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("the names are readable");
    assert_eq!(listing.entries.len(), 1);
    assert!(listing.entries[0].is_encrypted);

    let error = browser
        .preview_member("file:///fixture/Bundle.7z", "secret.txt", &Cancellation::new())
        .expect_err("not opened");
    assert!(matches!(error, ArchiveError::SevenZipEncryption), "{error:?}");
    let fixture = fixture_with(&bytes);
    let error = fixture.extract("Unpacked").expect_err("not extracted");
    assert!(matches!(error, ArchiveError::SevenZipEncryption), "{error:?}");
    fixture.assert_no_output();
}

/// parity: ARC-016
#[test]
fn a_7z_with_encrypted_names_says_so_before_listing() {
    let bytes = encoded_header_seven_zip(&AES, &[0x13, 0x00]);
    let root = tempfile::tempdir().expect("a folder");
    let error = browser_of(bytes, root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect_err("names unreadable");
    assert!(matches!(error, ArchiveError::SevenZipEncryption), "{error:?}");
}

/// parity: ARC-016
#[test]
fn a_7z_method_the_reader_lacks_is_listed_and_refused_plainly() {
    let bytes = one_file_seven_zip("notes.txt", &PPMD, &[6, 0, 0, 0, 1], &[0x42; 16]);
    let root = tempfile::tempdir().expect("a folder");
    let listing = browser_of(bytes.clone(), root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("lists");
    assert_eq!(listing.entries.len(), 1);
    assert!(!listing.entries[0].can_open);
    let fixture = fixture_with(&bytes);
    let error = fixture.extract("Unpacked").expect_err("refused");
    assert!(matches!(error, ArchiveError::SevenZipMethod), "{error:?}");
    fixture.assert_no_output();
}

/// Forged counts and sizes in a tiny header are refused before anything
/// that large is set aside.
///
/// parity: ARC-005
#[test]
fn forged_7z_counts_and_sizes_are_refused_without_reserving_them() {
    let root = tempfile::tempdir().expect("a folder");
    for count in [100_001, 1 << 40, u64::MAX] {
        let error = browser_of(seven_zip_claiming_files(count), root.path())
            .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
            .expect_err("refused");
        assert!(
            matches!(
                error,
                ArchiveError::TooManyMembers | ArchiveError::DamagedSevenZip
            ),
            "{count}: {error:?}"
        );
    }

    // A dictionary of 4 GiB.
    let bytes = one_file_seven_zip("big.txt", &LZMA2, &[40], &[0x42; 16]);
    let fixture = fixture_with(&bytes);
    let error = fixture.extract("Unpacked").expect_err("refused");
    assert!(matches!(error, ArchiveError::DecoderMemoryLimit), "{error:?}");
    fixture.assert_no_output();

    // A header that claims to be larger than the file.
    let mut header_too_long = raw_seven_zip(&[], &[0x01, 0x00]);
    header_too_long[20..28].copy_from_slice(&(1u64 << 40).to_le_bytes());
    let start_crc = crc32fast::hash(&header_too_long[12..32]);
    header_too_long[8..12].copy_from_slice(&start_crc.to_le_bytes());
    let error = browser_of(header_too_long, root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect_err("refused");
    assert!(matches!(error, ArchiveError::DamagedSevenZip), "{error:?}");
}

/// Counts what an opener hands out: streams opened and bytes read.
#[derive(Default)]
struct Reads {
    opened: AtomicUsize,
    bytes: AtomicUsize,
}

struct CountingStream {
    inner: std::io::Cursor<Vec<u8>>,
    reads: Arc<Reads>,
}

impl Read for CountingStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.reads.bytes.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

impl Seek for CountingStream {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

fn counting_opener(bytes: Vec<u8>, reads: &Arc<Reads>) -> Arc<dyn ArchiveOpener> {
    let reads = Arc::clone(reads);
    let opener = move |_uri: &str, _cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        reads.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingStream {
            inner: std::io::Cursor::new(bytes.clone()),
            reads: Arc::clone(&reads),
        }))
    };
    Arc::new(opener)
}

/// Browsing folder after folder of the same 7z reads its header once;
/// a changed archive is read again.
///
/// parity: ARC-003
#[test]
fn browsing_a_7z_again_reuses_its_header() {
    let entries: Vec<SevenEntry> = (0..2000)
        .map(|index| SevenEntry::file(&format!("Folder {}/file {index}.txt", index % 20), b"data"))
        .collect();
    let bytes = seven_zip_bytes(&entries, true);
    let reads = Arc::new(Reads::default());
    let root = tempfile::tempdir().expect("a folder");
    let uri = "file:///fixture/cache-test.7z";
    let browser = ArchiveBrowser::new(counting_opener(bytes, &reads), root.path().to_path_buf());

    browser.list(uri, "", &Cancellation::new()).expect("lists");
    let first = reads.bytes.swap(0, Ordering::SeqCst);
    browser
        .list(uri, "Folder 3", &Cancellation::new())
        .expect("lists");
    let again = reads.bytes.load(Ordering::SeqCst);
    // The header of 2,000 names is packed into about 2 KB.
    assert!(first > 1500, "the header was read: {first}");
    // The first bytes, which tell a 7z from a ZIP or TAR, and the start
    // header.
    assert!(again <= 262 + 32, "only the start header was read again: {again}");

    let changed = seven_zip_bytes(&[SevenEntry::file("new.txt", b"new")], true);
    let browser = ArchiveBrowser::new(counting_opener(changed, &reads), root.path().to_path_buf());
    let names: Vec<String> = browser
        .list(uri, "", &Cancellation::new())
        .expect("lists")
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["new.txt"]);
}

/// A solid 7z of 10,000 files lists at once and extracts in one pass:
/// its block is decoded once, not once per file.
///
/// parity: ARC-003, ARC-024
#[test]
fn a_solid_7z_of_10000_files_lists_quickly_and_extracts_in_one_pass() {
    let entries: Vec<SevenEntry> = (0..10_000)
        .map(|index| {
            SevenEntry::file(
                &format!("Batch {}/item {index}.txt", index % 100),
                format!("item {index}").as_bytes(),
            )
        })
        .collect();
    let bytes = seven_zip_bytes(&entries, true);
    let reads = Arc::new(Reads::default());
    let root = tempfile::tempdir().expect("a folder");

    let started = Instant::now();
    let browser = ArchiveBrowser::new(counting_opener(bytes.clone(), &reads), root.path().to_path_buf());
    let listing = browser
        .list("file:///fixture/large.7z", "", &Cancellation::new())
        .expect("lists");
    assert_eq!(listing.entries.len(), 100);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );

    let fixture = fixture_with(&bytes);
    reads.opened.store(0, Ordering::SeqCst);
    let mut extractor = ZipExtractor::new(
        counting_opener(bytes, &reads),
        gio_factory(),
        Arc::new(LocalFileOutput::owner_only()),
    );
    let extracted = fixture
        .extract_with(&mut extractor, "Unpacked")
        .expect("extracts");
    assert_eq!(extracted.summary.file_count, 10_000);
    let item = fixture.destination.join("Unpacked/Batch 7/item 9907.txt");
    assert_eq!(fs::read(item).expect("a file"), b"item 9907");
    assert!(
        reads.opened.load(Ordering::SeqCst) <= 3,
        "opened {:?} times",
        reads.opened
    );
}

/// BCJ2, which 7-Zip's Ultra level uses for programs, splits a file into
/// four streams; its block is decoded like any other.
///
/// parity: ARC-024
#[test]
fn a_bcj2_block_as_7zip_ultra_writes_for_programs_extracts() {
    let data = b"program text without branch opcodes";
    // Main data, calls, jumps and the range coder's five starting bytes.
    let streams: [&[u8]; 4] = [data, b"", b"", &[0; 5]];
    let coders = [TestCoder {
        method: &BCJ2,
        inputs: 4,
        properties: &[],
    }];
    let bytes = coded_seven_zip(
        "tool.bin",
        &coders,
        &[],
        &[0, 1, 2, 3],
        &streams,
        &[data.len() as u64],
    );
    let fixture = fixture_with(&bytes);

    fixture.extract("Unpacked").expect("extracts");

    let tool = fixture.destination.join("Unpacked/tool.bin");
    assert_eq!(fs::read(tool).expect("a file"), data);
}

/// A forged BCJ2 block whose second coder reads its own output never
/// decodes: the archive is refused at once instead of following the loop
/// until the stack runs out.
///
/// parity: ARC-017
#[test]
fn coders_linked_in_a_loop_are_refused_at_once() {
    let coder = |method, inputs| TestCoder {
        method,
        inputs,
        properties: &[],
    };
    let coders = [coder(&BCJ2[..], 4), coder(&COPY[..], 1), coder(&COPY[..], 1)];
    // BCJ2's first input is coder 1's output, and so is coder 1's input.
    let bind_pairs = [(0, 1), (4, 1)];
    let streams: [&[u8]; 4] = [b"", b"", &[0; 5], b"data"];
    let bytes = coded_seven_zip(
        "loop.bin",
        &coders,
        &bind_pairs,
        &[1, 2, 3, 5],
        &streams,
        &[4, 4, 4],
    );
    let fixture = fixture_with(&bytes);
    let started = Instant::now();

    let error = fixture.extract("Unpacked").expect_err("refused");

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(
            error,
            ArchiveError::SevenZipMethod | ArchiveError::DamagedSevenZip
        ),
        "{error:?}"
    );
    fixture.assert_no_output();
}

/// 7-Zip writes `BZip2` and Deflate 7z archives when asked (`-m0=BZip2`,
/// `-m0=Deflate`). A `BZip2` one opens like any other 7z; a Deflate one is
/// listed and refused plainly, as the reader does not decode Deflate.
///
/// parity: ARC-024
#[test]
fn a_bzip2_7z_extracts_and_a_deflate_one_is_refused_plainly() {
    use std::io::Write;

    let data = "Fictional field notes, written twice. ".repeat(200);
    let mut bzip2 = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    bzip2.write_all(data.as_bytes()).expect("compressed");
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    deflate.write_all(data.as_bytes()).expect("compressed");
    let coded = |method: &[u8], packed: &[u8]| {
        let coders = [TestCoder {
            method,
            inputs: 1,
            properties: &[],
        }];
        coded_seven_zip("notes.txt", &coders, &[], &[0], &[packed], &[data.len() as u64])
    };

    let fixture = fixture_with(&coded(&BZIP2, &bzip2.finish().expect("compressed")));
    fixture.extract("Unpacked").expect("a BZip2 7z extracts");
    let notes = fixture.destination.join("Unpacked/notes.txt");
    assert_eq!(fs::read(notes).expect("a file"), data.as_bytes());

    let bytes = coded(&DEFLATE, &deflate.finish().expect("compressed"));
    let root = tempfile::tempdir().expect("a folder");
    let listing = browser_of(bytes.clone(), root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("lists");
    assert_eq!(listing.entries.len(), 1);
    let fixture = fixture_with(&bytes);
    let error = fixture.extract("Unpacked").expect_err("refused");
    assert!(matches!(error, ArchiveError::SevenZipMethod), "{error:?}");
    fixture.assert_no_output();
}

/// Opening or copying a late file of a solid block decodes every file
/// before it. A huge, highly compressed stream placed before a small
/// file counts against the decompression limits too, so it is refused
/// at once instead of being decoded and dropped.
///
/// parity: ARC-006, ARC-017
#[test]
fn a_late_file_behind_a_huge_compressed_stream_is_refused_without_decoding_it() {
    let filler = vec![0u8; 64 * 1024 * 1024];
    let entries = [
        SevenEntry::file("filler.bin", &filler),
        SevenEntry::file("notes.txt", b"small notes"),
    ];
    let bytes = seven_zip_bytes(&entries, true);
    assert!(bytes.len() < 1024 * 1024, "the filler packs tightly");
    let root = tempfile::tempdir().expect("a folder");
    let browser = browser_of(bytes.clone(), root.path());

    let started = Instant::now();
    let error = browser
        .preview_member("file:///fixture/Bundle.7z", "notes.txt", &Cancellation::new())
        .expect_err("refused");
    assert!(matches!(error, ArchiveError::MemberTooLarge), "{error:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );

    let fixture = fixture_with(&bytes);
    let mut extractor = fixture.extractor().with_selection(&["notes.txt".to_owned()]);
    let error = fixture.extract_with(&mut extractor, "Copy").expect_err("refused");
    assert!(matches!(error, ArchiveError::MemberTooLarge), "{error:?}");
    fixture.assert_no_output();
}

/// The data is decoded from a second opening of the archive. If the
/// archive was replaced in between, even by one of the same shape, it is
/// refused rather than read under the first one's names.
///
/// parity: ARC-017
#[test]
fn an_archive_replaced_between_reading_and_decoding_is_refused() {
    let first = seven_zip_bytes(&[SevenEntry::file("notes.txt", b"first text")], true);
    let second = seven_zip_bytes(&[SevenEntry::file("notes.txt", b"other text")], true);
    assert_eq!(first.len(), second.len(), "the same shape and size");
    let opens = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&opens);
    let opener = move |_uri: &str, _cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        let bytes = if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            first.clone()
        } else {
            second.clone()
        };
        Ok(Box::new(std::io::Cursor::new(bytes)))
    };
    let fixture = fixture_with(b"");
    let mut extractor = ZipExtractor::new(
        Arc::new(opener),
        gio_factory(),
        Arc::new(LocalFileOutput::owner_only()),
    );

    let error = fixture
        .extract_with(&mut extractor, "Unpacked")
        .expect_err("refused");

    assert!(matches!(error, ArchiveError::SevenZipChanged), "{error:?}");
    fixture.assert_no_output();
}

/// The substreams record is optional: without it each block holds one
/// file of the block's size, as 7-Zip reads it.
///
/// parity: ARC-022, ARC-024
#[test]
fn a_7z_without_a_substreams_record_lists_and_extracts() {
    let bytes = seven_zip_without_sub_streams("notes.txt", b"stored notes");
    let root = tempfile::tempdir().expect("a folder");
    let listing = browser_of(bytes.clone(), root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("lists");
    assert!(
        matches!(listing.entries[0].kind, ArchiveEntryKind::File { size: 12, .. }),
        "{:?}",
        listing.entries
    );
    let fixture = fixture_with(&bytes);

    fixture.extract("Unpacked").expect("extracts");

    let notes = fixture.destination.join("Unpacked/notes.txt");
    assert_eq!(fs::read(notes).expect("a file"), b"stored notes");
}

/// An empty file listed between two files of a solid block has no data
/// in it: it opens and copies even when the block's method is one the
/// reader lacks.
///
/// parity: ARC-016
#[test]
fn an_empty_file_inside_an_unreadable_solid_block_still_opens() {
    let bytes = seven_zip_with_empty_file_between(&PPMD, &[6, 0, 0, 0, 1], &[0x42; 16], &[0x43; 16]);
    let root = tempfile::tempdir().expect("a folder");
    let listing = browser_of(bytes.clone(), root.path())
        .list("file:///fixture/Bundle.7z", "", &Cancellation::new())
        .expect("lists");
    let can_open: Vec<(String, bool)> = listing
        .entries
        .into_iter()
        .map(|entry| (entry.name, entry.can_open))
        .collect();
    assert_eq!(
        can_open,
        [
            ("Empty.txt".to_owned(), true),
            ("First.txt".to_owned(), false),
            ("Second.txt".to_owned(), false),
        ],
    );

    let fixture = fixture_with(&bytes);
    let mut extractor = fixture.extractor().with_selection(&["Empty.txt".to_owned()]);
    fixture.extract_with(&mut extractor, "Copy").expect("copied");
    let copy = fixture.destination.join("Copy/Empty.txt");
    assert_eq!(fs::read(copy).expect("a file"), b"");
}

/// A stream that loses its connection once the decoder reads data, as a
/// GIO stream reports it (an error of no particular kind).
struct LostConnection(std::io::Cursor<Vec<u8>>);

impl Read for LostConnection {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.0.position() >= 32 {
            return Err(std::io::Error::other("Connection lost"));
        }
        self.0.read(buffer)
    }
}

impl Seek for LostConnection {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(position)
    }
}

/// A connection lost while a file is decoded is that failure, so that
/// it can be retried, and not a damaged archive.
///
/// parity: ARC-017
#[test]
fn a_connection_lost_while_decoding_is_not_called_damage() {
    let bytes = seven_zip_bytes(&[SevenEntry::file("notes.txt", b"remote notes")], true);
    let opener = move |_uri: &str, _cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        let cursor = std::io::Cursor::new(bytes.clone());
        if std::thread::current().name() == Some("ox-7z-decoder") {
            Ok(Box::new(LostConnection(cursor)))
        } else {
            Ok(Box::new(cursor))
        }
    };
    let root = tempfile::tempdir().expect("a folder");
    let browser = ArchiveBrowser::new(Arc::new(opener), root.path().to_path_buf());

    let error = browser
        .preview_member("file:///fixture/Remote.7z", "notes.txt", &Cancellation::new())
        .expect_err("the connection is lost");

    assert!(matches!(error, ArchiveError::Backend(_)), "{error:?}");
    assert!(error.to_string().contains("Connection lost"), "{error}");
}
