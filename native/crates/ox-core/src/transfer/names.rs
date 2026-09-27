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
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;

use super::error::TransferError;
use super::node::Node;

/// Staging names are `.winspace-transfer-<32 hex>.part` (XFER-001).
const STAGING_PREFIX: &str = ".winspace-transfer-";
const STAGING_SUFFIX: &str = ".part";
/// Backups of replaced files are `.winspace-replaced-<32 hex>.backup`
/// (XFER-010).
const BACKUP_PREFIX: &str = ".winspace-replaced-";
const BACKUP_SUFFIX: &str = ".backup";
/// The number of hexadecimal digits in a generated name: 128 random bits.
const RANDOM_DIGITS: usize = 32;
/// The payload item inside a local or network staging folder.
pub(crate) const PAYLOAD_NAME: &str = "payload";

/// 32 lowercase hexadecimal digits from `/dev/urandom` (like
/// `uuid.uuid4().hex` in Python: unpredictable, not merely unique).
///
/// # Errors
///
/// When the kernel's random source cannot be read. Each caller explains
/// the failure in terms of the name it asked for.
fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; RANDOM_DIGITS / 2];
    let mut source = std::fs::File::open("/dev/urandom")?;
    source.read_exact(&mut bytes)?;
    let pairs: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(pairs.concat())
}

/// A new `.winspace-transfer-<32 hex>.part` name for a private staging item.
///
/// # Errors
///
/// When the kernel's random source cannot be read. A copy asks for its
/// staging name before it creates anything, so nothing was changed.
pub(crate) fn staging_name() -> Result<String, TransferError> {
    let digits = random_hex().map_err(|error| staging_name_failure(&error))?;
    Ok(format!("{STAGING_PREFIX}{digits}{STAGING_SUFFIX}"))
}

/// A new `.winspace-replaced-<32 hex>.backup` name that keeps the original
/// file during a reversible replacement.
///
/// # Errors
///
/// When the kernel's random source cannot be read.
pub(crate) fn backup_name() -> Result<String, TransferError> {
    let digits = random_hex().map_err(|error| backup_name_failure(&error))?;
    Ok(format!("{BACKUP_PREFIX}{digits}{BACKUP_SUFFIX}"))
}

/// The error when no staging name could be generated. The copy has not
/// created anything yet, so the message can say that nothing changed.
fn staging_name_failure(error: &io::Error) -> TransferError {
    TransferError::failed(format!(
        "Could not reserve a private staging name. Nothing was changed. {error}"
    ))
}

/// The error when no backup name could be generated, worded like
/// `_replace_via_backup` in `desktop/operations.py`. It says nothing about
/// earlier changes: during a folder merge, other items may already have
/// been replaced.
fn backup_name_failure(error: &io::Error) -> TransferError {
    TransferError::failed(format!("Could not reserve a temporary replacement name. {error}"))
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
    let is_lower_hex = |digit: u8| matches!(digit, b'0'..=b'9' | b'a'..=b'f');
    digits.len() == RANDOM_DIGITS && digits.bytes().all(is_lower_hex)
}

/// `folder.child(name)` after checking that `name` is exactly one path
/// component. GIO resolves `..` and `a/b` relative to the folder, so an
/// unchecked name could address an item outside the folder.
///
/// The check works on bytes, so names that are not valid UTF-8 pass through
/// unchanged. Backslashes are allowed: they are ordinary characters in POSIX
/// names.
///
/// # Errors
///
/// A name that is empty, `.`, `..`, or contains `/` or a NUL byte.
pub(crate) fn child_node(folder: &dyn Node, name: impl AsRef<OsStr>) -> Result<Box<dyn Node>, TransferError> {
    let name = name.as_ref();
    if !is_single_component(name) {
        return Err(TransferError::failed("Invalid child name."));
    }
    Ok(folder.child(name))
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

    /// parity: XFER-001
    #[test]
    fn generated_staging_names_are_unpredictable_and_recognised() {
        let first = staging_name().expect("urandom is readable");
        let second = staging_name().expect("urandom is readable");

        assert!(is_own_staging_name(&first), "{first}");
        assert_ne!(first, second);
    }

    /// parity: XFER-002, XFER-010
    #[test]
    fn backup_names_are_never_taken_for_staging() {
        let backup = backup_name().expect("urandom is readable");

        assert!(backup.starts_with(".winspace-replaced-") && backup.ends_with(".backup"));
        assert_eq!(backup.len(), ".winspace-replaced-".len() + 32 + ".backup".len());
        assert!(!is_own_staging_name(&backup));
    }

    /// Port of `StagingNameTests.test_only_exact_generated_names_match` in
    /// `desktop/tests/test_device_staging.py`.
    ///
    /// parity: XFER-002
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

    /// parity: XFER-010
    #[test]
    fn a_backup_name_failure_names_the_replacement_not_the_staging() {
        let unreadable = io::Error::other("no random source");

        let backup = backup_name_failure(&unreadable).to_string();
        let staging = staging_name_failure(&unreadable).to_string();

        assert_eq!(
            backup,
            "Could not reserve a temporary replacement name. no random source"
        );
        assert!(!backup.contains("Nothing was changed"), "{backup}");
        assert_eq!(
            staging,
            "Could not reserve a private staging name. Nothing was changed. no random source"
        );
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
