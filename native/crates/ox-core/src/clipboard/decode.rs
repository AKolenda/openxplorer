// SPDX-License-Identifier: AGPL-3.0-only
//! Reading a file list another window or application published.
//!
//! Ports `decode_clipboard` in `v2.0.0:desktop/file_clipboard.py`. Every format is
//! read fail-closed: anything that is not exactly a valid file list is no
//! file list at all, so paste has nothing to do.

use serde::Deserialize;

use super::lines::python_split_lines;
use super::{new_token, ClipboardFiles, ClipboardMode, CUSTOM, GNOME, MAX_BYTES, URI_LIST};

/// The longest KDE cut marker accepted: `1` followed by NUL padding.
const MAX_CUT_MARKER_BYTES: usize = 16;
/// The longest token kept from a [`CUSTOM`] payload, in characters.
const MAX_TOKEN_CHARS: usize = 80;

/// A file-list format another window may publish, in the order paste tries
/// them.
///
/// Plain text is deliberately not a variant: text copied in another
/// application is never pasted as files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileListFormat<'a> {
    /// [`CUSTOM`]: JSON with mode, URIs and owner token.
    Custom,
    /// [`GNOME`]: `copy` or `cut`, then one URI per line.
    Gnome,
    /// [`URI_LIST`]: one URI per line, a cut only with an exact KDE marker.
    UriList {
        /// The [`KDE_CUT`](super::KDE_CUT) payload, which the caller must
        /// read from the same clipboard owner as the list; `None` when the
        /// owner offers none.
        kde_cut_marker: Option<&'a [u8]>,
    },
}

impl FileListFormat<'_> {
    /// The MIME type the format is published and read as.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Custom => CUSTOM,
            Self::Gnome => GNOME,
            Self::UriList { .. } => URI_LIST,
        }
    }

    /// The file-list format published as `mime_type`, such as the one a
    /// clipboard read reports it chose.
    ///
    /// A URI list comes without its KDE marker, which the caller reads next
    /// from the same owner. Plain text and the [`KDE_CUT`](super::KDE_CUT)
    /// marker itself are not file lists, so they give `None`.
    pub fn from_mime_type(mime_type: &str) -> Option<Self> {
        match mime_type {
            CUSTOM => Some(Self::Custom),
            GNOME => Some(Self::Gnome),
            URI_LIST => Some(Self::UriList { kde_cut_marker: None }),
            _ => None,
        }
    }
}

/// The [`CUSTOM`] payload as read. The token may be any JSON value; only a
/// short string is kept.
#[derive(Deserialize)]
struct CustomPayload {
    mode: ClipboardMode,
    uris: Vec<String>,
    #[serde(default)]
    token: serde_json::Value,
}

/// Decodes a file list published in `format`.
///
/// Safety rule (fail closed): malformed and oversized payloads are never a
/// file list, whatever format they claim to be, and plain text cannot even
/// be named as a [`FileListFormat`].
pub fn decode(format: FileListFormat<'_>, payload: &[u8]) -> Option<ClipboardFiles> {
    if payload.is_empty() || payload.len() > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?.trim_end_matches('\0');
    match format {
        FileListFormat::Custom => decode_custom(text),
        FileListFormat::Gnome => decode_gnome(payload, text),
        FileListFormat::UriList { kde_cut_marker } => decode_uri_list(payload, text, kde_cut_marker),
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
