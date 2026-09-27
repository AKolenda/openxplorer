// SPDX-License-Identifier: AGPL-3.0-only
//! Names the engine generates or accepts: private staging names, replacement
//! backups and validated child names. "Keep both" names come from
//! [`crate::location::try_new_copy_name`], shared with the rest of the app.
//!
//! Ports the `.winspace-transfer-` and `.winspace-replaced-` names and
//! `is_own_staging_name` of `desktop/operations.py`.
//!
//! Staging and backup names carry 128 random bits from the kernel, so another
//! program cannot predict (and pre-create or swap) them. A name the engine
//! did not create successfully is never deleted by it.

use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;

use super::error::TransferError;
use super::node::Node;

const STAGING_PREFIX: &str = ".winspace-transfer-";
const STAGING_SUFFIX: &str = ".part";
const BACKUP_PREFIX: &str = ".winspace-replaced-";
const BACKUP_SUFFIX: &str = ".backup";
/// The payload item inside a local or network staging folder.
pub(crate) const PAYLOAD_NAME: &str = "payload";

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
///
/// # Errors
///
/// When the kernel's random source cannot be read; nothing was changed.
pub fn staging_name() -> Result<String, TransferError> {
    Ok(format!("{STAGING_PREFIX}{}{STAGING_SUFFIX}", random_hex()?))
}

/// A new `.winspace-replaced-<32 hex>.backup` name that keeps the original
/// file during a reversible replacement.
///
/// # Errors
///
/// When the kernel's random source cannot be read; nothing was changed.
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

/// `directory.child(name)` after checking that `name` is exactly one path
/// component. GIO resolves `..` and `a/b` relative to the folder, so an
/// unchecked name could address an item outside the folder.
///
/// The check works on bytes, so names that are not valid UTF-8 pass through
/// unchanged. Backslashes are allowed: they are ordinary characters in POSIX
/// names.
pub(crate) fn child_node(
    directory: &dyn Node,
    name: impl AsRef<OsStr>,
) -> Result<Box<dyn Node>, TransferError> {
    let name = name.as_ref();
    if !is_single_component(name) {
        return Err(TransferError::failed("Invalid child name."));
    }
    Ok(directory.child(name))
}

/// True for a non-empty name that is not `.` or `..` and contains no `/` or
/// NUL byte.
fn is_single_component(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    let is_special = bytes.is_empty() || bytes == b"." || bytes == b"..";
    !is_special && !bytes.contains(&b'/') && !bytes.contains(&0)
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

    #[test]
    fn child_names_are_single_components_checked_on_bytes() {
        let latin1 = OsStr::from_bytes(b"caf\xe9.mp3");
        assert!(is_single_component(latin1));
        assert!(is_single_component(OsStr::new("back\\slash")));
        for bad in ["", ".", "..", "a/b", "/", "nul\0byte"] {
            assert!(!is_single_component(OsStr::new(bad)), "{bad:?}");
        }
    }
}
