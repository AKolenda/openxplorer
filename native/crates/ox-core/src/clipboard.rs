// SPDX-License-Identifier: AGPL-3.0-only
//! File clipboard formats shared with GNOME, KDE and the Python application.
//!
//! Ports the pure functions of `desktop/file_clipboard.py`
//! (`validate_clipboard`, `encode_clipboard`, `decode_clipboard`). This
//! module does not claim or clear the system clipboard. A GTK caller must
//! also reject reads whose owner changed while the payload was being
//! fetched.
//!
//! A selection is published in four formats at once:
//!
//! | MIME type | Read by | Payload |
//! |---|---|---|
//! | [`CUSTOM`] | Python and native app windows | JSON with mode, URIs and token |
//! | [`GNOME`] | Nautilus and other GTK apps | `copy` or `cut`, then one URI per line |
//! | [`URI_LIST`] | every file manager | CRLF-terminated URIs |
//! | [`KDE_CUT`] | Dolphin and other KDE apps | `1` for cut, `0` for copy |
//!
//! Paste tries [`CUSTOM`], then [`GNOME`], then [`URI_LIST`] with the
//! [`KDE_CUT`] marker of the same clipboard owner.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::location::{require_item_uri, LocationError};

/// The app's own cross-process format; the name is a compatibility
/// contract with the Python app and older Winspace windows.
pub const CUSTOM: &str = "application/x-winspace-files";
/// GNOME's combined operation and URI list.
pub const GNOME: &str = "x-special/gnome-copied-files";
/// Standard URI list. Copy semantics unless accompanied by an exact KDE cut
/// marker.
pub const URI_LIST: &str = "text/uri-list";
/// KDE's cut marker, published beside [`URI_LIST`]: `1` for cut, `0` for
/// copy.
///
/// This is the name KIO writes in `setClipboardDataCut` and reads in
/// `isClipboardDataCut` (`kio/src/widgets/paste.cpp`). The Python app uses
/// `x-kde-cutselection`, without the `application/` prefix, which KDE never
/// offers or asks for, so its cuts become copies in Dolphin and back.
pub const KDE_CUT: &str = "application/x-kde-cutselection";
/// Maximum size accepted from an external clipboard.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Maximum number of items per copy or cut, before deduplication.
pub const MAX_ITEMS: usize = 200;

/// The longest KDE cut marker accepted: `1` followed by NUL padding.
const MAX_CUT_MARKER_BYTES: usize = 16;
/// The longest token kept from a [`CUSTOM`] payload, in characters.
const MAX_TOKEN_CHARS: usize = 80;

/// Whether pasting copies the items or moves them after confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardMode {
    /// Keep the source items.
    #[serde(rename = "copy")]
    Copy,
    /// Move the source items; serialized as `move` for Python compatibility.
    #[serde(rename = "move")]
    Cut,
}

impl ClipboardMode {
    /// The name in the [`CUSTOM`] payload and in external fingerprints.
    fn wire_name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "move",
        }
    }

    /// The first line of a GNOME payload.
    fn gnome_verb(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "cut",
        }
    }

    fn from_gnome_verb(verb: &str) -> Option<Self> {
        match verb {
            "copy" => Some(Self::Copy),
            "cut" => Some(Self::Cut),
            _ => None,
        }
    }

    /// The [`KDE_CUT`] payload.
    fn kde_cut_marker(self) -> &'static [u8] {
        match self {
            Self::Copy => b"0",
            Self::Cut => b"1",
        }
    }
}

/// One published format of a selection: the bytes a reader gets when it
/// asks for `mime_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    /// The MIME type the payload is offered as.
    pub mime_type: &'static str,
    /// The payload itself.
    pub bytes: Vec<u8>,
}

/// A validated, ordered file selection and the identity of its clipboard owner.
///
/// The fields are private so encoding cannot bypass URI validation or inject
/// another line into an external file-list format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClipboardFiles {
    mode: ClipboardMode,
    uris: Vec<String>,
    token: String,
}

impl ClipboardFiles {
    /// Validates items, removes duplicates in selection order and assigns a token.
    ///
    /// # Errors
    ///
    /// A [`LocationError`] with the message to show when the selection has
    /// fewer than 1 or more than [`MAX_ITEMS`] items, or when an item is
    /// not a file or folder that can be copied (a share or device root, a
    /// foreign scheme, an address with credentials).
    pub fn new(mode: ClipboardMode, uris: &[String]) -> Result<Self, LocationError> {
        Self::validated(mode, uris, new_token())
    }

    fn validated(mode: ClipboardMode, uris: &[String], token: String) -> Result<Self, LocationError> {
        if !(1..=MAX_ITEMS).contains(&uris.len()) {
            return Err(LocationError::new(
                "Copy or cut between 1 and 200 items at a time.",
            ));
        }
        let mut seen = HashSet::new();
        let mut canonical = Vec::with_capacity(uris.len());
        for uri in uris {
            let uri = require_item_uri(uri)?;
            if seen.insert(uri.clone()) {
                canonical.push(uri);
            }
        }
        Ok(Self {
            mode,
            uris: canonical,
            token,
        })
    }

    /// Copy or cut semantics for this selection.
    pub fn mode(&self) -> ClipboardMode {
        self.mode
    }

    /// Canonical item URIs, in the original selection order.
    pub fn uris(&self) -> &[String] {
        &self.uris
    }

    /// Identity used to avoid consuming a newer clipboard after a slow paste.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Removes successfully moved items only when the paste still owns this cut.
    ///
    /// Returns true if anything was removed. An empty result means the caller
    /// can clear the clipboard after confirming that its owner is still current.
    pub fn consume(&mut self, token: &str, moved: &[String]) -> bool {
        // Safety rule (cut identity): a copy, or a clipboard that now holds
        // another selection, is never altered by an older paste.
        if self.mode != ClipboardMode::Cut || self.token != token {
            return false;
        }
        let original_len = self.uris.len();
        self.uris.retain(|uri| !moved.contains(uri));
        self.uris.len() != original_len
    }

    /// Encodes all four formats for a single system-clipboard owner.
    ///
    /// A consumed, empty selection has no file formats and should clear the
    /// clipboard instead of publishing an invalid empty file list.
    pub fn encode(&self) -> Vec<ClipboardPayload> {
        if self.uris.is_empty() {
            return Vec::new();
        }
        vec![
            ClipboardPayload {
                mime_type: CUSTOM,
                bytes: self.custom_json(),
            },
            ClipboardPayload {
                mime_type: GNOME,
                bytes: encode_gnome(self.mode, &self.uris).into_bytes(),
            },
            ClipboardPayload {
                mime_type: URI_LIST,
                bytes: encode_uri_list(&self.uris).into_bytes(),
            },
            ClipboardPayload {
                mime_type: KDE_CUT,
                bytes: self.mode.kde_cut_marker().to_vec(),
            },
        ]
    }

    /// The [`CUSTOM`] payload: `{"mode": ..., "uris": [...], "token": ...}`.
    fn custom_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("strings and a unit enum always serialize to JSON")
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

/// Decodes a recognized file format.
///
/// Safety rule (fail closed): malformed, oversized and plain-text payloads
/// are never a file list, whatever format they claim to be. A KDE marker
/// affects only `text/uri-list`; the GTK caller must read `cut_selection`
/// from the same clipboard owner as `payload`.
pub fn decode(mime_type: &str, payload: &[u8], cut_selection: Option<&[u8]>) -> Option<ClipboardFiles> {
    if payload.is_empty() || payload.len() > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?.trim_end_matches('\0');
    match mime_type {
        CUSTOM => decode_custom(text),
        GNOME => decode_gnome(payload, text),
        URI_LIST => decode_uri_list(payload, text, cut_selection),
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
    ClipboardFiles::validated(custom.mode, &custom.uris, token).ok()
}

/// Decodes a GNOME payload: `copy` or `cut`, then one URI per line.
fn decode_gnome(payload: &[u8], text: &str) -> Option<ClipboardFiles> {
    let mut lines = payload_lines(text).into_iter();
    let mode = ClipboardMode::from_gnome_verb(lines.next()?)?;
    let uris: Vec<String> = lines.map(str::to_owned).collect();
    let token = external_token(GNOME, payload, mode)?;
    ClipboardFiles::validated(mode, &uris, token).ok()
}

/// Decodes a URI list, skipping blank lines and `#` comments. It is a cut
/// only with an exact KDE cut marker.
fn decode_uri_list(payload: &[u8], text: &str, cut_selection: Option<&[u8]>) -> Option<ClipboardFiles> {
    let mode = if is_kde_cut_marker(cut_selection) {
        ClipboardMode::Cut
    } else {
        ClipboardMode::Copy
    };
    let uris: Vec<String> = payload_lines(text)
        .into_iter()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let token = external_token(URI_LIST, payload, mode)?;
    ClipboardFiles::validated(mode, &uris, token).ok()
}

/// Safety rule (exact cut marker): only `1`, optionally NUL-padded and at
/// most [`MAX_CUT_MARKER_BYTES`] long, grants move semantics. KIO always
/// writes exactly `1` or `0`; anything else is a copy.
fn is_kde_cut_marker(marker: Option<&[u8]>) -> bool {
    let Some(marker) = marker else {
        return false;
    };
    if marker.len() > MAX_CUT_MARKER_BYTES {
        return false;
    }
    marker
        .strip_prefix(b"1")
        .is_some_and(|padding| padding.iter().all(|byte| *byte == 0))
}

/// Splits `text` the way Python's `str.splitlines()` does, including bare
/// CR and Unicode separators. A final line ending does not add an empty
/// URI, and CRLF is one separator.
fn payload_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !is_line_separator(character) {
            continue;
        }
        lines.push(&text[start..index]);
        start = index + character.len_utf8();
        if character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n') {
            characters.next();
            start += 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The characters Python's `str.splitlines()` splits on.
fn is_line_separator(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// A fresh owner token: 32 hex digits, like Python's `uuid.uuid4().hex`.
fn new_token() -> String {
    glib::uuid_string_random().replace('-', "")
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
    checksum.update(mode.wire_name().as_bytes());
    Some(format!("external-{}", checksum.string()?))
}

/// Encodes validated URIs as a GNOME payload, with `cut` or `copy` on the
/// first line. `uris` is never empty here.
fn encode_gnome(mode: ClipboardMode, uris: &[String]) -> String {
    format!("{}\n{}", mode.gnome_verb(), uris.join("\n"))
}

/// Encodes validated URIs as a CRLF-separated, terminated URI list.
fn encode_uri_list(uris: &[String]) -> String {
    let mut list = String::new();
    for uri in uris {
        list.push_str(uri);
        list.push_str("\r\n");
    }
    list
}

#[cfg(test)]
mod tests;
