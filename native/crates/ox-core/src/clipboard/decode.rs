// SPDX-License-Identifier: AGPL-3.0-only
//! Reading a file list another window or application published.
//!
//! Ports `decode_clipboard` in `desktop/file_clipboard.py`. Every format is
//! read fail-closed: anything that is not exactly a valid file list is no
//! file list at all, so paste has nothing to do.

use serde::Deserialize;

use super::lines::python_split_lines;
use super::{new_token, ClipboardFiles, ClipboardMode, CUSTOM, GNOME, MAX_BYTES, URI_LIST};

/// The longest KDE cut marker accepted: `1` followed by NUL padding.
const MAX_CUT_MARKER_BYTES: usize = 16;
/// The longest token kept from a [`CUSTOM`] payload, in characters.
const MAX_TOKEN_CHARS: usize = 80;

/// The [`CUSTOM`] payload as read. The token may be any JSON value; only a
/// short string is kept.
#[derive(Deserialize)]
struct CustomPayload {
    mode: ClipboardMode,
    uris: Vec<String>,
    #[serde(default)]
    token: serde_json::Value,
}

/// Decodes a recognized file format.
///
/// Safety rule (fail closed): malformed, oversized and plain-text payloads
/// are never a file list, whatever format they claim to be. The
/// `kde_cut_marker` affects only `text/uri-list`; the GTK caller must read
/// it from the same clipboard owner as `payload`.
pub fn decode(mime_type: &str, payload: &[u8], kde_cut_marker: Option<&[u8]>) -> Option<ClipboardFiles> {
    if payload.is_empty() || payload.len() > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?.trim_end_matches('\0');
    match mime_type {
        CUSTOM => decode_custom(text),
        GNOME => decode_gnome(payload, text),
        URI_LIST => decode_uri_list(payload, text, kde_cut_marker),
        _ => None,
    }
}

/// Decodes the [`CUSTOM`] format, keeping its token so a paste in
/// another window can consume the cut it read.
fn decode_custom(text: &str) -> Option<ClipboardFiles> {
    let custom: CustomPayload = serde_json::from_str(text).ok()?;
    let token = custom
        .token
        .as_str()
        .filter(|token| token.chars().count() <= MAX_TOKEN_CHARS)
        .map_or_else(new_token, str::to_owned);
    ClipboardFiles::with_token(custom.mode, &custom.uris, token).ok()
}

/// Decodes a [`GNOME`] payload: `copy` or `cut`, then one URI per line.
fn decode_gnome(payload: &[u8], text: &str) -> Option<ClipboardFiles> {
    let mut lines = python_split_lines(text).into_iter();
    let mode = ClipboardMode::from_gnome_verb(lines.next()?)?;
    let uris: Vec<String> = lines.map(str::to_owned).collect();
    let token = external_token(GNOME, payload, mode)?;
    ClipboardFiles::with_token(mode, &uris, token).ok()
}

/// Decodes a URI list, skipping blank lines and `#` comments. It is a cut
/// only with an exact KDE cut marker.
fn decode_uri_list(payload: &[u8], text: &str, kde_cut_marker: Option<&[u8]>) -> Option<ClipboardFiles> {
    let mode = if is_exact_cut_marker(kde_cut_marker) {
        ClipboardMode::Cut
    } else {
        ClipboardMode::Copy
    };
    let uris: Vec<String> = python_split_lines(text)
        .into_iter()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let token = external_token(URI_LIST, payload, mode)?;
    ClipboardFiles::with_token(mode, &uris, token).ok()
}

/// Safety rule (exact cut marker): only `1`, optionally NUL-padded and at
/// most [`MAX_CUT_MARKER_BYTES`] long, grants move semantics. KIO always
/// writes exactly `1` or `0`; anything else is a copy.
fn is_exact_cut_marker(marker: Option<&[u8]>) -> bool {
    let Some(marker) = marker else {
        return false;
    };
    if marker.len() > MAX_CUT_MARKER_BYTES {
        return false;
    }
    let Some(padding) = marker.strip_prefix(b"1") else {
        return false;
    };
    padding.iter().all(|byte| *byte == 0)
}

/// A stable identity for a payload another application published.
///
/// External formats carry no token. Re-reading the same payload must keep
/// its identity so moved items can be consumed across reads, while a
/// changed payload must never be cleared by an older paste. The fingerprint
/// is the one `decode_clipboard` computes in Python, so Python and native
/// windows agree on it.
fn external_token(mime_type: &str, payload: &[u8], mode: ClipboardMode) -> Option<String> {
    let mut checksum = glib::Checksum::new(glib::ChecksumType::Sha256)?;
    checksum.update(mime_type.as_bytes());
    checksum.update(b"\0");
    checksum.update(payload);
    checksum.update(b"\0");
    checksum.update(mode.as_str().as_bytes());
    Some(format!("external-{}", checksum.string()?))
}
