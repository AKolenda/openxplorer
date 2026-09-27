// SPDX-License-Identifier: AGPL-3.0-only
//! File clipboard formats shared with other file managers.
//!
//! Ports the format half of `desktop/file_clipboard.py` (lines 1-69) and the
//! URI-list validation in `desktop/native_file_drop.py` (`decode_uris`):
//! `x-special/gnome-copied-files`, `text/uri-list` and KDE's
//! `application/x-kde-cutselection`. Only an exact cut marker means move;
//! plain text never becomes a file list.

/// Copy or cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Cut,
}

/// `x-special/gnome-copied-files` payload.
pub fn encode_gnome(mode: ClipboardMode, uris: &[String]) -> String {
    let verb = match mode {
        ClipboardMode::Copy => "copy",
        ClipboardMode::Cut => "cut",
    };
    std::iter::once(verb.to_string())
        .chain(uris.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text/uri-list` payload (CRLF separated).
pub fn encode_uri_list(uris: &[String]) -> String {
    uris.iter().map(|u| format!("{u}\r\n")).collect()
}
