// SPDX-License-Identifier: AGPL-3.0-only
//! File names, "Keep both" names and sidebar labels.
//!
//! Ports `validate_name`, `new_copy_name` and `safe_label` from
//! `desktop/core.py`.

use super::text::{has_control_character, python_strip};
use super::LocationError;

/// Longest file name most Linux filesystems accept (`NAME_MAX`), in bytes.
const NAME_MAX_BYTES: usize = 255;

/// Longest sidebar label, in characters.
pub const MAX_LABEL_CHARS: usize = 120;

/// Whether a name belongs to a file or a folder. Only file names have an
/// extension, so a folder `Folder.v1` keeps its whole name as the stem of
/// a duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A file: the copy marker goes before the last extension.
    File,
    /// A folder: the copy marker goes at the end.
    Folder,
}

/// Validates a single file or folder name: not empty, `.` or `..`, no
/// slash, backslash or control character, and at most 255 UTF-8 bytes.
///
/// # Errors
///
/// A [`LocationError`] with the Python app's message when any of those
/// rules is broken.
pub fn validate_name(name: &str) -> Result<&str, LocationError> {
    // Safety rule (`core.py::validate_name`): a name is exactly one path
    // component, so creating or renaming an item never reaches outside its
    // folder.
    if name.is_empty() || name == "." || name == ".." {
        return Err(LocationError::new(
            "Enter a non-empty file name, not “.” or “..”.",
        ));
    }
    if name.contains(['/', '\\']) || has_control_character(name) {
        return Err(LocationError::new(
            "A name cannot contain slashes or control characters.",
        ));
    }
    if name.len() > NAME_MAX_BYTES {
        return Err(LocationError::new("This name is longer than 255 bytes."));
    }
    Ok(name)
}

/// The name for the `number`th duplicate when both copies are kept:
/// `file (copy 2).pdf`, `.env (copy 2)`, `Folder.v1 (copy 3)`.
///
/// Folders and names whose only dot is leading keep the marker at the end;
/// files put it before the last extension. The stem is shortened a whole
/// character at a time to stay within 255 bytes.
///
/// # Errors
///
/// The [`validate_name`] error for an invalid name, and "This file name is
/// too long to generate a duplicate name." when the extension alone leaves
/// no room for the marker: the name is then rejected rather than renamed
/// beyond recognition.
pub fn try_new_copy_name(name: &str, number: u32, kind: ItemKind) -> Result<String, LocationError> {
    validate_name(name)?;
    let (mut stem, suffix) = split_extension(name, kind);
    let marker = format!(" (copy {number})");
    let fits = |stem: &str| stem.len() + marker.len() + suffix.len() <= NAME_MAX_BYTES;
    while !fits(stem) && !stem.is_empty() {
        stem = without_last_char(stem);
    }
    if stem.is_empty() {
        return Err(LocationError::new(
            "This file name is too long to generate a duplicate name.",
        ));
    }
    Ok(format!("{stem}{marker}{suffix}"))
}

/// Infallible form of [`try_new_copy_name`] for callers that let the
/// filesystem reject the result.
///
/// When no valid duplicate name exists it returns the unshortened
/// `stem (copy N).ext`, which is either invalid or longer than 255 bytes,
/// so creating it fails with an error instead of writing a surprising name.
/// Prefer [`try_new_copy_name`] in new code.
pub fn new_copy_name(name: &str, number: u32, kind: ItemKind) -> String {
    try_new_copy_name(name, number, kind).unwrap_or_else(|_| {
        let (stem, suffix) = split_extension(name, kind);
        format!("{stem} (copy {number}){suffix}")
    })
}

/// A user-supplied sidebar label, trimmed, or `fallback` when it is empty.
///
/// # Errors
///
/// A [`LocationError`] for a label over 120 characters or with a control
/// character.
pub fn safe_label(value: &str, fallback: &str) -> Result<String, LocationError> {
    let value = python_strip(value);
    if value.is_empty() {
        return Ok(fallback.to_string());
    }
    if has_control_character(value) || value.chars().count() > MAX_LABEL_CHARS {
        return Err(LocationError::new(
            "A sidebar label must be at most 120 characters and contain no control characters.",
        ));
    }
    Ok(value.to_string())
}

/// Splits `report.final.pdf` into `report.final` and `.pdf`. Folders and
/// names without a dot after their leading dots (`.env`, `..x`) have no
/// extension.
fn split_extension(name: &str, kind: ItemKind) -> (&str, &str) {
    let has_extension = kind == ItemKind::File && name.trim_start_matches('.').contains('.');
    match name.rfind('.') {
        Some(dot) if has_extension => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `text` without its last character; never splits a UTF-8 sequence.
fn without_last_char(text: &str) -> &str {
    let mut chars = text.chars();
    chars.next_back();
    chars.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-006
    #[test]
    fn names_are_validated_like_python() {
        assert_eq!(validate_name("Résumé 2026.txt"), Ok("Résumé 2026.txt"));
        let too_long = "é".repeat(200);
        for bad in [
            "",
            ".",
            "..",
            "../bad",
            "a/b",
            "a\\b",
            "x\0",
            "x\n",
            "del\u{7f}",
            too_long.as_str(),
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        // 255 bytes is allowed; C1 controls are not in the Python pattern.
        assert!(validate_name(&"a".repeat(255)).is_ok());
        assert!(validate_name("x\u{85}").is_ok());
    }

    /// parity: XFER-008
    #[test]
    fn copy_names_follow_python() {
        assert_eq!(new_copy_name("file.pdf", 2, ItemKind::File), "file (copy 2).pdf");
        assert_eq!(new_copy_name(".env", 2, ItemKind::File), ".env (copy 2)");
        assert_eq!(
            new_copy_name("Folder.v1", 3, ItemKind::Folder),
            "Folder.v1 (copy 3)"
        );
        assert_eq!(
            new_copy_name("archive.tar.gz", 2, ItemKind::File),
            "archive.tar (copy 2).gz"
        );
        assert_eq!(
            new_copy_name("..hidden.txt", 4, ItemKind::File),
            "..hidden (copy 4).txt"
        );
        assert_eq!(
            new_copy_name("trailing.", 2, ItemKind::File),
            "trailing (copy 2)."
        );
        assert_eq!(new_copy_name("README", 10, ItemKind::File), "README (copy 10)");
    }

    /// parity: XFER-008
    #[test]
    fn long_copy_names_are_shortened_by_whole_characters() {
        // The Python suite's case: 244 bytes plus the marker still fits.
        let name = format!("{}.txt", "é".repeat(120));
        let copy = try_new_copy_name(&name, 2, ItemKind::File).expect("the name fits");
        assert!(copy.len() <= 255, "{} bytes", copy.len());
        // 254 bytes: four two-byte characters must go.
        let name = format!("{}.txt", "é".repeat(125));
        let copy = try_new_copy_name(&name, 2, ItemKind::File).expect("the stem can be shortened");
        assert_eq!(copy, format!("{} (copy 2).txt", "é".repeat(121)));
        assert_eq!(copy.len(), 255);
    }

    /// parity: XFER-008
    #[test]
    fn impossible_copy_names_are_rejected() {
        let long_extension = format!("a.{}", "x".repeat(250));
        assert!(try_new_copy_name(&long_extension, 2, ItemKind::File).is_err());
        assert!(try_new_copy_name("a/b", 2, ItemKind::File).is_err());
        // The infallible form yields a name the filesystem will refuse.
        assert!(new_copy_name(&long_extension, 2, ItemKind::File).len() > 255);
    }

    /// parity: SAFE-018
    #[test]
    fn labels_are_trimmed_and_bounded() {
        assert_eq!(
            safe_label("  Projects (Z:) ", "Folder").as_deref(),
            Ok("Projects (Z:)")
        );
        assert_eq!(safe_label(" \t", "Folder").as_deref(), Ok("Folder"));
        assert_eq!(
            safe_label(&"é".repeat(120), "Folder").map(|label| label.chars().count()),
            Ok(120)
        );
        assert!(safe_label(&"é".repeat(121), "Folder").is_err());
        assert!(safe_label("a\u{1}b", "Folder").is_err());
    }
}
