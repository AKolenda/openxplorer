// SPDX-License-Identifier: AGPL-3.0-only
//! Checks the Zstandard history window before the decoder may retain it.
//!
//! The window is a frame-header field, independent of TAR member sizes.
//! A stream with a large window can make a streaming read retain that
//! much output before yielding its first byte, so extraction's output
//! limits cannot protect this allocation.

use std::io::{self, Read};

use crate::archive::MAX_DECODER_MEMORY_KIB;

/// Checks one frame's window and returns the header bytes already read.
/// The decoder must replay these bytes instead of rereading a source that
/// could change in between. Other magic values, including skippable frames,
/// are left to the decoder. No compressed block is read here.
pub(super) fn checked_header(source: &mut (impl Read + ?Sized)) -> io::Result<Vec<u8>> {
    let mut header = RecordedHeader {
        source,
        bytes: Vec::with_capacity(18),
    };
    let window = read_window(&mut header)?;
    if window > u64::from(MAX_DECODER_MEMORY_KIB) * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "Zstandard window exceeds the archive decoder memory limit",
        ));
    }
    Ok(header.bytes)
}

/// Retains the at most 18 bytes the window parser consumes for replay.
struct RecordedHeader<'a, R: ?Sized> {
    source: &'a mut R,
    bytes: Vec<u8>,
}

impl<R: Read + ?Sized> Read for RecordedHeader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.source.read(buffer)?;
        self.bytes.extend_from_slice(&buffer[..count]);
        Ok(count)
    }
}

/// The frame's history window. In a single-segment frame it is the
/// content size; otherwise it is computed from the window descriptor.
fn read_window(source: &mut (impl Read + ?Sized)) -> io::Result<u64> {
    let mut magic = [0; 4];
    source.read_exact(&mut magic)?;
    if magic != [0x28, 0xb5, 0x2f, 0xfd] {
        return Ok(0);
    }
    let mut descriptor = [0];
    source.read_exact(&mut descriptor)?;
    let descriptor = descriptor[0];
    if descriptor & 0x20 == 0 {
        let mut window = [0];
        source.read_exact(&mut window)?;
        let base = 1u64 << (10 + (window[0] >> 3));
        return Ok(base + (base / 8) * u64::from(window[0] & 7));
    }
    let dictionary_bytes = [0, 1, 2, 4][usize::from(descriptor & 3)];
    let mut dictionary = [0; 4];
    source.read_exact(&mut dictionary[..dictionary_bytes])?;
    let size_bytes = [1, 2, 4, 8][usize::from(descriptor >> 6)];
    let mut size = [0; 8];
    source.read_exact(&mut size[..size_bytes])?;
    let size = u64::from_le_bytes(size);
    // Two-byte content sizes store their value minus 256.
    Ok(if size_bytes == 2 { size + 256 } else { size })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn window_descriptor_boundary_is_checked_without_reading_blocks() {
        // Exponent 17 describes a 128 MiB window; a nonzero mantissa
        // increases it beyond the budget. No compressed block is supplied.
        for (descriptor, allowed) in [(136, true), (137, false), (255, false)] {
            let mut source = Cursor::new([0x28, 0xb5, 0x2f, 0xfd, 0, descriptor]);
            let result = checked_header(&mut source);
            assert_eq!(result.is_ok(), allowed);
            if !allowed {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
            }
            assert_eq!(source.position(), 6);
        }
    }

    #[test]
    fn single_segment_sizes_and_dictionary_fields_are_read_without_allocating() {
        for (flag, size_bytes, size) in [(0, 1, 42u64), (1, 2, 513), (2, 4, 1024), (3, 8, 2048)] {
            for (dictionary_flag, dictionary_bytes) in [(0, 0), (1, 1), (2, 2), (3, 4)] {
                let mut header = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20 | flag << 6 | dictionary_flag];
                header.extend(vec![0; dictionary_bytes]);
                let encoded = if size_bytes == 2 { size - 256 } else { size };
                header.extend_from_slice(&encoded.to_le_bytes()[..size_bytes]);
                assert_eq!(read_window(&mut header.as_slice()).unwrap(), size);
            }
        }
        for size in [128 * 1024 * 1024u64, 128 * 1024 * 1024 + 1, u64::MAX] {
            let mut header = vec![0x28, 0xb5, 0x2f, 0xfd, 0xe0];
            header.extend_from_slice(&size.to_le_bytes());
            let mut source = Cursor::new(header);
            assert_eq!(checked_header(&mut source).is_ok(), size <= 128 * 1024 * 1024);
            assert_eq!(source.position(), 13);
        }
    }

    #[test]
    fn skippable_frames_are_left_for_the_decoder() {
        let mut source = Cursor::new([0x50, 0x2a, 0x4d, 0x18]);
        assert_eq!(checked_header(&mut source).unwrap(), [0x50, 0x2a, 0x4d, 0x18]);
        assert_eq!(source.position(), 4);
    }

    #[test]
    fn replay_uses_the_checked_header_even_if_the_source_changes() {
        let original = [0x28, 0xb5, 0x2f, 0xfd, 0, 0, 42];
        let mut source = Cursor::new(original);
        let header = checked_header(&mut source).unwrap();
        // The source changes after inspection. The decoder receives the
        // inspected prefix, with the unread bytes from the current source.
        source.get_mut()[5] = 255;
        let mut replayed = Vec::new();
        Cursor::new(header)
            .chain(source)
            .read_to_end(&mut replayed)
            .unwrap();
        assert_eq!(replayed, original);
    }
}
