// SPDX-License-Identifier: AGPL-3.0-only
//! Unpredictable names from the kernel's random source.
//!
//! Python names its private files and capabilities with `uuid.uuid4().hex`
//! and `secrets.token_hex`, which both read the kernel's random source, so
//! another program cannot guess a name and create or swap it first. Every
//! private name this crate creates takes its digits from here: the transfer
//! engine's staging and backup names, the ZIP extractor's staging and
//! preview folders, New from template's stage, tab-transfer capabilities,
//! the settings file's temporary files and private integration files.
//!
//! Identifiers that are only compared, never used as a name another
//! program could race for (the clipboard owner token, a search scan's
//! generation), use `GLib`'s UUIDs, which cannot fail.

use std::io::{self, Read};

/// The random bytes of a generated name: 128 bits, 32 hexadecimal digits,
/// the length of Python's `uuid.uuid4().hex`.
pub(crate) const NAME_BYTES: usize = 16;

/// `byte_count` random bytes from `/dev/urandom` as lowercase hexadecimal
/// digits, two per byte.
///
/// # Errors
///
/// When the kernel's random source cannot be read. Each caller explains
/// the failure in terms of the name it asked for.
pub(crate) fn random_hex(byte_count: usize) -> io::Result<String> {
    let mut bytes = vec![0u8; byte_count];
    let mut source = std::fs::File::open("/dev/urandom")?;
    source.read_exact(&mut bytes)?;
    let digits: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(digits.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_names_are_lowercase_hex_and_differ() {
        let first = random_hex(NAME_BYTES).expect("the kernel's random source");
        let second = random_hex(NAME_BYTES).expect("the kernel's random source");

        assert_eq!(first.len(), 32);
        assert!(first
            .bytes()
            .all(|digit| matches!(digit, b'0'..=b'9' | b'a'..=b'f')));
        assert_ne!(first, second);
    }
}
