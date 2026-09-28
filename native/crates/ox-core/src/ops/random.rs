// SPDX-License-Identifier: AGPL-3.0-only
//! Unpredictable names from the kernel's random source: the private
//! `.winspace-new-<hex>` stage of New from template and the tab-transfer
//! capability. The same source and form as the transfer engine's staging
//! names; Python uses `uuid.uuid4().hex` and `secrets.token_hex`.

use std::io::{self, Read};

/// `byte_count` random bytes from `/dev/urandom` as lowercase hexadecimal
/// digits, two per byte.
///
/// # Errors
///
/// When the kernel's random source cannot be read.
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
        let first = random_hex(16).expect("the kernel's random source");
        let second = random_hex(16).expect("the kernel's random source");

        assert_eq!(first.len(), 32);
        assert!(first
            .bytes()
            .all(|digit| matches!(digit, b'0'..=b'9' | b'a'..=b'f')));
        assert_ne!(first, second);
    }
}
