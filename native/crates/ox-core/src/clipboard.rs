// SPDX-License-Identifier: AGPL-3.0-only
//! File clipboard formats shared with GNOME, KDE and the Python application.
//!
//! Ports the pure format and validation functions in `desktop/file_clipboard.py`.
//! This module does not claim or clear the system clipboard. A GTK caller must
//! also reject reads whose owner changed while the payload was being fetched.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::location::{require_item_uri, LocationError};

/// OpenXplorer's existing cross-process format; the name is a compatibility contract.
pub const CUSTOM: &str = "application/x-winspace-files";
/// GNOME's combined operation and URI list.
pub const GNOME: &str = "x-special/gnome-copied-files";
/// Standard URI list. Copy semantics unless accompanied by an exact KDE cut marker.
pub const URI_LIST: &str = "text/uri-list";
/// KDE's separate cut marker (the MIME name has no `application/` prefix).
pub const KDE_CUT: &str = "x-kde-cutselection";
/// Maximum size accepted from an external clipboard.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Maximum number of items per copy or cut, before deduplication.
pub const MAX_ITEMS: usize = 200;

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
    fn wire_name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "move",
        }
    }
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
    pub fn encode(&self) -> Vec<(&'static str, Vec<u8>)> {
        if self.uris.is_empty() {
            return Vec::new();
        }
        // This struct contains only strings, an enum and an array of strings;
        // serde_json cannot fail to serialize any of those values.
        let custom = serde_json::to_vec(self).expect("clipboard fields are JSON-compatible values");
        vec![
            (CUSTOM, custom),
            (GNOME, encode_gnome(self.mode, &self.uris).into_bytes()),
            (URI_LIST, encode_uri_list(&self.uris).into_bytes()),
            (
                KDE_CUT,
                if self.mode == ClipboardMode::Cut {
                    b"1"
                } else {
                    b"0"
                }
                .to_vec(),
            ),
        ]
    }
}

#[derive(Deserialize)]
struct CustomPayload {
    mode: ClipboardMode,
    uris: Vec<String>,
    #[serde(default)]
    token: serde_json::Value,
}

/// Decodes a recognized file format; malformed, oversized and text payloads fail closed.
///
/// A KDE marker affects only `text/uri-list`. The GTK caller must read the
/// marker from the same clipboard owner; arbitrary text is never a file list.
pub fn decode(mime: &str, payload: &[u8], cut_selection: Option<&[u8]>) -> Option<ClipboardFiles> {
    if payload.is_empty() || payload.len() > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?.trim_end_matches('\0');
    if mime == CUSTOM {
        let custom: CustomPayload = serde_json::from_str(text).ok()?;
        let token = custom
            .token
            .as_str()
            .filter(|token| token.chars().count() <= 80)
            .map(str::to_owned)
            .unwrap_or_else(new_token);
        return ClipboardFiles::validated(custom.mode, &custom.uris, token).ok();
    }

    let mut lines = payload_lines(text).into_iter();
    let mode = match mime {
        GNOME => match lines.next()? {
            "copy" => ClipboardMode::Copy,
            "cut" => ClipboardMode::Cut,
            _ => return None,
        },
        URI_LIST if exact_cut_marker(cut_selection) => ClipboardMode::Cut,
        URI_LIST => ClipboardMode::Copy,
        _ => return None,
    };
    let uris: Vec<String> = lines
        .filter(|line| mime != URI_LIST || (!line.is_empty() && !line.starts_with('#')))
        .map(str::to_owned)
        .collect();
    let token = external_token(mime, payload, mode)?;
    ClipboardFiles::validated(mode, &uris, token).ok()
}

fn exact_cut_marker(marker: Option<&[u8]>) -> bool {
    marker.is_some_and(|bytes| {
        bytes.len() <= 16
            && bytes
                .strip_prefix(b"1")
                .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
    })
}

// Match Python's str.splitlines(), including bare CR and Unicode separators.
// A final line ending does not add an empty URI, and CRLF is one separator.
fn payload_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !matches!(
            character,
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
        ) {
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

fn new_token() -> String {
    glib::uuid_string_random().replace('-', "")
}

fn external_token(mime: &str, payload: &[u8], mode: ClipboardMode) -> Option<String> {
    let mut checksum = glib::Checksum::new(glib::ChecksumType::Sha256)?;
    for bytes in [
        mime.as_bytes(),
        b"\0",
        payload,
        b"\0",
        mode.wire_name().as_bytes(),
    ] {
        checksum.update(bytes);
    }
    Some(format!("external-{}", checksum.string()?))
}

/// Encodes validated URIs as a GNOME payload, with `cut` or `copy` on the first line.
fn encode_gnome(mode: ClipboardMode, uris: &[String]) -> String {
    let verb = match mode {
        ClipboardMode::Copy => "copy",
        ClipboardMode::Cut => "cut",
    };
    std::iter::once(verb.to_string())
        .chain(uris.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Encodes validated URIs as a CRLF-separated, terminated URI list.
fn encode_uri_list(uris: &[String]) -> String {
    uris.iter().map(|uri| format!("{uri}\r\n")).collect()
}

#[cfg(test)]
mod tests;
