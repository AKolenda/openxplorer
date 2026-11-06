// SPDX-License-Identifier: AGPL-3.0-only
//! File clipboard formats shared with GNOME, KDE and the Python application.
//!
//! Ports the pure functions of `v2.0.0:desktop/file_clipboard.py`
//! (`validate_clipboard`, `encode_clipboard`, `decode_clipboard`). This
//! module does not claim or clear the system clipboard. A GTK caller must
//! also reject reads whose owner changed while the payload was being
//! fetched.
//!
//! A selection is published in five formats at once:
//!
//! | MIME type | Read by | Payload |
//! |---|---|---|
//! | [`CUSTOM`] | Python and native app windows | JSON with mode, URIs and token |
//! | [`GNOME`] | Nautilus and other GTK apps | `copy` or `cut`, then one URI per line |
//! | [`URI_LIST`] | every file manager | CRLF-terminated URIs |
//! | [`TEXT`] | terminals and editors that take text | each item's path, one per line, when every path is one line |
//! | [`KDE_CUT`] | Dolphin and other KDE apps | `1` for cut, `0` for copy |
//!
//! Paste tries [`CUSTOM`], then [`GNOME`], then [`URI_LIST`] with the
//! [`KDE_CUT`] marker of the same clipboard owner. [`FileListFormat`] names
//! these three formats, and [`decode()`] reads one of them. [`TEXT`] is
//! published for other applications only: text is never read back as files.

mod decode;
mod lines;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::location::{breaks_a_line, require_item_uri, LocationContext, LocationError};

pub use decode::{decode, FileListFormat};

/// The app's own cross-process format; the name is a compatibility
/// contract with the Python app and older Winspace windows.
pub const CUSTOM: &str = "application/x-winspace-files";
/// GNOME's combined operation and URI list.
pub const GNOME: &str = "x-special/gnome-copied-files";
/// Standard URI list. Copy semantics unless accompanied by an exact KDE cut
/// marker.
pub const URI_LIST: &str = "text/uri-list";
/// The items' addresses as plain text, one per line, unquoted, for
/// terminals, editors and anything else that pastes text only.
///
/// GTK publishes it for a file list (the `text/plain` serializer of
/// `gdk/gdkcontentserializer.c`), which is why copying in Nautilus and
/// pasting into a terminal gives the files' paths. It carries
/// [`LocationContext::copied_location`], so a local item is its path, an SMB
/// item its `\\server\share\…` path and a phone's item `Pixel 7 / path`.
/// Nothing is quoted: Windows' quoted form belongs to Copy as path. A
/// selection with an address that holds a control character publishes no
/// text at all, since pasting it into a terminal would paste more lines than
/// the items it names ([`ClipboardFiles::encode`]).
pub const TEXT: &str = "text/plain;charset=utf-8";
/// KDE's cut marker, published beside [`URI_LIST`]: `1` for cut, `0` for
/// copy.
///
/// This is the name KIO writes in `setClipboardDataCut` and reads in
/// `isClipboardDataCut` (`kio/src/widgets/paste.cpp`). The Python app uses
/// `x-kde-cutselection`, without the `application/` prefix, which KDE never
/// offers or asks for, so its cuts become copies in Dolphin and back.
pub const KDE_CUT: &str = "application/x-kde-cutselection";
/// Maximum size accepted from an external clipboard: room for
/// [`MAX_ITEMS`] addresses of about 330 bytes each, so a paste of the
/// largest selection this app copies is never refused for its size.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
/// Maximum number of items per copy or cut, before deduplication: as many
/// as one transfer takes ([`crate::transfer::MAX_ITEMS`]). Windows
/// Explorer sets no limit of its own.
pub const MAX_ITEMS: usize = crate::transfer::MAX_ITEMS;

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
    #[error("{}", crate::i18n::gettext("Select between 1 and 100,000 items."))]
    ItemCount,
    /// The selection's addresses are over [`MAX_BYTES`] in a published
    /// format, which no paste would read back.
    #[error(
        "{}",
        crate::i18n::gettext("This file selection is too large to copy or cut.")
    )]
    TooLarge,
    /// An item is not a file or folder that can be copied: a share or
    /// device root, a foreign scheme or an address with credentials.
    #[error(transparent)]
    Location(#[from] LocationError),
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
    /// fresh owner token. `locations` names the devices and the home folder,
    /// whose labels the [`TEXT`] payload shows.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::ItemCount`] when the selection has fewer than 1 or
    /// more than [`MAX_ITEMS`] items, [`ClipboardError::Location`] when an
    /// item is not a file or folder that can be copied, and
    /// [`ClipboardError::TooLarge`] when a published format would be over
    /// [`MAX_BYTES`], so a paste could not read it back.
    pub fn new(
        mode: ClipboardMode,
        uris: &[String],
        locations: &LocationContext,
    ) -> Result<Self, ClipboardError> {
        let files = Self::with_token(mode, uris, new_token())?;
        // The CUSTOM payload holds every URI in quotes with a comma, and the
        // text every path, which a device label can make longer than its
        // address; the other formats are smaller. A paste refuses a payload
        // over MAX_BYTES, so publishing one would leave nothing to paste.
        let text_bytes = files.text(locations).map_or(0, |text| text.len());
        if files.custom_json().len() > MAX_BYTES || text_bytes > MAX_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        Ok(files)
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
            // v2.0.0:desktop/file_clipboard.py, `require_item_uri` in v2.0.0:desktop/core.py).
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
        let moved: HashSet<&str> = moved.iter().map(String::as_str).collect();
        let original_len = self.uris.len();
        self.uris.retain(|uri| !moved.contains(uri.as_str()));
        self.uris.len() != original_len
    }

    /// Encodes the formats for a single system-clipboard owner: the four
    /// file-list formats, and [`TEXT`] when every item's address is one line
    /// and the whole of it fits a paste.
    ///
    /// A consumed, empty selection has no file formats and should clear the
    /// clipboard instead of publishing an invalid empty file list.
    pub fn encode(&self, locations: &LocationContext) -> Vec<ClipboardPayload> {
        if self.uris.is_empty() {
            return Vec::new();
        }
        let mut payloads = vec![
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
        ];
        if let Some(text) = self.text(locations) {
            // A selection this app made was measured by ClipboardFiles::new,
            // but one decoded from another owner was not, and the move that
            // consumes a cut republishes what is left with the labels of this
            // moment: a phone's label alone can make 100,000 short addresses
            // into more text than a paste reads. Publishing that would
            // advertise a format no paste accepts, so the text is left out
            // and the file formats still name every item.
            if text.len() <= MAX_BYTES {
                payloads.push(ClipboardPayload {
                    mime_type: TEXT,
                    bytes: text.into_bytes(),
                });
            }
        }
        payloads.push(ClipboardPayload {
            mime_type: KDE_CUT,
            bytes: self.mode.kde_cut_marker().to_vec(),
        });
        payloads
    }

    /// The [`TEXT`] payload: each item's display address, one per line,
    /// unquoted, without a trailing newline, as GTK's file-list text
    /// serializer writes them; `None` when there is no such payload.
    ///
    /// Safety rule (one line per item): an address holding a character that
    /// ends a line is no line, so the selection publishes no text rather than
    /// a payload whose lines are not items, which a terminal would paste as
    /// more than was copied. A control character is one, and so is U+2028 or
    /// U+2029 ([`breaks_a_line`]), which an editor honours as a line break.
    /// No URI can bring a control character, since [`require_item_uri`]
    /// refuses an encoded one; a device's label could, and the app cleans a
    /// label as it reads it. This is the guard behind the public `devices`
    /// field that keeps such a name from ever becoming a second line.
    fn text(&self, locations: &LocationContext) -> Option<String> {
        let lines: Vec<String> = self
            .uris
            .iter()
            .map(|uri| locations.copied_location(uri))
            .collect();
        if lines.iter().any(|line| line.chars().any(breaks_a_line)) {
            return None;
        }
        Some(lines.join("\n"))
    }

    /// The [`CUSTOM`] payload: `{"mode": ..., "uris": [...], "token": ...}`.
    fn custom_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("strings and a unit enum always serialize to JSON")
    }
}

/// A fresh owner token: 32 hex digits, in the form of Python's
/// `uuid.uuid4().hex`. The token only tells one cut from another and is
/// published on the clipboard anyway, so `GLib`'s UUID, which cannot fail,
/// is enough (see [`crate::random`]).
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
