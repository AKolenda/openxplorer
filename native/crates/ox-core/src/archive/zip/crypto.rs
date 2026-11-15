// SPDX-License-Identifier: AGPL-3.0-only
//! The traditional PKWARE encryption of ZIP members ("ZipCrypto"), section
//! 6.1 of the ZIP specification (APPNOTE.TXT), as Python's `zipfile`
//! (`_ZipDecrypter`) and Info-ZIP read it. Only reading is supported: the
//! app never creates password-protected archives.
//!
//! Each encrypted member starts with a 12-byte header. Its last byte,
//! once decrypted, must equal the check byte: the high byte of the CRC-32,
//! or of the modification time when the sizes follow the data (flag bit
//! 3). A wrong password fails that check 255 times in 256; the CRC-32 of
//! the data catches the rest.

use std::io::{self, Read};

/// The length of the encryption header in front of the data.
pub(super) const HEADER_LENGTH: u64 = 12;

/// The three keys of the cipher.
#[derive(Clone)]
pub(super) struct Keys([u32; 3]);

impl Keys {
    /// The keys after `password`.
    pub(super) fn new(password: &[u8]) -> Self {
        let mut keys = Self([0x1234_5678, 0x2345_6789, 0x3456_7890]);
        for &byte in password {
            keys.update(byte);
        }
        keys
    }

    /// Mixes the plain byte `byte` into the keys.
    fn update(&mut self, byte: u8) {
        let [first, second, third] = &mut self.0;
        *first = crc32_byte(*first, byte);
        *second = second
            .wrapping_add(*first & 0xff)
            .wrapping_mul(134_775_813)
            .wrapping_add(1);
        *third = crc32_byte(*third, (*second >> 24) as u8);
    }

    /// The next byte of the key stream.
    fn stream_byte(&self) -> u8 {
        let temp = (self.0[2] | 2) & 0xffff;
        (temp.wrapping_mul(temp ^ 1) >> 8) as u8
    }

    /// Decrypts `bytes` in place.
    pub(super) fn decrypt(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            let plain = *byte ^ self.stream_byte();
            self.update(plain);
            *byte = plain;
        }
    }
}

/// One step of the CRC-32 of the ZIP specification, without the final
/// inversion: the update the cipher is built on.
fn crc32_byte(crc: u32, byte: u8) -> u32 {
    let mut value = (crc ^ u32::from(byte)) & 0xff;
    for _ in 0..8 {
        value = if value & 1 == 0 {
            value >> 1
        } else {
            (value >> 1) ^ 0xedb8_8320
        };
    }
    (crc >> 8) ^ value
}

/// Decrypts the encryption header in `header` with `password` and says
/// whether its last byte is `check_byte`, and returns the keys to decrypt
/// the data that follows with.
pub(super) fn open_header(password: &[u8], header: &mut [u8; 12], check_byte: u8) -> Option<Keys> {
    let mut keys = Keys::new(password);
    keys.decrypt(header);
    (header[11] == check_byte).then_some(keys)
}

/// A reader that decrypts the encrypted bytes `inner` yields.
pub(super) struct Decrypting<R> {
    inner: R,
    keys: Keys,
}

impl<R> Decrypting<R> {
    /// Decrypts `inner` with `keys`, as [`open_header`] left them.
    pub(super) fn new(inner: R, keys: Keys) -> Self {
        Self { inner, keys }
    }
}

impl<R: Read> Read for Decrypting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.keys.decrypt(&mut buffer[..count]);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CRC step agrees with the full CRC-32 of a byte string.
    #[test]
    fn the_crc_step_is_the_zip_crc() {
        let data = b"Tidewater harbor notes";
        let crc = data.iter().fold(0xffff_ffff, |crc, &byte| crc32_byte(crc, byte));
        assert_eq!(!crc, crc32fast::hash(data));
    }

    /// Encrypting is decrypting's mirror; the header check finds the
    /// right password and refuses another.
    #[test]
    fn a_header_opens_with_its_password_only() {
        let mut keys = Keys::new(b"lighthouse");
        let mut header = [7u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0xa5];
        for byte in &mut header {
            let plain = *byte;
            *byte ^= keys.stream_byte();
            keys.update(plain);
        }

        let mut wrong = header;
        assert!(open_header(b"lighthouses", &mut wrong, 0xa5).is_none());
        let mut right = header;
        assert!(open_header(b"lighthouse", &mut right, 0xa5).is_some());
        assert_eq!(right[11], 0xa5);
    }
}
