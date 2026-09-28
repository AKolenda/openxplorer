// SPDX-License-Identifier: AGPL-3.0-only
//! Clipboard protocol regressions from
//! `desktop/tests/test_file_clipboard_interop.py` and the `ClipboardTests`
//! in `desktop/tests/test_v05.py`.

use super::*;

/// The first file of the fixture selections; its name needs percent-encoding.
const README_URI: &str = "file:///home/demo/Read%20me.txt";
/// The second file of the fixture selections.
const PLANNING_URI: &str = "file:///home/demo/Planning.pdf";

/// A valid two-item selection with a fresh token.
fn selection(mode: ClipboardMode) -> ClipboardFiles {
    ClipboardFiles::new(mode, &[README_URI.into(), PLANNING_URI.into()]).expect("valid selection")
}

/// The bytes `files` publishes as `mime_type`.
fn published(files: &ClipboardFiles, mime_type: &str) -> Vec<u8> {
    files
        .encode()
        .into_iter()
        .find(|payload| payload.mime_type == mime_type)
        .unwrap_or_else(|| panic!("{mime_type} is published"))
        .bytes
}

/// A [`URI_LIST`] read with `kde_cut_marker` from the same clipboard owner.
fn uri_list(kde_cut_marker: Option<&[u8]>) -> FileListFormat<'_> {
    FileListFormat::UriList { kde_cut_marker }
}

/// Ported from `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_gnome_cut_can_consume_successful_items_across_reads`
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
    assert!(files.encode().is_empty());
}

/// Ported from `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_changed_external_payload_is_not_consumed_by_old_operation`
/// and `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_changed_mode_is_not_consumed_by_old_cut`
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

/// Ported from `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_uri_list_without_exact_kde_cut_marker_remains_copy`,
/// `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_nul_terminated_kde_cut_marker`
/// and `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_gnome_copy_ignores_unrelated_kde_cut_marker`
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

/// Ported from `desktop/tests/test_v05.py::ClipboardTests::test_deduplication`
///
/// parity: CLIP-005
#[test]
fn uri_list_discards_comments_and_deduplicates_canonical_addresses() {
    let payload =
        b"# copied files\r\n\r\nfile://localhost/tmp/a\r\nfile:///tmp/a\r\nsmb://STUDIO-NAS/Shared/a\r\n";
    let files = decode(uri_list(None), payload).expect("canonical list");
    assert_eq!(files.uris(), &["file:///tmp/a", "smb://studio-nas/Shared/a"]);
}

/// Ported from `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_kde_marker_cannot_turn_plain_text_into_files`,
/// `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_kde_marker_cannot_turn_invalid_uri_list_into_files`
/// and the `test_plain_text_is_not_file_clipboard`, `test_unsafe_scheme`,
/// `test_no_password_in_clipboard_uri`, `test_share_itself_not_transferable`,
/// `test_malformed_json` and `test_large_rejected` cases of
/// `desktop/tests/test_v05.py::ClipboardTests`
///
/// parity: CLIP-005, SAFE-010
#[test]
fn invalid_external_payloads_fail_closed() {
    // Plain text is not a file-list format, so it is never decoded at all.
    assert_eq!(FileListFormat::from_mime_type("text/plain"), None);
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
        RefusedPayload::uri_list("more than 200 items", None, too_many_items),
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

/// Ported from `desktop/tests/test_file_clipboard_interop.py::ExternalClipboardTests::test_custom_payload_keeps_priority_and_its_token`
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

/// Ported from `desktop/tests/test_v05.py::ClipboardTests::test_custom_copy_roundtrip`,
/// `desktop/tests/test_v05.py::ClipboardTests::test_custom_cut_roundtrip`
/// and `desktop/tests/test_v05.py::ClipboardTests::test_gnome_cut_flag`
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
        let advertised: Vec<&str> = files.encode().iter().map(|payload| payload.mime_type).collect();
        assert_eq!(advertised, [CUSTOM, GNOME, URI_LIST, KDE_CUT]);
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
    let desktop = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../desktop");
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

/// parity: CLIP-004
#[test]
fn selections_outside_one_to_two_hundred_items_are_refused_in_the_python_wording() {
    let too_many = vec![README_URI.to_owned(); MAX_ITEMS + 1];
    for uris in [Vec::new(), too_many] {
        let error = ClipboardFiles::new(ClipboardMode::Copy, &uris).expect_err("item count");
        assert_eq!(error, ClipboardError::ItemCount);
        assert_eq!(
            error.to_string(),
            "Copy or cut between 1 and 200 items at a time."
        );
    }
}

/// parity: CLIP-004, OPS-035
#[test]
fn share_roots_cannot_be_copied_and_keep_the_location_message() {
    let error = ClipboardFiles::new(ClipboardMode::Cut, &["smb://nas/share".into()]).expect_err("share root");
    let ClipboardError::Location(location_error) = &error else {
        panic!("expected a location error, got {error:?}");
    };
    assert_eq!(error.to_string(), location_error.to_string());
    assert!(
        error.to_string().starts_with("Open the network share first"),
        "{error}"
    );
}
