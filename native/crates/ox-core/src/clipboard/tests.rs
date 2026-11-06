// SPDX-License-Identifier: AGPL-3.0-only
//! Clipboard protocol regressions from
//! `v2.0.0:desktop/tests/test_file_clipboard_interop.py` and the `ClipboardTests`
//! in `v2.0.0:desktop/tests/test_v05.py`.

use std::fmt::Write as _;

use super::*;
use crate::location::DeviceLabel;

/// The first file of the fixture selections; its name needs percent-encoding.
const README_URI: &str = "file:///home/demo/Read%20me.txt";
/// The second file of the fixture selections.
const PLANNING_URI: &str = "file:///home/demo/Planning.pdf";

/// The session facts of the fixtures: no device is mounted, so every device
/// label is "Connected device".
fn context() -> LocationContext {
    LocationContext::default()
}

/// A valid two-item selection with a fresh token.
fn selection(mode: ClipboardMode) -> ClipboardFiles {
    ClipboardFiles::new(mode, &[README_URI.into(), PLANNING_URI.into()], &context()).expect("valid selection")
}

/// The bytes `files` publishes as `mime_type`, for a session like the
/// fixtures'.
fn published(files: &ClipboardFiles, mime_type: &str) -> Vec<u8> {
    published_with(files, mime_type, &context())
}

/// The bytes `files` publishes as `mime_type` in `locations`' session.
fn published_with(files: &ClipboardFiles, mime_type: &str, locations: &LocationContext) -> Vec<u8> {
    files
        .encode(locations)
        .into_iter()
        .find(|payload| payload.mime_type == mime_type)
        .unwrap_or_else(|| panic!("{mime_type} is published"))
        .bytes
}

/// A [`URI_LIST`] read with `kde_cut_marker` from the same clipboard owner.
fn uri_list(kde_cut_marker: Option<&[u8]>) -> FileListFormat<'_> {
    FileListFormat::UriList { kde_cut_marker }
}

/// Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_gnome_cut_can_consume_successful_items_across_reads`
///
/// parity: CLIP-008
#[test]
fn gnome_cut_keeps_identity_across_reads_and_consumes_only_successful_items() {
    let payload = format!("cut\n{README_URI}\n{PLANNING_URI}");
    let mut files = decode(FileListFormat::Gnome, payload.as_bytes()).expect("GNOME cut");
    let again = decode(FileListFormat::Gnome, payload.as_bytes()).expect("same owner");
    assert_eq!(files.token(), again.token());
    assert!(files.consume(again.token(), &[README_URI.into()]));
    assert_eq!(files.uris(), &[PLANNING_URI]);
    assert!(files.consume(again.token(), &[PLANNING_URI.into()]));
    assert!(files.encode(&context()).is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_changed_external_payload_is_not_consumed_by_old_operation`
/// and `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_changed_mode_is_not_consumed_by_old_cut`
///
/// parity: CLIP-008
#[test]
fn old_cut_cannot_consume_a_changed_payload_or_a_copy() {
    let first = decode(FileListFormat::Gnome, format!("cut\n{README_URI}").as_bytes()).expect("cut");
    for payload in [format!("cut\n{PLANNING_URI}"), format!("copy\n{README_URI}")] {
        let mut changed = decode(FileListFormat::Gnome, payload.as_bytes()).expect("changed selection");
        assert!(!changed.consume(first.token(), &[README_URI.into(), PLANNING_URI.into()]));
        assert_eq!(changed.uris().len(), 1);
    }
}

/// A KDE cut marker read beside a URI list, and the mode it must give.
struct MarkerCase {
    /// What the marker is; printed if it gives the wrong mode.
    reason: &'static str,
    /// The [`KDE_CUT`] payload, or `None` when the owner offers none.
    marker: Option<&'static [u8]>,
    /// The mode the URI list must decode with.
    mode: ClipboardMode,
}

/// Only an exact `1`, optionally NUL-padded, makes a URI list a cut.
const MARKER_CASES: [MarkerCase; 9] = [
    MarkerCase {
        reason: "no marker",
        marker: None,
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "KIO's copy marker",
        marker: Some(b"0"),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "an empty marker",
        marker: Some(b""),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "a word instead of the marker",
        marker: Some(b"cut"),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "a trailing newline",
        marker: Some(b"1\n"),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "twenty 1s",
        marker: Some(&[b'1'; 20]),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "NUL bytes without the 1",
        marker: Some(&[0; 17]),
        mode: ClipboardMode::Copy,
    },
    MarkerCase {
        reason: "KIO's cut marker",
        marker: Some(b"1"),
        mode: ClipboardMode::Cut,
    },
    MarkerCase {
        reason: "a NUL-terminated cut marker",
        marker: Some(b"1\0"),
        mode: ClipboardMode::Cut,
    },
];

/// Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_uri_list_without_exact_kde_cut_marker_remains_copy`,
/// `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_nul_terminated_kde_cut_marker`
/// and `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_gnome_copy_ignores_unrelated_kde_cut_marker`
///
/// parity: CLIP-006
#[test]
fn kde_requires_an_exact_short_cut_marker() {
    let payload = format!("{README_URI}\r\n{PLANNING_URI}\r\n");
    for case in MARKER_CASES {
        let files = decode(uri_list(case.marker), payload.as_bytes()).expect("URI list");
        assert_eq!(files.mode(), case.mode, "{}", case.reason);
    }
    // A GNOME payload names its own mode: `FileListFormat::Gnome` has no
    // place for a KDE marker, so an unrelated one cannot make it a cut.
    let gnome_copy = format!("copy\n{README_URI}");
    let gnome = decode(FileListFormat::Gnome, gnome_copy.as_bytes()).expect("GNOME copy");
    assert_eq!(gnome.mode(), ClipboardMode::Copy);
}

/// Safety rule (exact cut marker): NUL padding counts toward the 16-byte
/// limit, so a marker that starts with `1` but is longer is a copy.
///
/// parity: CLIP-006
#[test]
fn cut_marker_padding_ends_at_sixteen_bytes() {
    let payload = format!("{README_URI}\r\n");
    let mut marker = vec![0; 16];
    marker[0] = b'1';
    let longest = decode(uri_list(Some(&marker)), payload.as_bytes()).expect("URI list");
    assert_eq!(longest.mode(), ClipboardMode::Cut);
    marker.push(0);
    let too_long = decode(uri_list(Some(&marker)), payload.as_bytes()).expect("URI list");
    assert_eq!(too_long.mode(), ClipboardMode::Copy);
}

/// Regression: the marker was published as `x-kde-cutselection`, a name KDE
/// never reads, so a cut pasted in Dolphin became a copy. The literal is
/// the one in KIO's `setClipboardDataCut` and `isClipboardDataCut`
/// (`kio/src/widgets/paste.cpp`).
///
/// parity: CLIP-004, CLIP-006
#[test]
fn kde_cut_marker_uses_the_mime_type_kio_reads() {
    let kio_mime_type = "application/x-kde-cutselection";
    assert_eq!(KDE_CUT, kio_mime_type);
    assert_eq!(published(&selection(ClipboardMode::Cut), kio_mime_type), b"1");
    assert_eq!(published(&selection(ClipboardMode::Copy), kio_mime_type), b"0");
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::ClipboardTests::test_deduplication`
///
/// parity: CLIP-005
#[test]
fn uri_list_discards_comments_and_deduplicates_canonical_addresses() {
    let payload =
        b"# copied files\r\n\r\nfile://localhost/tmp/a\r\nfile:///tmp/a\r\nsmb://STUDIO-NAS/Shared/a\r\n";
    let files = decode(uri_list(None), payload).expect("canonical list");
    assert_eq!(files.uris(), &["file:///tmp/a", "smb://studio-nas/Shared/a"]);
}

/// Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_kde_marker_cannot_turn_plain_text_into_files`,
/// `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_kde_marker_cannot_turn_invalid_uri_list_into_files`
/// and the `test_plain_text_is_not_file_clipboard`, `test_unsafe_scheme`,
/// `test_no_password_in_clipboard_uri`, `test_share_itself_not_transferable`,
/// `test_malformed_json` and `test_large_rejected` cases of
/// `v2.0.0:desktop/tests/test_v05.py::ClipboardTests`
///
/// parity: CLIP-005, SAFE-010
#[test]
fn invalid_external_payloads_fail_closed() {
    // Plain text is not a file-list format, so it is never decoded at all,
    // not even the text this app publishes beside a file list.
    assert_eq!(FileListFormat::from_mime_type("text/plain"), None);
    assert_eq!(FileListFormat::from_mime_type(TEXT), None);
    let cases = refused_uri_lists().into_iter().chain(refused_other_formats());
    for case in cases {
        let decoded = decode(case.format, &case.bytes);
        assert!(
            decoded.is_none(),
            "{} decoded despite {}",
            case.format.mime_type(),
            case.reason
        );
    }
}

/// A payload that must not decode as a file list.
struct RefusedPayload {
    /// Why it is refused; printed if it decodes after all.
    reason: &'static str,
    /// The format the payload claims to be, with the KDE cut marker read
    /// beside a URI list.
    format: FileListFormat<'static>,
    /// The payload itself.
    bytes: Vec<u8>,
}

impl RefusedPayload {
    /// A [`URI_LIST`] payload, read with `kde_cut_marker`.
    fn uri_list(reason: &'static str, kde_cut_marker: Option<&'static [u8]>, bytes: Vec<u8>) -> Self {
        Self {
            reason,
            format: uri_list(kde_cut_marker),
            bytes,
        }
    }
}

/// A KDE cut marker, which must not turn an invalid payload into a cut.
const CUT_MARKER: Option<&[u8]> = Some(b"1");

/// A payload one byte over [`MAX_BYTES`].
fn oversized_payload() -> Vec<u8> {
    vec![b'x'; MAX_BYTES + 1]
}

/// URI lists that are no file list, cut marker or not.
fn refused_uri_lists() -> Vec<RefusedPayload> {
    let too_many_items = vec![README_URI; MAX_ITEMS + 1].join("\n").into_bytes();
    vec![
        RefusedPayload::uri_list("an empty payload", CUT_MARKER, Vec::new()),
        RefusedPayload::uri_list("bytes that are not UTF-8", CUT_MARKER, b"\xff".to_vec()),
        RefusedPayload::uri_list("a web address", CUT_MARKER, b"https://example.test/file".to_vec()),
        RefusedPayload::uri_list("a share root", CUT_MARKER, b"smb://studio-nas/Shared".to_vec()),
        RefusedPayload::uri_list("credentials", CUT_MARKER, b"smb://user:pass@nas/share/a".to_vec()),
        RefusedPayload::uri_list("an encoded line break", CUT_MARKER, b"file:///tmp/a%0Ab".to_vec()),
        RefusedPayload::uri_list("its size", CUT_MARKER, oversized_payload()),
        RefusedPayload::uri_list("more than 100,000 items", None, too_many_items),
    ]
}

/// Payloads of the other formats that are no file list.
fn refused_other_formats() -> Vec<RefusedPayload> {
    vec![
        RefusedPayload {
            reason: "the unknown operation `move`",
            format: FileListFormat::Gnome,
            bytes: format!("move\n{README_URI}").into_bytes(),
        },
        RefusedPayload {
            reason: "a share root",
            format: FileListFormat::Gnome,
            bytes: b"cut\nsmb://nas/share".to_vec(),
        },
        RefusedPayload {
            reason: "malformed JSON",
            format: FileListFormat::Custom,
            bytes: b"{".to_vec(),
        },
        RefusedPayload {
            reason: "its size",
            format: FileListFormat::Custom,
            bytes: oversized_payload(),
        },
    ]
}

/// Paste can name each file-list format by the MIME type a clipboard read
/// chose; the KDE marker alone is not a file list.
///
/// parity: CLIP-005
#[test]
fn file_list_formats_map_to_and_from_their_mime_types() {
    let formats = [FileListFormat::Custom, FileListFormat::Gnome, uri_list(None)];
    for format in formats {
        assert_eq!(FileListFormat::from_mime_type(format.mime_type()), Some(format));
    }
    assert_eq!(FileListFormat::from_mime_type(KDE_CUT), None);
}

/// Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_custom_payload_keeps_priority_and_its_token`
///
/// parity: CLIP-005
#[test]
fn custom_payload_preserves_token_and_rejects_invalid_operations() {
    let custom = format!(r#"{{"mode":"move","uris":["{README_URI}"],"token":"previous-owner"}}"#);
    let files = decode(FileListFormat::Custom, custom.as_bytes()).expect("custom payload");
    assert_eq!(files.token(), "previous-owner");
    assert_eq!(files.mode(), ClipboardMode::Cut);
    let delete = br#"{"mode":"delete","uris":["file:///tmp/a"]}"#;
    assert!(decode(FileListFormat::Custom, delete).is_none());
    let no_items = br#"{"mode":"copy","uris":[]}"#;
    assert!(decode(FileListFormat::Custom, no_items).is_none());
    let without_token = br#"{"mode":"copy","uris":["file:///tmp/a"]}"#;
    let no_token = decode(FileListFormat::Custom, without_token).expect("missing token");
    assert_eq!(no_token.token().len(), 32);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::ClipboardTests::test_custom_copy_roundtrip`,
/// `v2.0.0:desktop/tests/test_v05.py::ClipboardTests::test_custom_cut_roundtrip`
/// and `v2.0.0:desktop/tests/test_v05.py::ClipboardTests::test_gnome_cut_flag`
///
/// parity: CLIP-004
#[test]
fn every_advertised_format_round_trips() {
    for mode in [ClipboardMode::Copy, ClipboardMode::Cut] {
        let files = selection(mode);
        let marker = published(&files, KDE_CUT);
        let file_lists = [
            FileListFormat::Custom,
            FileListFormat::Gnome,
            uri_list(Some(&marker)),
        ];
        let advertised: Vec<&str> = files
            .encode(&context())
            .iter()
            .map(|payload| payload.mime_type)
            .collect();
        assert_eq!(advertised, [CUSTOM, GNOME, URI_LIST, TEXT, KDE_CUT]);
        for format in file_lists {
            let payload = published(&files, format.mime_type());
            let result = decode(format, &payload).expect("published format decodes");
            assert_eq!(result.mode(), mode);
            assert_eq!(result.uris(), files.uris());
            if format == FileListFormat::Custom {
                assert_eq!(result.token(), files.token());
            }
        }
    }
}

/// parity: CLIP-008
#[test]
fn external_fingerprint_matches_the_python_clipboard() {
    // Use the actual shipped decoder so token changes cannot silently break
    // cut consumption between Python and native windows.
    let files = decode(FileListFormat::Gnome, b"cut\nfile:///tmp/a").expect("cut");
    let script = concat!(
        "from file_clipboard import decode_clipboard, GNOME\n",
        "print(decode_clipboard(GNOME, b'cut\\nfile:///tmp/a')['token'])",
    );
    // The Python app's modules (v2.0.0:desktop/), which native/tools/check.py
    // extracts and names in OX_PYTHON_APP.
    let desktop = std::env::var_os("OX_PYTHON_APP").expect(
        "OX_PYTHON_APP names the Python app's modules (v2.0.0:desktop/); run the tests through \
         native/tools/check.py, which extracts them",
    );
    let output = std::process::Command::new("python3")
        .current_dir(desktop)
        .args(["-c", script])
        .output()
        .expect("Python oracle");
    assert!(output.status.success());
    assert_eq!(
        files.token(),
        String::from_utf8(output.stdout).expect("hex token").trim()
    );
}

/// parity: CLIP-005
#[test]
fn legacy_line_endings_do_not_drop_valid_selections() {
    for separator in ["\n", "\r\n", "\r", "\u{85}", "\u{2028}"] {
        let payload = format!("cut{separator}{README_URI}{separator}{PLANNING_URI}{separator}");
        let files = decode(FileListFormat::Gnome, payload.as_bytes()).expect("legacy line endings");
        assert_eq!(files.mode(), ClipboardMode::Cut);
        assert_eq!(files.uris(), &[README_URI, PLANNING_URI]);
    }
}

/// parity: CLIP-004
#[test]
fn published_file_lists_match_the_python_encoding() {
    let files = selection(ClipboardMode::Cut);
    let expected_gnome = format!("cut\n{README_URI}\n{PLANNING_URI}");
    let expected_uri_list = format!("{README_URI}\r\n{PLANNING_URI}\r\n");
    assert_eq!(published(&files, GNOME), expected_gnome.as_bytes());
    assert_eq!(published(&files, URI_LIST), expected_uri_list.as_bytes());
}

/// The text payload is the paths, one per line, with no quotes and no
/// trailing newline, as GTK's file-list text serializer writes them, so a
/// terminal pastes exactly what a shell reads.
///
/// parity: CLIP-004
#[test]
fn copy_publishes_the_paths_as_text_one_per_line() {
    let uris: Vec<String> = [
        README_URI,
        // 文档/Plan Q3.txt
        "file:///home/demo/%E6%96%87%E6%A1%A3/Plan%20Q3.txt",
        "file:///home/demo/100%25%20done.txt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    let files = ClipboardFiles::new(ClipboardMode::Copy, &uris, &context()).expect("valid selection");
    let text = String::from_utf8(published(&files, TEXT)).expect("the text is UTF-8");

    assert_eq!(
        text,
        "/home/demo/Read me.txt\n/home/demo/文档/Plan Q3.txt\n/home/demo/100% done.txt"
    );
    assert!(
        !text.contains('"'),
        "plain text is never quoted: Copy as path quotes"
    );
    assert!(!text.ends_with('\n'), "the last line ends without a newline");
    assert_eq!(TEXT, "text/plain;charset=utf-8");
}

/// An SMB item's line is its UNC path, a phone's its device path, and an
/// item with no local path keeps its address, as Copy as path writes them.
///
/// parity: CLIP-004
#[test]
fn text_shows_unc_device_and_address_only_items() {
    let locations = LocationContext {
        devices: vec![DeviceLabel {
            uri: "mtp://[usb:001,010]/".to_owned(),
            label: "Pixel 7".to_owned(),
        }],
        ..LocationContext::default()
    };
    let uris: Vec<String> = [
        "smb://studio-nas/projects/Plan%20Q3.txt",
        "mtp://[usb:001,010]/Internal%20storage/DCIM",
        "sftp://ada@build/Q3%20plans/%C3%A9t%C3%A9%20%231",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    let files = ClipboardFiles::new(ClipboardMode::Cut, &uris, &locations).expect("valid selection");
    let text = String::from_utf8(published_with(&files, TEXT, &locations)).expect("the text is UTF-8");

    assert_eq!(
        text,
        r"\\studio-nas\projects\Plan Q3.txt
Pixel 7 / Internal storage/DCIM
sftp://build/Q3 plans/été %231"
    );
}

/// A share, a server and a device root never reach the text payload:
/// [`ClipboardFiles::new`] refuses them, so nothing is published for them.
///
/// parity: CLIP-004, OPS-035
#[test]
fn share_and_device_roots_are_refused_before_any_text_is_published() {
    for uri in ["smb://nas/", "smb://nas/share", "mtp://[usb:001,010]/"] {
        assert!(
            ClipboardFiles::new(ClipboardMode::Copy, &[uri.to_owned()], &context()).is_err(),
            "{uri} is no item"
        );
    }
}

/// An address holding a character that ends a line is no line: the selection
/// then publishes no text at all, and the four file formats still name the
/// item exactly. A terminal pasting two lines would run the second as a
/// command.
///
/// Only a device's label can reach this, since `require_item_uri` refuses a
/// URI carrying an encoded control character; the app also cleans a label as
/// it reads it, so this is the guard behind the type's public
/// [`LocationContext::devices`] field.
///
/// parity: CLIP-004
#[test]
fn a_label_holding_a_line_break_or_a_unicode_separator_publishes_no_text() {
    // A phone that names itself "Pixel\n7", "Pixel\u{2028}7" or "Pixel\u{0085}7"
    // would make the path two lines for a terminal, for an editor that honours
    // Unicode separators, or for one that honours NEL, though only the first is
    // a C0 control.
    for label in ["Pixel\n7", "Pixel\u{2028}7", "Pixel\u{2029}7", "Pixel\u{85}7"] {
        let locations = LocationContext {
            devices: vec![DeviceLabel {
                uri: "mtp://[usb:001,010]/".to_owned(),
                label: label.to_owned(),
            }],
            ..LocationContext::default()
        };

        let uris = vec!["mtp://[usb:001,010]/Internal%20storage/DCIM".to_owned()];
        let files = ClipboardFiles::new(ClipboardMode::Copy, &uris, &locations).expect("a valid item");

        let published: Vec<&str> = files
            .encode(&locations)
            .into_iter()
            .map(|payload| payload.mime_type)
            .collect();
        assert_eq!(
            published,
            vec![CUSTOM, GNOME, URI_LIST, KDE_CUT],
            "no text for {label:?}, but every file format"
        );
    }
}

/// A file list read from another owner was never measured against
/// [`MAX_BYTES`], and a device label can make its paths longer than its
/// addresses: republishing it (a move that consumes part of a cut does) leaves
/// the text out rather than advertising a payload no paste would read.
///
/// parity: CLIP-004
#[test]
fn text_a_label_makes_too_large_is_left_out_of_a_republished_cut() {
    let label = "P".repeat(400);
    let locations = LocationContext {
        devices: vec![DeviceLabel {
            uri: "mtp://[usb:001,010]/".to_owned(),
            label,
        }],
        ..LocationContext::default()
    };
    let mut incoming = String::new();
    for number in 0..MAX_ITEMS {
        write!(incoming, "mtp://[usb:001,010]/DCIM{number}\r\n").expect("a string takes the write");
    }
    let incoming = decode(uri_list(None), incoming.as_bytes()).expect("an external cut");

    let published: Vec<&str> = incoming
        .encode(&locations)
        .into_iter()
        .map(|payload| payload.mime_type)
        .collect();

    assert_eq!(
        published,
        vec![CUSTOM, GNOME, URI_LIST, KDE_CUT],
        "the same items without a text payload over the size a paste reads"
    );
}

/// parity: CLIP-004
#[test]
fn selections_outside_one_to_a_hundred_thousand_items_are_refused_as_a_transfer_is() {
    let too_many = vec![README_URI.to_owned(); MAX_ITEMS + 1];
    for uris in [Vec::new(), too_many] {
        let error = ClipboardFiles::new(ClipboardMode::Copy, &uris, &context()).expect_err("item count");
        assert_eq!(error, ClipboardError::ItemCount);
        assert_eq!(error.to_string(), "Select between 1 and 100,000 items.");
    }
}

/// 100,000 items with long names can be over [`MAX_BYTES`], which a
/// paste would refuse to read: Copy and Cut refuse them instead of
/// publishing a clipboard that pastes nothing.
///
/// parity: CLIP-004
#[test]
fn a_selection_too_large_to_paste_back_is_refused() {
    // 40 CJK characters, nine encoded bytes each, make a 360-byte name.
    let name = "%E6%96%87".repeat(40);
    let uris: Vec<String> = (0..MAX_ITEMS)
        .map(|number| format!("file:///home/demo/{name}{number}"))
        .collect();

    let error = ClipboardFiles::new(ClipboardMode::Copy, &uris, &context()).expect_err("over MAX_BYTES");

    assert_eq!(error, ClipboardError::TooLarge);
    assert_eq!(
        error.to_string(),
        "This file selection is too large to copy or cut."
    );
}

/// The limit counts the text payload as well: a device label can make a
/// path longer than the address it came from, so a selection whose paths
/// alone a reader would refuse is refused before anything is published.
///
/// parity: CLIP-004
#[test]
fn a_selection_whose_text_is_over_the_size_limit_is_refused() {
    // Every item's line is the device label and its path, so a label of over
    // MAX_BYTES / MAX_ITEMS bytes takes the text over the limit while the
    // addresses stay far below it.
    let locations = LocationContext {
        devices: vec![DeviceLabel {
            uri: "mtp://[usb:001,010]/".to_owned(),
            label: "x".repeat(MAX_BYTES / MAX_ITEMS + 32),
        }],
        ..LocationContext::default()
    };
    let uris: Vec<String> = (0..MAX_ITEMS)
        .map(|number| format!("mtp://[usb:001,010]/item{number}"))
        .collect();

    let error = ClipboardFiles::new(ClipboardMode::Copy, &uris, &locations).expect_err("over MAX_BYTES");

    assert_eq!(error, ClipboardError::TooLarge);
}

/// parity: CLIP-004, OPS-035
#[test]
fn share_roots_cannot_be_copied_and_keep_the_location_message() {
    let error = ClipboardFiles::new(ClipboardMode::Cut, &["smb://nas/share".into()], &context())
        .expect_err("share root");
    let ClipboardError::Location(location_error) = &error else {
        panic!("expected a location error, got {error:?}");
    };
    assert_eq!(error.to_string(), location_error.to_string());
    assert!(
        error.to_string().starts_with("Open the network share first"),
        "{error}"
    );
}
