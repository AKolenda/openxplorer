// SPDX-License-Identifier: AGPL-3.0-only
//! Names the engine generates or accepts: private staging names, replacement
//! backups, Windows-style "Keep both" names and validated child names.
//!
//! Staging and backup names carry 128 random bits from the kernel, so another
//! program cannot predict (and pre-create or swap) them. A name the engine
//! did not create successfully is never deleted by it.

use std::io::Read;

use super::node::{Node, TransferError};

const STAGING_PREFIX: &str = ".winspace-transfer-";
const STAGING_SUFFIX: &str = ".part";
const BACKUP_PREFIX: &str = ".winspace-replaced-";
const BACKUP_SUFFIX: &str = ".backup";
/// The payload item inside a local or network staging folder.
pub(crate) const PAYLOAD_NAME: &str = "payload";
/// Linux `NAME_MAX` in bytes.
const NAME_MAX: usize = 255;

/// 32 lowercase hexadecimal digits from `/dev/urandom` (like
/// `uuid.uuid4().hex` in Python: unpredictable, not merely unique).
fn random_hex() -> Result<String, TransferError> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| {
            TransferError::failed(format!(
                "Could not reserve a private staging name. Nothing was changed. {error}"
            ))
        })?;
    let digits: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(digits.concat())
}

/// A new `.winspace-transfer-<32 hex>.part` name for a private staging item.
pub fn staging_name() -> Result<String, TransferError> {
    Ok(format!("{STAGING_PREFIX}{}{STAGING_SUFFIX}", random_hex()?))
}

/// A new `.winspace-replaced-<32 hex>.backup` name that keeps the original
/// file during a reversible replacement.
pub fn backup_name() -> Result<String, TransferError> {
    Ok(format!("{BACKUP_PREFIX}{}{BACKUP_SUFFIX}", random_hex()?))
}

/// True only for names exactly of the form this engine generates for
/// staging: `.winspace-transfer-` followed by 32 lowercase hex digits and
/// `.part`.
pub fn is_own_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(STAGING_PREFIX) else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(STAGING_SUFFIX) else {
        return false;
    };
    let lower_hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    digits.len() == 32 && digits.chars().all(lower_hex)
}

/// The Windows-style duplicate name used by "Keep both":
/// `file (copy 2).pdf`, `.env (copy 2)`, `Folder.v1 (copy 3)`.
///
/// Port of `new_copy_name` in `desktop/core.py`. Folders and names whose
/// only dot is leading keep no extension. The stem is shortened (whole
/// characters, never splitting UTF-8) to respect `NAME_MAX`; a name whose
/// extension alone is too long is refused rather than renamed beyond
/// recognition.
pub fn new_copy_name(name: &str, number: u32, is_directory: bool) -> Result<String, TransferError> {
    crate::location::validate_name(name).map_err(|error| TransferError::failed(error.to_string()))?;
    let has_extension = name.trim_start_matches('.').contains('.');
    let (stem, suffix) = match name.rfind('.') {
        Some(dot) if !is_directory && has_extension => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let marker = format!(" (copy {number})");
    let mut stem = stem.to_string();
    while stem.len() + marker.len() + suffix.len() > NAME_MAX && !stem.is_empty() {
        stem.pop();
    }
    if stem.is_empty() {
        return Err(TransferError::failed(
            "This file name is too long to generate a duplicate name.",
        ));
    }
    Ok(format!("{stem}{marker}{suffix}"))
}

/// `dir.child(name)` after checking that `name` is exactly one path
/// component. GIO resolves `..` and `a/b` relative to the folder, so an
/// unchecked name could address an item outside the folder. Backslashes are
/// allowed: they are ordinary characters in POSIX names.
pub(crate) fn child_node(directory: &dyn Node, name: &str) -> Result<Box<dyn Node>, TransferError> {
    let invalid = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']);
    if invalid {
        return Err(TransferError::failed("Invalid child name."));
    }
    Ok(directory.child(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_staging_names_are_recognised() {
        let first = staging_name().expect("urandom is readable");
        let second = staging_name().expect("urandom is readable");
        assert!(is_own_staging_name(&first));
        assert_ne!(first, second);
        let backup = backup_name().expect("urandom is readable");
        assert!(backup.starts_with(".winspace-replaced-") && backup.ends_with(".backup"));
        assert!(!is_own_staging_name(&backup));
    }

    /// Port of `StagingNameTests` in `desktop/tests/test_device_staging.py`.
    #[test]
    fn only_exact_generated_names_match() {
        let good = format!(".winspace-transfer-{}.part", "a".repeat(32));
        assert!(is_own_staging_name(&good));
        let bad = [
            format!(".winspace-transfer-{}.part", "A".repeat(32)),
            format!(".winspace-transfer-{}.part", "a".repeat(31)),
            format!("x.winspace-transfer-{}.part", "a".repeat(32)),
            format!(".winspace-transfer-{}.part/x", "a".repeat(32)),
            format!(".winspace-replaced-{}.backup", "a".repeat(32)),
            "payload".to_string(),
            String::new(),
        ];
        for name in bad {
            assert!(!is_own_staging_name(&name), "{name}");
        }
    }

    /// Port of `test_validation` (copy names) in `desktop/tests/test_core.py`.
    #[test]
    fn copy_names_match_the_python_app() {
        let name = |n: &str, number, dir| new_copy_name(n, number, dir).expect("valid name");
        assert_eq!(name("file.pdf", 2, false), "file (copy 2).pdf");
        assert_eq!(name(".env", 2, false), ".env (copy 2)");
        assert_eq!(name("Folder.v1", 3, true), "Folder.v1 (copy 3)");
        assert_eq!(name("archive.tar.gz", 2, false), "archive.tar (copy 2).gz");
        assert_eq!(name("a.", 2, false), "a (copy 2).");
        let long = format!("{}.txt", "é".repeat(120));
        assert!(name(&long, 2, false).len() <= 255);
        assert!(name(&long, 2, false).ends_with(" (copy 2).txt"));
    }

    #[test]
    fn copy_name_with_an_oversized_extension_is_refused() {
        let name = format!("a.{}", "x".repeat(250));
        assert!(new_copy_name(&name, 2, false).is_err());
        assert!(new_copy_name("..", 2, false).is_err());
    }
}
