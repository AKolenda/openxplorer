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
//! [`KDE_CUT`] marker of the same clipboard owner. [`FileListFormat`] names
//! these three formats, and [`decode()`] reads one of them.

mod decode;
mod lines;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::location::{require_item_uri, LocationError};

pub use decode::{decode, FileListFormat};

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
    /// The mode's name in the [`CUSTOM`] payload and in Python: `copy` or
    /// `move`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "move",
        }
    }

    /// The first line of a [`GNOME`] payload.
    fn gnome_verb(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "cut",
        }
    }

    /// The mode a [`GNOME`] payload's first line names, if it names one.
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

/// Why a selection cannot be copied or cut.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardError {
    /// The selection is empty or has more than [`MAX_ITEMS`] items.
    #[error("Copy or cut between 1 and {max} items at a time.", max = MAX_ITEMS)]
    ItemCount,
    /// An item is not a file or folder that can be copied: a share or
    /// device root, a foreign scheme or an address with credentials.
    #[error(transparent)]
    Item(#[from] LocationError),
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

/// A validated, ordered file selection and the identity of its clipboard
/// owner. Serializes as the [`CUSTOM`] payload.
///
/// The fields are private so that no selection can bypass the safety rule
/// (validated items) that [`ClipboardFiles::new`] enforces, and so that
/// encoding can never inject another line into an external file-list format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClipboardFiles {
    mode: ClipboardMode,
    uris: Vec<String>,
    token: String,
}

impl ClipboardFiles {
    /// Validates items, removes duplicates in selection order and assigns a
    /// fresh owner token.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::ItemCount`] when the selection has fewer than 1 or
    /// more than [`MAX_ITEMS`] items, and [`ClipboardError::Item`] when an
    /// item is not a file or folder that can be copied.
    pub fn new(mode: ClipboardMode, uris: &[String]) -> Result<Self, ClipboardError> {
        Self::with_token(mode, uris, new_token())
    }

    /// [`ClipboardFiles::new`] with an owner token read from a clipboard.
    fn with_token(mode: ClipboardMode, uris: &[String], token: String) -> Result<Self, ClipboardError> {
        if !(1..=MAX_ITEMS).contains(&uris.len()) {
            return Err(ClipboardError::ItemCount);
        }
        let mut seen = HashSet::new();
        let mut canonical = Vec::with_capacity(uris.len());
        for uri in uris {
            // Safety rule (validated items): only canonical file and folder
            // URIs enter a selection, so no payload can carry credentials, a
            // share or device root, or an encoded line break into the GNOME
            // or URI-list formats (`validate_clipboard` in
            // desktop/file_clipboard.py, `require_item_uri` in desktop/core.py).
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

    /// Removes successfully moved items only when the paste still owns this
    /// cut.
    ///
    /// Returns `true` if any URI was removed. When [`ClipboardFiles::uris`]
    /// is empty afterwards, [`ClipboardFiles::encode`] publishes nothing and
    /// the caller should clear the clipboard after confirming that its
    /// owner is still current.
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

/// A fresh owner token: 32 hex digits, like Python's `uuid.uuid4().hex`.
fn new_token() -> String {
    glib::uuid_string_random().replace('-', "")
}

/// Encodes validated URIs as a [`GNOME`] payload, with `cut` or `copy` on
/// the first line. `uris` is never empty here.
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
