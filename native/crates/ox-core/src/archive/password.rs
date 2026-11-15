// SPDX-License-Identifier: AGPL-3.0-only
//! The password of a password-protected ZIP, as the user typed it.
//!
//! It stays in memory only: it is never written to disk, logged or shown,
//! its `Debug` form is redacted, and its bytes are overwritten when it is
//! dropped.

use std::fmt;

/// A ZIP password, encoded as UTF-8 the way Info-ZIP and Python's
/// `zipfile` encode a password typed on Linux.
#[derive(Clone, PartialEq, Eq)]
pub struct ArchivePassword(Vec<u8>);

impl ArchivePassword {
    /// The password's bytes, for the decryption only.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<&str> for ArchivePassword {
    fn from(text: &str) -> Self {
        Self(text.as_bytes().to_vec())
    }
}

impl fmt::Debug for ArchivePassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ArchivePassword(<redacted>)")
    }
}

impl Drop for ArchivePassword {
    fn drop(&mut self) {
        self.0.fill(0);
        // Keeps the overwrite from being optimised away before the memory
        // is freed.
        std::hint::black_box(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_shows_in_debug_output() {
        let password = ArchivePassword::from("lighthouse");
        assert_eq!(format!("{password:?}"), "ArchivePassword(<redacted>)");
        assert_eq!(password.bytes(), b"lighthouse");
    }
}
