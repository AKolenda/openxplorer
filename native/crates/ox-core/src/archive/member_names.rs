// SPDX-License-Identifier: AGPL-3.0-only
//! Which member names the archive browser shows and opens, and the folder
//! name an extraction suggests.
//!
//! Ports `safe_member` and `SAFE_VIEW_EXT` of `desktop/archives.py` and
//! `suggested_name` of `desktop/zip_extraction.py`.

use crate::location::{validate_name, LocationError};

/// Longest member name the browser shows, in characters.
const MAX_MEMBER_NAME_CHARS: usize = 4096;

/// ARC-006: extensions of the documents, images and media a member can be
/// opened as; scripts, executables and templates never are.
const PREVIEWABLE_EXTENSIONS: [&str; 30] = [
    ".txt", ".md", ".csv", ".json", ".html", ".htm", ".pdf", ".doc", ".docx", ".odt", ".xls", ".xlsx",
    ".ods", ".ppt", ".pptx", ".odp", ".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp", ".svg", ".mp4",
    ".mkv", ".webm", ".mp3", ".wav", ".ogg", ".flac",
];

/// The folder name suggested when nothing is left of the archive's name.
const FALLBACK_FOLDER_NAME: &str = "Extracted files";

/// ARC-004: true for a member path the browser may show and open.
///
/// Refused are empty and overlong names, control characters, backslashes,
/// absolute paths, `.`, `..` and empty segments, and a colon in the first
/// segment (a Windows drive such as `C:`). A folder's trailing slash is
/// allowed.
///
/// Python's `safe_member` raised an `IndexError` for the names `.` and `./`,
/// which made the whole listing fail; here they are simply unsafe.
pub(crate) fn is_safe_member(name: &str) -> bool {
    let has_forbidden_character =
        name.contains('\\') || name.chars().any(|character| character.is_ascii_control());
    let is_overlong = name.chars().count() > MAX_MEMBER_NAME_CHARS;
    if name.is_empty() || has_forbidden_character || is_overlong || name.starts_with('/') {
        return false;
    }
    let segments: Vec<&str> = name.trim_end_matches('/').split('/').collect();
    let has_relative_segment = segments.iter().any(|segment| matches!(*segment, "" | "." | ".."));
    !has_relative_segment && !segments[0].contains(':')
}

/// ARC-006: true when a file named `file_name` may be opened from an
/// archive, judged by its extension alone.
pub(crate) fn is_previewable(file_name: &str) -> bool {
    let Some(extension) = extension(file_name) else {
        return false;
    };
    PREVIEWABLE_EXTENSIONS.contains(&extension.to_lowercase().as_str())
}

/// The extension of `file_name` with its dot, like `PurePath.suffix`: none
/// for a leading dot only (`.bashrc`) or a trailing dot (`notes.`).
fn extension(file_name: &str) -> Option<&str> {
    let dot = file_name.rfind('.')?;
    let is_inner_dot = dot > 0 && dot + 1 < file_name.len();
    is_inner_dot.then(|| &file_name[dot..])
}

/// ARC-009: the new folder name an extraction suggests for an archive
/// named `archive_name`: the name without `.zip` (in any case) and without
/// trailing spaces or dots, or "Extracted files" when nothing is left.
///
/// # Errors
///
/// [`validate_name`]'s error when the rest is not a valid folder name.
pub fn suggested_folder_name(archive_name: &str) -> Result<String, LocationError> {
    let stem = strip_zip_extension(archive_name).trim_end_matches([' ', '.']);
    let name = if stem.is_empty() {
        FALLBACK_FOLDER_NAME
    } else {
        stem
    };
    validate_name(name).map(str::to_owned)
}

/// `archive_name` without a final `.zip` in any case.
fn strip_zip_extension(archive_name: &str) -> &str {
    let Some(start) = archive_name.len().checked_sub(".zip".len()) else {
        return archive_name;
    };
    match archive_name.get(start..) {
        Some(extension) if extension.eq_ignore_ascii_case(".zip") => &archive_name[..start],
        _ => archive_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `desktop/tests/test_v05.py::ZipTests::test_absolute_rejected`,
    /// `test_windows_path_rejected` and `test_backslash_rejected`.
    ///
    /// parity: ARC-004
    #[test]
    fn absolute_windows_and_backslash_paths_are_unsafe() {
        assert!(!is_safe_member("/tmp/x"));
        assert!(!is_safe_member("C:\\tmp\\x"));
        assert!(!is_safe_member("dir\\x"));
    }

    /// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_zip_rejects_ambiguous_and_control_names`.
    ///
    /// parity: ARC-004
    #[test]
    fn ambiguous_and_control_names_are_unsafe() {
        let unsafe_names = [
            "a//b.txt",
            "a/./b.txt",
            "a/../b.txt",
            "/root.txt",
            "a\nb.txt",
            "a\u{7f}b.txt",
            "C:/file",
            "../file",
        ];
        for name in unsafe_names {
            assert!(!is_safe_member(name), "{name:?}");
        }
        assert!(is_safe_member("Design files/draft.txt"));
    }

    /// parity: ARC-004
    #[test]
    fn folders_colons_after_the_first_segment_and_long_unicode_names_are_safe() {
        assert!(is_safe_member("Docs/"));
        assert!(is_safe_member("Docs//"));
        assert!(is_safe_member("notes/C:"));
        assert!(is_safe_member(&"é".repeat(4096)));
        assert!(!is_safe_member(&"a".repeat(4097)));
        assert!(!is_safe_member(""));
    }

    /// Python raised `IndexError` for these names and failed the listing.
    ///
    /// parity: ARC-004
    #[test]
    fn dot_names_are_unsafe_instead_of_failing() {
        for name in [".", "./", "././"] {
            assert!(!is_safe_member(name), "{name:?}");
        }
    }

    /// parity: ARC-006
    #[test]
    fn only_documents_images_and_media_are_previewable() {
        for name in ["notes.txt", "Report.PDF", "clip.webm", "..pdf", "a.tar.md"] {
            assert!(is_previewable(name), "{name}");
        }
        for name in ["run.sh", "setup.exe", "notes.", ".txt", "txt", "macro.docm"] {
            assert!(!is_previewable(name), "{name}");
        }
    }

    /// Ported from `desktop/tests/test_zip_extract.py::ZipExtractTests::test_suggested_names`.
    ///
    /// parity: ARC-009, ARC-012
    #[test]
    fn suggested_names_drop_the_zip_extension() {
        assert_eq!(suggested_folder_name("Assets.ZIP"), Ok("Assets".to_owned()));
        assert_eq!(suggested_folder_name(".zip"), Ok("Extracted files".to_owned()));
        assert_eq!(
            suggested_folder_name("Multi.part.zip"),
            Ok("Multi.part".to_owned())
        );
    }

    /// parity: ARC-009
    #[test]
    fn suggested_names_trim_trailing_dots_and_spaces_and_stay_valid() {
        assert_eq!(suggested_folder_name("Photos .. .zip"), Ok("Photos".to_owned()));
        assert_eq!(suggested_folder_name("Café.zip"), Ok("Café".to_owned()));
        assert_eq!(suggested_folder_name("archive.tar"), Ok("archive.tar".to_owned()));
        assert!(suggested_folder_name("a\u{1}b.zip").is_err());
    }
}
