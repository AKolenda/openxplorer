// SPDX-License-Identifier: AGPL-3.0-only
//! Pictures generated for tests: flat colours, with or without an EXIF
//! date taken. No photograph and no person is in them.

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};

/// A small picture of one flat colour, as PNG bytes.
pub(crate) fn png_bytes(colour: u32) -> Vec<u8> {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, 64, 48).expect("a picture");
    pixbuf.fill(colour << 8 | 0xff);
    pixbuf.save_to_bufferv("png", &[]).expect("PNG bytes")
}

/// A JPEG of one flat colour whose EXIF says it was taken at `taken`
/// (`2025:12:24 10:00:00`, as cameras write it), or carries no EXIF.
pub(crate) fn jpeg_bytes(colour: u32, taken: Option<&str>) -> Vec<u8> {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, 64, 48).expect("a picture");
    pixbuf.fill(colour << 8 | 0xff);
    let plain = pixbuf.save_to_bufferv("jpeg", &[]).expect("JPEG bytes");
    let Some(taken) = taken else {
        return plain;
    };
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(exif_segment(taken));
    bytes.extend_from_slice(&plain[2..]);
    bytes
}

/// An APP1 segment with TIFF IFD0 pointing at an EXIF IFD that holds
/// only `DateTimeOriginal`.
fn exif_segment(taken: &str) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8_u32.to_le_bytes());
    // IFD0 at 8: one entry, ExifIFDPointer, then no next IFD.
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    tiff.extend_from_slice(&0x8769_u16.to_le_bytes());
    tiff.extend_from_slice(&4_u16.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&26_u32.to_le_bytes());
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    // EXIF IFD at 26: DateTimeOriginal, 20 ASCII bytes at 44.
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    tiff.extend_from_slice(&0x9003_u16.to_le_bytes());
    tiff.extend_from_slice(&2_u16.to_le_bytes());
    tiff.extend_from_slice(&20_u32.to_le_bytes());
    tiff.extend_from_slice(&44_u32.to_le_bytes());
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    let mut date = taken.as_bytes().to_vec();
    date.resize(19, b' ');
    date.push(0);
    tiff.extend(date);
    let mut segment = vec![0xff, 0xe1];
    let length = u16::try_from(2 + 6 + tiff.len()).expect("a short segment");
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend(tiff);
    segment
}
