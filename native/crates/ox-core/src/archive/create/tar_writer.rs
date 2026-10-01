// SPDX-License-Identifier: AGPL-3.0-only
//! Writing a `.tar.xz` (ARC-023): POSIX ustar headers, a GNU long-name
//! entry for paths over 100 bytes, sizes over 8 GiB in GNU's base-256
//! form, compressed with XZ as `tar -cJ` does. Files are stored with mode
//! 0644 and folders 0755, like the ZIP writer's fixed attributes; links
//! and special files never reach it.

use std::io::{Read, Write};

use lzma_rust2::{XzOptions, XzWriter};

use crate::archive::ArchiveError;
use crate::transfer::Cancellation;

/// The size of a header and of the blocks data is padded to.
const BLOCK: usize = 512;
/// The longest name a ustar header holds by itself.
const NAME_FIELD: usize = 100;
/// The XZ preset `xz` uses by default.
const XZ_PRESET: u32 = 6;
/// The size of the blocks file data is copied in.
const COPY_BYTES: usize = 64 * 1024;

/// Writes TAR entries into an XZ stream.
pub(super) struct TarXzWriter<W: Write> {
    output: XzWriter<W>,
}

impl<W: Write> TarXzWriter<W> {
    /// A writer compressing into `output`.
    ///
    /// # Errors
    ///
    /// The XZ encoder's error.
    pub(super) fn new(output: W) -> Result<Self, ArchiveError> {
        let output = XzWriter::new(output, XzOptions::with_preset(XZ_PRESET))?;
        Ok(Self { output })
    }

    /// Adds the folder `name` (ending with `/`) with the `rwx` bits
    /// `mode`.
    ///
    /// # Errors
    ///
    /// The output's error.
    pub(super) fn add_folder(&mut self, name: &str, modified: u64, mode: u32) -> Result<(), ArchiveError> {
        self.write_header(name, b'5', mode & 0o777, 0, modified)
    }

    /// Adds the file `name`, modified and with the `rwx` bits as
    /// `(modified, mode)` say, with the `size` bytes `content` holds.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::SourceChanged`] when `content` holds more or fewer
    /// bytes than `size`, [`ArchiveError::Cancelled`], or the error of
    /// reading or writing.
    pub(super) fn add_file(
        &mut self,
        name: &str,
        (modified, mode): (u64, u32),
        size: u64,
        content: &mut dyn Read,
        cancel: &Cancellation,
    ) -> Result<(), ArchiveError> {
        self.write_header(name, b'0', mode & 0o777, size, modified)?;
        let mut left = size;
        let mut block = vec![0u8; COPY_BYTES];
        while left > 0 {
            cancel.check()?;
            let wanted = usize::try_from(left.min(COPY_BYTES as u64)).unwrap_or(COPY_BYTES);
            let count = content.read(&mut block[..wanted])?;
            if count == 0 {
                return Err(ArchiveError::SourceChanged);
            }
            self.output.write_all(&block[..count])?;
            left -= count as u64;
        }
        // A file that grew since it was measured would be cut short.
        if content.read(&mut block[..1])? != 0 {
            return Err(ArchiveError::SourceChanged);
        }
        self.pad(size)
    }

    /// Writes the two empty blocks that end a TAR and finishes the XZ
    /// stream.
    ///
    /// # Errors
    ///
    /// The output's error.
    pub(super) fn finish(mut self) -> Result<W, ArchiveError> {
        self.output.write_all(&[0u8; 2 * BLOCK])?;
        Ok(self.output.finish()?)
    }

    /// Writes the header of `name`, preceded by a GNU long-name entry when
    /// the name does not fit.
    fn write_header(
        &mut self,
        name: &str,
        kind: u8,
        mode: u32,
        size: u64,
        modified: u64,
    ) -> Result<(), ArchiveError> {
        let bytes = name.as_bytes();
        if bytes.len() > NAME_FIELD {
            let long = [bytes, b"\0"].concat();
            let length = long.len() as u64;
            self.output
                .write_all(&header(b"././@LongLink", b'L', 0o644, length, 0))?;
            self.output.write_all(&long)?;
            self.pad(length)?;
        }
        let short = &bytes[..bytes.len().min(NAME_FIELD)];
        self.output
            .write_all(&header(short, kind, mode, size, modified))?;
        Ok(())
    }

    /// Pads data of `size` bytes to whole blocks.
    fn pad(&mut self, size: u64) -> Result<(), ArchiveError> {
        let used = usize::try_from(size % BLOCK as u64).unwrap_or(0);
        if used > 0 {
            self.output.write_all(&[0u8; BLOCK][used..])?;
        }
        Ok(())
    }
}

/// A ustar header.
fn header(name: &[u8], kind: u8, mode: u32, size: u64, modified: u64) -> [u8; BLOCK] {
    let mut block = [0u8; BLOCK];
    block[..name.len()].copy_from_slice(name);
    block[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
    block[108..116].copy_from_slice(b"0000000\0");
    block[116..124].copy_from_slice(b"0000000\0");
    write_number(&mut block[124..136], size);
    write_number(&mut block[136..148], modified);
    block[156] = kind;
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    block[148..156].copy_from_slice(b"        ");
    let sum: u32 = block.iter().map(|&byte| u32::from(byte)).sum();
    block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    block
}

/// `value` as 11 octal digits, or in base-256 when it does not fit.
fn write_number(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    if value < 1 << (3 * digits) {
        field.copy_from_slice(format!("{value:0digits$o}\0").as_bytes());
        return;
    }
    field.fill(0);
    let bytes = value.to_be_bytes();
    let start = field.len() - bytes.len();
    field[start..].copy_from_slice(&bytes);
    field[0] = 0x80;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_too_large_for_octal_use_base_256() {
        let mut field = [0u8; 12];
        write_number(&mut field, 0o777);
        assert_eq!(&field, b"00000000777\0");
        write_number(&mut field, 1 << 40);
        assert_eq!(field[0], 0x80);
        assert_eq!(field[6], 1);
    }
}
