// SPDX-License-Identifier: AGPL-3.0-only
//! Searching inside files as well as their names.
//!
//! New in the native app, from Dolphin's search box, which searches either
//! file names or contents (`SearchSettings::What`, SRCH-036), and Windows
//! File Explorer's "File contents" search option. Neither the Python app
//! nor its cache reads file contents, so a contents search always walks
//! the folder live (see [`super::live`]). Only local regular files are
//! read, up to [`MAX_READ_BYTES`], and only when they look like text: a
//! file with a NUL byte near its start is binary and is matched by name
//! only. Words are matched ignoring case, as in names; a wildcard word
//! still has to match the whole name.

use std::fs::File;
use std::io::Read;

use gio::prelude::*;
use serde::{Deserialize, Serialize};

use crate::entry::{Entry, EntryKind};

/// Largest file whose text is searched; larger files match by name only.
pub const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;

/// How much of a file's start is checked for a NUL byte.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// What a search matches its words against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchIn {
    /// File and folder names, as the cache and the filter do.
    #[default]
    Names,
    /// Names, and the text of the files.
    NamesAndContents,
}

/// The lower-cased text of `entry`, when it is a local regular file of at
/// most [`MAX_READ_BYTES`] that looks like text; `None` otherwise, or when
/// it cannot be read.
pub(super) fn lowercase_text(entry: &Entry) -> Option<String> {
    let is_readable = entry.kind == EntryKind::File && !entry.is_virtual;
    if !is_readable || entry.size.is_some_and(|size| size > MAX_READ_BYTES) {
        return None;
    }
    if !entry.uri.starts_with("file://") {
        return None;
    }
    let path = gio::File::for_uri(&entry.uri).path()?;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_READ_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let start = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if start.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).to_lowercase())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::entry::inspect;

    fn inspect_path(path: &Path) -> Result<Entry, crate::entry::EntryError> {
        inspect(&gio::File::for_path(path).uri(), None)
    }

    #[test]
    fn text_files_are_read_and_binary_files_are_not() {
        let folder = tempfile::tempdir().unwrap();
        let notes = folder.path().join("notes.txt");
        let image = folder.path().join("photo.bin");
        fs::write(&notes, "Quarterly BUDGET review").unwrap();
        fs::write(&image, b"\x89PNG\0\0budget").unwrap();

        let text = lowercase_text(&inspect_path(&notes).unwrap());
        let binary = lowercase_text(&inspect_path(&image).unwrap());
        let folder_text = lowercase_text(&inspect_path(folder.path()).unwrap());

        assert_eq!(text.as_deref(), Some("quarterly budget review"));
        assert_eq!(binary, None);
        assert_eq!(folder_text, None);
    }
}
