// SPDX-License-Identifier: AGPL-3.0-only
//! Clipboard protocol regressions from `desktop/tests/test_file_clipboard_interop.py`.

use super::*;

const ONE: &str = "file:///home/demo/Read%20me.txt";
const TWO: &str = "file:///home/demo/Planning.pdf";

fn selection(mode: ClipboardMode) -> ClipboardFiles {
    ClipboardFiles::new(mode, &[ONE.into(), TWO.into()]).expect("valid selection")
}

fn published(files: &ClipboardFiles, mime_type: &str) -> Vec<u8> {
    files
        .encode()
        .into_iter()
        .find(|payload| payload.mime_type == mime_type)
        .unwrap_or_else(|| panic!("{mime_type} is published"))
        .bytes
}

/// parity: CLIP-008
#[test]
fn gnome_cut_keeps_identity_across_reads_and_consumes_only_successful_items() {
    let payload = format!("cut\n{ONE}\n{TWO}");
    let mut files = decode(GNOME, payload.as_bytes(), None).expect("GNOME cut");
    let again = decode(GNOME, payload.as_bytes(), None).expect("same owner");
    assert_eq!(files.token(), again.token());
    assert!(files.consume(again.token(), &[ONE.into()]));
    assert_eq!(files.uris(), &[TWO]);
    assert!(files.consume(again.token(), &[TWO.into()]));
    assert!(files.encode().is_empty());
}

/// parity: CLIP-008
#[test]
fn old_cut_cannot_consume_a_changed_payload_or_a_copy() {
    let first = decode(GNOME, format!("cut\n{ONE}").as_bytes(), None).expect("cut");
    for payload in [format!("cut\n{TWO}"), format!("copy\n{ONE}")] {
        let mut changed = decode(GNOME, payload.as_bytes(), None).expect("changed selection");
        assert!(!changed.consume(first.token(), &[ONE.into(), TWO.into()]));
        assert_eq!(changed.uris().len(), 1);
    }
}

/// parity: CLIP-006
#[test]
fn kde_requires_an_exact_short_cut_marker() {
    let payload = format!("{ONE}\r\n{TWO}\r\n");
    for marker in [
        None,
        Some(&b"0"[..]),
        Some(b"cut"),
        Some(b"1\n"),
        Some(&[b'1'; 20]),
        Some(&[0; 17]),
    ] {
        let files = decode(URI_LIST, payload.as_bytes(), marker).expect("URI list");
        assert_eq!(files.mode(), ClipboardMode::Copy, "marker: {marker:?}");
    }
    for marker in [b"1".as_slice(), b"1\0"] {
        let files = decode(URI_LIST, payload.as_bytes(), Some(marker)).expect("KDE cut");
        assert_eq!(files.mode(), ClipboardMode::Cut);
    }
    let gnome = decode(GNOME, format!("copy\n{ONE}").as_bytes(), Some(b"1")).expect("GNOME copy");
    assert_eq!(gnome.mode(), ClipboardMode::Copy);
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

/// parity: CLIP-005
#[test]
fn uri_list_discards_comments_and_deduplicates_canonical_addresses() {
    let payload =
        b"# copied files\r\n\r\nfile://localhost/tmp/a\r\nfile:///tmp/a\r\nsmb://STUDIO-NAS/Shared/a\r\n";
    let files = decode(URI_LIST, payload, None).expect("canonical list");
    assert_eq!(files.uris(), &["file:///tmp/a", "smb://studio-nas/Shared/a"]);
}

/// parity: CLIP-005
#[test]
fn invalid_external_payloads_fail_closed() {
    let oversized = vec![b'x'; MAX_BYTES + 1];
    for payload in [
        b"".as_slice(),
        b"\xff",
        b"https://example.test/file",
        b"smb://studio-nas/Shared",
        b"file:///tmp/a%0Ab",
        oversized.as_slice(),
    ] {
        assert!(decode(URI_LIST, payload, Some(b"1")).is_none());
    }
    assert!(decode("text/plain", ONE.as_bytes(), Some(b"1")).is_none());
    assert!(decode(GNOME, format!("move\n{ONE}").as_bytes(), None).is_none());
    let too_many = std::iter::repeat_n(ONE, MAX_ITEMS + 1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(decode(URI_LIST, too_many.as_bytes(), None).is_none());
}

/// parity: CLIP-005
#[test]
fn custom_payload_preserves_token_and_rejects_invalid_operations() {
    let custom = format!(r#"{{"mode":"move","uris":["{ONE}"],"token":"previous-owner"}}"#);
    let files = decode(CUSTOM, custom.as_bytes(), None).expect("custom payload");
    assert_eq!(files.token(), "previous-owner");
    assert_eq!(files.mode(), ClipboardMode::Cut);
    assert!(decode(CUSTOM, br#"{"mode":"delete","uris":["file:///tmp/a"]}"#, None).is_none());
    assert!(decode(CUSTOM, br#"{"mode":"copy","uris":[]}"#, None).is_none());
    let no_token =
        decode(CUSTOM, br#"{"mode":"copy","uris":["file:///tmp/a"]}"#, None).expect("missing token");
    assert_eq!(no_token.token().len(), 32);
}

/// parity: CLIP-004
#[test]
fn every_advertised_format_round_trips() {
    for mode in [ClipboardMode::Copy, ClipboardMode::Cut] {
        let files = selection(mode);
        let marker = published(&files, KDE_CUT);
        let file_lists = files
            .encode()
            .into_iter()
            .filter(|payload| payload.mime_type != KDE_CUT);
        for payload in file_lists {
            let result =
                decode(payload.mime_type, &payload.bytes, Some(&marker)).expect("published format decodes");
            assert_eq!(result.mode(), mode);
            assert_eq!(result.uris(), files.uris());
            if payload.mime_type == CUSTOM {
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
    let files = decode(GNOME, b"cut\nfile:///tmp/a", None).expect("cut");
    let script = "from file_clipboard import decode_clipboard, GNOME; print(decode_clipboard(GNOME, b'cut\\nfile:///tmp/a')['token'])";
    let output = std::process::Command::new("python3")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../desktop"))
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
        let payload = format!("cut{separator}{ONE}{separator}{TWO}{separator}");
        let files = decode(GNOME, payload.as_bytes(), None).expect("legacy line endings");
        assert_eq!(files.mode(), ClipboardMode::Cut);
        assert_eq!(files.uris(), &[ONE, TWO]);
    }
}

#[test]
fn published_file_lists_match_the_python_encoding() {
    let files = selection(ClipboardMode::Cut);
    let gnome = format!("cut\n{ONE}\n{TWO}");
    let uri_list = format!("{ONE}\r\n{TWO}\r\n");
    assert_eq!(published(&files, GNOME), gnome.as_bytes());
    assert_eq!(published(&files, URI_LIST), uri_list.as_bytes());
}
