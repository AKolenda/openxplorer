// SPDX-License-Identifier: AGPL-3.0-only
//! Password-protected ZIPs: a member encrypted with the traditional PKWARE
//! method ("ZipCrypto") is listed like any other, and opening or
//! extracting it asks for the password, as Windows 11 File Explorer does.
//! A missing password and a wrong one are told apart so the window can
//! ask again, and nothing is written until the password is right. AES
//! encryption is listed but left to an archive manager.

mod archive_support;

use std::fs;

use ox_core::archive::{ArchiveBrowser, ArchiveError, ArchivePassword, ExtractionRequest};

use archive_support::{file_uri, opener, Compression, ExtractionFixture, TestMember, ENCRYPTED_FLAG};

/// The password of the test archives.
const PASSWORD: &str = "lighthouse";

/// `Harbor/`, `Harbor/tides.txt` (deflated) and `Harbor/buoys.txt`
/// (stored), made with Info-ZIP's `zip -P lighthouse -r`: both files are
/// encrypted and carry data descriptors, so their check byte is the high
/// byte of the modification time.
const INFO_ZIP_ARCHIVE: &[&str] = &[
    "504b03040a000000000000603c5d00000000000000000000000007000000486172626f722f504b030414000900080000",
    "603c5daf04387b3a0000002f00000010000000486172626f722f74696465732e747874d8daa2a75707a61c3ae8c85451",
    "16b5a380e86c35a76e80ef5aee77d70513decb90524e7f0e62e6bbdbfe07d7e545e921841e74009873302e2a24504b07",
    "08af04387b3a0000002f000000504b03040a000900000000603c5d88babb53160000000a00000010000000486172626f",
    "722f62756f79732e74787431b01f189b41b3f495b677a07a9758107dc9280fc721504b070888babb53160000000a0000",
    "00504b01021e030a000000000000603c5d000000000000000000000000070000000000000000001000ed410000000048",
    "6172626f722f504b01021e0314000900080000603c5daf04387b3a0000002f000000100000000000000001000000a481",
    "25000000486172626f722f74696465732e747874504b01021e030a000900000000603c5d88babb53160000000a000000",
    "100000000000000001000000a4819d000000486172626f722f62756f79732e747874504b05060000000003000300b100",
    "0000f10000000000",
];

/// The bytes of [`INFO_ZIP_ARCHIVE`].
fn info_zip_archive() -> Vec<u8> {
    let text: String = INFO_ZIP_ARCHIVE.concat();
    (0..text.len())
        .step_by(2)
        .map(|start| u8::from_str_radix(&text[start..start + 2], 16).expect("hex digits"))
        .collect()
}

/// An archive of a folder and two encrypted files, one deflated and one
/// stored.
fn encrypted_members() -> Vec<TestMember> {
    vec![
        TestMember::folder("Harbor/"),
        TestMember::file("Harbor/tides.txt", b"High water at noon.").encrypted_with(PASSWORD),
        TestMember::file("Harbor/buoys.txt", b"Red, green, red.")
            .compressed_with(Compression::Stored)
            .encrypted_with(PASSWORD),
    ]
}

/// A browser over the fixture's archive, with `password` if any.
fn browser(fixture: &ExtractionFixture, password: Option<&str>) -> ArchiveBrowser {
    let browser = ArchiveBrowser::new(opener(), fixture.root.join("previews"));
    match password {
        Some(password) => browser.with_password(ArchivePassword::from(password)),
        None => browser,
    }
}

/// The request to extract the fixture's archive as `Harbor copy`.
fn request(fixture: &ExtractionFixture) -> ExtractionRequest {
    ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: fixture.destination_uri(),
        folder_name: "Harbor copy".to_owned(),
    }
}

/// Browsing needs no password: the names, sizes and folders are in the
/// clear, and the files can be opened (which then asks).
///
/// parity: ARC-006
#[test]
fn a_password_protected_zip_lists_its_files_without_a_password() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&encrypted_members());

    let listing = browser(&fixture, None)
        .list(&fixture.archive_uri(), "Harbor/", &fixture.cancel)
        .expect("the archive lists");

    let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["buoys.txt", "tides.txt"]);
    assert!(listing
        .entries
        .iter()
        .all(|entry| entry.is_encrypted && entry.can_open));
}

/// Opening a file asks for the password; a wrong one is told apart from
/// a missing one, and the right one opens the private copy. Nothing is
/// left behind by a refused try.
///
/// parity: ARC-006
#[test]
fn opening_an_encrypted_file_needs_its_password() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&encrypted_members());
    let uri = fixture.archive_uri();

    for (password, refusal) in [
        (None, ArchiveError::PasswordRequired),
        (Some("lighthouses"), ArchiveError::WrongPassword),
        (Some(""), ArchiveError::WrongPassword),
    ] {
        let opened = browser(&fixture, password).preview_member(&uri, "Harbor/tides.txt", &fixture.cancel);
        assert_eq!(opened.unwrap_err(), refusal, "{password:?}");
    }
    let previews = fixture.root.join("previews");
    let left = fs::read_dir(&previews).map_or(0, Iterator::count);
    assert_eq!(left, 0, "a refused try leaves no copy");

    for (member, text) in [
        ("Harbor/tides.txt", "High water at noon."),
        ("Harbor/buoys.txt", "Red, green, red."),
    ] {
        let copy = browser(&fixture, Some(PASSWORD))
            .preview_member(&uri, member, &fixture.cancel)
            .expect("the right password opens the file");
        assert_eq!(fs::read_to_string(&copy.path).expect("read the copy"), text);
    }
}

/// The Extract dialog's check counts a password-protected ZIP like any
/// other: the password is asked when the extraction starts.
///
/// parity: ARC-016
#[test]
fn the_check_of_a_password_protected_zip_needs_no_password() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&encrypted_members());

    let summary = fixture
        .extractor()
        .inspect(&fixture.archive_uri(), &fixture.cancel)
        .expect("the check passes");

    assert_eq!((summary.file_count, summary.folder_count), (2, 1));
    fixture.assert_no_output();
}

/// Extracting without the password, or with a wrong one, says which and
/// writes nothing; the right password extracts every file.
///
/// parity: ARC-016
#[test]
fn extracting_needs_the_password_and_writes_nothing_until_it_is_right() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&encrypted_members());

    let missing = fixture.extractor().extract(&request(&fixture), &fixture.cancel);
    assert_eq!(missing.unwrap_err(), ArchiveError::PasswordRequired);
    fixture.assert_no_output();
    let wrong = fixture
        .extractor()
        .with_password(ArchivePassword::from("lighthouses"))
        .extract(&request(&fixture), &fixture.cancel);
    assert_eq!(wrong.unwrap_err(), ArchiveError::WrongPassword);
    fixture.assert_no_output();

    fixture
        .extractor()
        .with_password(ArchivePassword::from(PASSWORD))
        .extract(&request(&fixture), &fixture.cancel)
        .expect("the right password extracts");

    let folder = fixture.destination.join("Harbor copy").join("Harbor");
    assert_eq!(
        fs::read_to_string(folder.join("tides.txt")).unwrap(),
        "High water at noon."
    );
    assert_eq!(
        fs::read_to_string(folder.join("buoys.txt")).unwrap(),
        "Red, green, red."
    );
}

/// Copying items out of the ZIP extracts only them, and needs the
/// password too.
///
/// parity: ARC-026
#[test]
fn copying_an_encrypted_file_out_needs_the_password() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&encrypted_members());
    let selection = ["Harbor/buoys.txt".to_owned()];

    let missing = fixture
        .extractor()
        .with_selection(&selection)
        .extract(&request(&fixture), &fixture.cancel);
    assert_eq!(missing.unwrap_err(), ArchiveError::PasswordRequired);

    fixture
        .extractor()
        .with_selection(&selection)
        .with_password(ArchivePassword::from(PASSWORD))
        .extract(&request(&fixture), &fixture.cancel)
        .expect("the copy is made");
    let copy = fixture.destination.join("Harbor copy/Harbor/buoys.txt");
    assert_eq!(fs::read_to_string(copy).unwrap(), "Red, green, red.");
}

/// An archive that mixes plain and encrypted files asks only when an
/// encrypted one is read: a plain file opens without a password.
///
/// parity: ARC-006
#[test]
fn a_plain_file_beside_encrypted_ones_opens_without_a_password() {
    let fixture = ExtractionFixture::new();
    let mut members = encrypted_members();
    members.push(TestMember::file("readme.txt", b"Welcome aboard."));
    fixture.write_zip(&members);

    let copy = browser(&fixture, None)
        .preview_member(&fixture.archive_uri(), "readme.txt", &fixture.cancel)
        .expect("a plain file needs no password");

    assert_eq!(fs::read_to_string(copy.path).unwrap(), "Welcome aboard.");
}

/// An archive made by Info-ZIP's `zip -P`, with data descriptors, opens
/// and extracts with its password.
///
/// parity: ARC-016
#[test]
fn an_info_zip_archive_extracts_with_its_password() {
    let fixture = ExtractionFixture::new();
    fs::write(&fixture.archive, info_zip_archive()).expect("write the archive");

    let wrong = fixture
        .extractor()
        .with_password(ArchivePassword::from("Lighthouse"))
        .extract(&request(&fixture), &fixture.cancel);
    assert_eq!(wrong.unwrap_err(), ArchiveError::WrongPassword);
    fixture
        .extractor()
        .with_password(ArchivePassword::from(PASSWORD))
        .extract(&request(&fixture), &fixture.cancel)
        .expect("the right password extracts");

    let folder = fixture.destination.join("Harbor copy").join("Harbor");
    assert_eq!(
        fs::read_to_string(folder.join("tides.txt")).unwrap(),
        "Tide table for the fictional Tidewater harbor.\n"
    );
    assert_eq!(
        fs::read_to_string(folder.join("buoys.txt")).unwrap(),
        "Buoy list\n"
    );
}

/// AES encryption (method 99 of the WinZip specification) is listed, but
/// opening or extracting it says it needs an archive manager instead of
/// asking for a password the app cannot use.
///
/// parity: ARC-016
#[test]
fn aes_encrypted_files_are_listed_and_left_to_an_archive_manager() {
    let fixture = ExtractionFixture::new();
    fixture.write_zip(&[TestMember::file("vault.txt", b"x")
        .compressed_with(Compression::Unsupported(99))
        .with_flags(ENCRYPTED_FLAG)]);
    let uri = file_uri(&fixture.archive);

    let listing = browser(&fixture, None)
        .list(&uri, "", &fixture.cancel)
        .expect("lists");
    assert!(listing
        .entries
        .iter()
        .all(|entry| entry.is_encrypted && !entry.can_open));
    let opened = browser(&fixture, Some(PASSWORD)).preview_member(&uri, "vault.txt", &fixture.cancel);
    assert_eq!(opened.unwrap_err(), ArchiveError::UnsupportedEncryption);
    let checked = fixture.extractor().inspect(&uri, &fixture.cancel);
    assert_eq!(checked.unwrap_err(), ArchiveError::UnsupportedEncryption);
    assert!(ArchiveError::UnsupportedEncryption
        .to_string()
        .contains("archive manager"));
}
