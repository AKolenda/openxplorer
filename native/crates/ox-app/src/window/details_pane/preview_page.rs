// SPDX-License-Identifier: AGPL-3.0-only
//! What the Preview pane (View > Preview pane, Alt+P; VIEW-070) shows of
//! the selection, decided without widgets so it is tested on its own, and
//! how a text file's first bytes become the text it shows.
//!
//! As Windows 11 File Explorer's Preview pane: a text or source file's
//! beginning as plain, read-only text in a fixed-width font that wraps at
//! the pane's edge (the Windows TXT Previewer cannot turn wrapping off), a
//! PDF's pages one under the other, a picture as large as the pane allows
//! and never stretched, and "No preview available." in the middle of the
//! pane for a folder, several items, a file of another type, or a file
//! the app may not read (on a network share with network previews off).
//! With nothing selected Windows says "Select a file to preview.". Audio
//! and video keep the details pane's own player ([`PreviewPage::Media`]).

use gtk::gio;
use gtk::prelude::FileExt;

use crate::folder_view::item::FileItem;
use crate::i18n::message_id;

/// What Windows' Preview pane says when it has nothing to show.
pub(in crate::window) const NO_PREVIEW: &str = message_id("No preview available.");
/// What Windows' Preview pane says with nothing selected.
pub(in crate::window) const SELECT_A_FILE: &str = message_id("Select a file to preview.");

/// How much of a text file is read: its first mebibyte. Windows' text
/// previewer shows a long file's beginning without a note, so the rest is
/// left out silently.
pub(in crate::window) const TEXT_LIMIT: usize = 1024 * 1024;

/// The longest line drawn as one paragraph; a longer one (a minified
/// script, a log of one line) is broken there, which looks the same in a
/// pane that wraps anyway and keeps the text's layout quick.
const LINE_LIMIT: usize = 4096;

/// The most lines shown, after long ones are broken: a file of many short
/// lines (a log, a CSV) shows its first ones, as its first mebibyte.
pub(in crate::window) const LINE_COUNT_LIMIT: usize = 20_000;

/// The share of a text's bytes that may be invalid UTF-8 or control
/// characters before it counts as binary: one in fifty.
const BINARY_SHARE: usize = 50;

/// The extensions previewed as text whatever type the system names for
/// them: Windows' own text previewer takes these, and the system may
/// call a `.ts` script a video or a `.log` file plain data.
const TEXT_EXTENSIONS: [&str; 31] = [
    "txt", "text", "md", "markdown", "log", "csv", "tsv", "json", "xml", "ini", "conf", "cfg", "toml",
    "yaml", "yml", "rs", "py", "js", "mjs", "ts", "tsx", "jsx", "c", "cc", "cpp", "h", "hpp", "sh", "html",
    "htm", "css",
];

/// The types that are text without saying so through `text/plain`.
const TEXT_TYPES: [&str; 8] = [
    "application/json",
    "application/xml",
    "application/javascript",
    "application/x-shellscript",
    "application/toml",
    "application/x-yaml",
    "application/x-desktop",
    "application/sql",
];

/// What the Preview pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum PreviewPage {
    /// Words in the middle of the pane: [`NO_PREVIEW`] or
    /// [`SELECT_A_FILE`] (message ids, translated when shown).
    Message(&'static str),
    /// The beginning of the text file at `uri`, as it was at `modified`.
    Text(PreviewFile),
    /// The picture at `uri`.
    Picture(PreviewFile),
    /// The pages of the local PDF at `uri`.
    Pdf(PreviewFile),
    /// The details pane's own preview: the player of an audio or video
    /// file, as the Preview pane showed it before.
    Media,
}

/// The file a page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct PreviewFile {
    /// The file.
    pub uri: String,
    /// When it was changed, so a changed file is read again.
    pub modified: Option<u64>,
    /// Network previews are off: the file is read only once the file
    /// system it is really on (after its links) is found to be no network
    /// one, as its thumbnail is (VIEW-058). That is asked off the main
    /// thread, where a server that does not answer holds nothing up.
    pub local_only: bool,
}

/// What [`preview_page`] reads besides the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) struct PreviewRules {
    /// The folder or the selected item is on a network share, a phone or
    /// a camera, by its location: a recent item or a search result is
    /// judged by its own.
    pub remote: bool,
    /// The user lets such folders show previews (Settings' network
    /// previews, which thumbnails follow too).
    pub remote_previews: bool,
}

/// What the Preview pane shows for `selection`.
pub(in crate::window) fn preview_page(selection: &[FileItem], rules: PreviewRules) -> PreviewPage {
    let item = match selection {
        [] => return PreviewPage::Message(SELECT_A_FILE),
        [item] => item,
        _ => return PreviewPage::Message(NO_PREVIEW),
    };
    let entry = item.entry();
    if entry.is_dir || entry.is_virtual || (rules.remote && !rules.remote_previews) {
        return PreviewPage::Message(NO_PREVIEW);
    }
    // A file on a share is read through its local path, which GIO's FUSE
    // folder gives; an archive's member or a phone's file without one is
    // not read.
    let file = gio::File::for_uri(&entry.uri);
    if file.path().is_none() {
        return PreviewPage::Message(NO_PREVIEW);
    }
    let content_type = entry.content_type.as_deref().unwrap_or_default();
    let kind = content_type.split('/').next().unwrap_or_default();
    let file = PreviewFile {
        uri: entry.uri.clone(),
        modified: entry.modified,
        local_only: !rules.remote_previews,
    };
    if is_text(content_type, &entry.name) {
        PreviewPage::Text(file)
    } else if kind == "image" {
        PreviewPage::Picture(file)
    } else if content_type == "application/pdf" || extension_of(&entry.name) == "pdf" {
        PreviewPage::Pdf(file)
    } else if kind == "audio" || kind == "video" {
        PreviewPage::Media
    } else {
        PreviewPage::Message(NO_PREVIEW)
    }
}

/// The lower-case extension of the file name `name`, or nothing.
fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether a file of `content_type` named `name` is shown as text: a
/// picture's type wins, so an SVG is drawn as Windows' pane draws it.
fn is_text(content_type: &str, name: &str) -> bool {
    if content_type.starts_with("image/") {
        return false;
    }
    let by_type = TEXT_TYPES.contains(&content_type)
        || (!content_type.is_empty() && gio::content_type_is_a(content_type, "text/plain"));
    by_type || TEXT_EXTENSIONS.contains(&extension_of(name).as_str())
}

/// The text the Preview pane shows for `bytes`, the first bytes of a file
/// (`truncated` when the file goes on), or `None` for a binary file: one
/// with a NUL byte, or with more than one byte in [`BINARY_SHARE`] that is
/// no UTF-8 or a control character. A byte-order mark picks UTF-16 or is
/// dropped; a character cut by the limit is left out; a few invalid bytes
/// show as replacement characters.
pub(in crate::window) fn decode_text(bytes: &[u8], truncated: bool) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(b"\xff\xfe") {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(b"\xfe\xff") {
        utf16(rest, u16::from_be_bytes)
    } else {
        utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes), truncated)?
    };
    let controls = text.chars().filter(|&c| is_binary_control(c)).count();
    if text.contains('\0') || controls * BINARY_SHARE > text.chars().count().max(1) {
        return None;
    }
    Some(first_lines(&break_long_lines(&text)))
}

/// The first [`LINE_COUNT_LIMIT`] lines of `text`.
fn first_lines(text: &str) -> String {
    match text.match_indices('\n').nth(LINE_COUNT_LIMIT - 1) {
        Some((end, _)) => text[..=end].to_owned(),
        None => text.to_owned(),
    }
}

/// `bytes` as UTF-16 code units made by `unit`, with a dangling byte or
/// surrogate dropped and unpaired ones replaced.
fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// `bytes` as UTF-8, or `None` with too many invalid bytes or any NUL.
fn utf8(bytes: &[u8], truncated: bool) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let mut text = String::with_capacity(bytes.len());
    let mut invalid = 0_usize;
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        let broken = chunk.invalid();
        if broken.is_empty() {
            continue;
        }
        // The last character, cut by the limit, is no fault of the file.
        let is_end = std::ptr::eq(broken.as_ptr_range().end, bytes.as_ptr_range().end);
        if truncated && is_end && broken.len() < 4 {
            break;
        }
        invalid += broken.len();
        text.push(char::REPLACEMENT_CHARACTER);
    }
    (invalid * BINARY_SHARE <= bytes.len().max(1)).then_some(text)
}

/// Whether `c` is a control character no text file holds: not a tab, a
/// line break, a form feed or an escape (a log's colors).
fn is_binary_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0c' | '\x1b')
}

/// `text` with every line longer than [`LINE_LIMIT`] characters broken
/// there.
fn break_long_lines(text: &str) -> String {
    if !text.lines().any(|line| line.len() > LINE_LIMIT) {
        return text.to_owned();
    }
    let mut broken = String::with_capacity(text.len() + text.len() / LINE_LIMIT);
    let mut run = 0_usize;
    for c in text.chars() {
        if c == '\n' {
            run = 0;
        } else if run == LINE_LIMIT {
            broken.push('\n');
            run = 0;
        }
        if c != '\n' {
            run += 1;
        }
        broken.push(c);
    }
    broken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    const LOCAL: PreviewRules = PreviewRules {
        remote: false,
        remote_previews: false,
    };

    fn item(name: &str, content_type: &str) -> FileItem {
        let mut entry = file_entry(name);
        entry.content_type = Some(content_type.to_owned());
        FileItem::new(entry)
    }

    fn page_of(name: &str, content_type: &str) -> PreviewPage {
        preview_page(&[item(name, content_type)], LOCAL)
    }

    /// Text and source files show their text, pictures and PDFs their
    /// content, audio and video the player; anything else, a folder and
    /// several items say "No preview available.", and nothing selected
    /// "Select a file to preview.", as in Windows 11.
    ///
    /// parity: VIEW-070
    #[test]
    fn each_kind_of_selection_gets_windows_preview() {
        for (name, content_type) in [
            ("Notes.txt", "text/plain"),
            ("Read me.md", "text/markdown"),
            ("main.rs", "text/rust"),
            ("build.sh", "application/x-shellscript"),
            ("data.json", "application/json"),
            ("app.ts", "video/mp2t"),
            ("server.log", "application/octet-stream"),
        ] {
            assert!(
                matches!(page_of(name, content_type), PreviewPage::Text(_)),
                "{name}"
            );
        }
        assert!(matches!(
            page_of("Beach.jpg", "image/jpeg"),
            PreviewPage::Picture(_)
        ));
        assert!(matches!(
            page_of("Report.pdf", "application/pdf"),
            PreviewPage::Pdf(_)
        ));
        assert_eq!(page_of("Song.mp3", "audio/mpeg"), PreviewPage::Media);
        assert_eq!(page_of("Clip.mp4", "video/mp4"), PreviewPage::Media);
        assert_eq!(
            page_of("Bundle.zip", "application/zip"),
            PreviewPage::Message(NO_PREVIEW)
        );
        assert_eq!(
            page_of("setup.exe", "application/x-msdownload"),
            PreviewPage::Message(NO_PREVIEW)
        );
        let folder = FileItem::new(folder_entry("Projects"));
        assert_eq!(preview_page(&[folder], LOCAL), PreviewPage::Message(NO_PREVIEW));
        let two = [item("a.txt", "text/plain"), item("b.txt", "text/plain")];
        assert_eq!(preview_page(&two, LOCAL), PreviewPage::Message(NO_PREVIEW));
        assert_eq!(preview_page(&[], LOCAL), PreviewPage::Message(SELECT_A_FILE));
    }

    /// A share's file is read only while network previews are on, as its
    /// thumbnail is.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_network_file_follows_the_network_previews_setting() {
        let notes = [item("Notes.txt", "text/plain")];
        let off = PreviewRules {
            remote: true,
            remote_previews: false,
        };
        assert_eq!(preview_page(&notes, off), PreviewPage::Message(NO_PREVIEW));
        let on = PreviewRules {
            remote_previews: true,
            ..off
        };
        assert!(matches!(preview_page(&notes, on), PreviewPage::Text(_)));
    }

    /// parity: VIEW-070
    #[test]
    fn text_is_decoded_and_binary_refused() {
        assert_eq!(
            decode_text(b"hello\nworld\n", false).as_deref(),
            Some("hello\nworld\n")
        );
        assert_eq!(decode_text(b"\xef\xbb\xbfbom", false).as_deref(), Some("bom"));
        assert_eq!(decode_text(b"\xff\xfeh\0i\0", false).as_deref(), Some("hi"));
        assert_eq!(decode_text(b"\xfe\xff\0h\0i", false).as_deref(), Some("hi"));
        assert_eq!(decode_text(b"MZ\x90\0\x03\0\0\0", false), None, "a NUL byte");
        let mostly_invalid = [0xc3_u8, 0x28, 0xa0, 0xa1, 0xff, 0xfe, 0x80, 0x81];
        assert_eq!(decode_text(&mostly_invalid, false), None);
        let mut cafe = "café ".repeat(100).into_bytes();
        assert!(decode_text(&cafe, false).is_some());
        // Cut inside the last é: the half character is dropped quietly.
        cafe.truncate(cafe.len() - 2);
        let text = decode_text(&cafe, true).expect("text");
        assert!(text.ends_with("caf"), "{:?}", &text[text.len() - 8..]);
        assert!(!text.contains(char::REPLACEMENT_CHARACTER));
        let one_slip = [&b"x".repeat(200)[..], b"\xe9", &b"y".repeat(200)[..]].concat();
        assert!(decode_text(&one_slip, false).is_some_and(|t| t.contains('\u{fffd}')));
        let controls = "\x01\x02\x03\x04".repeat(10);
        assert_eq!(decode_text(controls.as_bytes(), false), None);
        assert!(decode_text(b"\x1b[31mred\x1b[0m\tok\r\n", false).is_some());
    }

    /// parity: VIEW-070
    #[test]
    fn a_very_long_line_is_broken_into_paragraphs() {
        let long = "a".repeat(LINE_LIMIT * 2 + 5);
        let text = decode_text(long.as_bytes(), false).expect("text");
        let lines: Vec<usize> = text.lines().map(str::len).collect();
        assert_eq!(lines, [LINE_LIMIT, LINE_LIMIT, 5]);
        assert_eq!(break_long_lines("short\nlines"), "short\nlines");
    }

    /// A file of many short lines shows its first ones only.
    ///
    /// parity: VIEW-070
    #[test]
    fn only_the_first_lines_are_shown() {
        let many = "x\n".repeat(LINE_COUNT_LIMIT + 10);
        let text = decode_text(many.as_bytes(), false).expect("text");
        assert_eq!(text.lines().count(), LINE_COUNT_LIMIT);
        assert!(text.ends_with('\n'));
        assert_eq!(first_lines("a\nb"), "a\nb");
    }

    /// With network previews off a file is checked for a network file
    /// system before it is read; with them on it is not.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_local_files_page_asks_for_its_file_system_with_network_previews_off() {
        let PreviewPage::Text(file) = page_of("Notes.txt", "text/plain") else {
            panic!("text");
        };
        assert!(file.local_only);
        let on = PreviewRules {
            remote: false,
            remote_previews: true,
        };
        let PreviewPage::Text(file) = preview_page(&[item("Notes.txt", "text/plain")], on) else {
            panic!("text");
        };
        assert!(!file.local_only);
    }
}
