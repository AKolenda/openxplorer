// SPDX-License-Identifier: AGPL-3.0-only
//! Rotate left and Rotate right (HOME-032), as Windows 11 File Explorer
//! offers them for photos: a quarter turn of the file itself.
//!
//! A JPEG is turned without touching its pixels: only its EXIF
//! orientation changes, which every viewer (and the app's own previews)
//! applies, so nothing is lost. The orientation is changed in place when
//! the JPEG has one; else IFD0 is copied, with an orientation added, to
//! the end of the EXIF block and the header pointed at the copy, so no
//! value moves and every offset in the block stays right; a JPEG without
//! EXIF gets a small EXIF block that holds only the orientation. A PNG
//! has no orientation viewers agree on, so its pixels are turned with
//! gdk-pixbuf and saved as PNG again, which is lossless too (its text
//! and colour chunks are not kept). Other types are not turned.
//!
//! The new file is written beside the photo and renamed over it, keeping
//! its permissions, so a failure leaves the photo as it was.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Why a photo was not turned.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RotateError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Not a JPEG or PNG, or damaged.
    #[error("This picture can't be rotated.")]
    Unsupported,
}

/// The most pixels a PNG may have to be turned in memory.
const PNG_PIXEL_LIMIT: i64 = 100_000_000;

/// The kinds of picture Rotate turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rotatable {
    Jpeg,
    Png,
}

impl Rotatable {
    /// The kind for a content type, if Rotate turns it.
    pub(crate) fn of_type(content_type: &str) -> Option<Self> {
        match content_type {
            "image/jpeg" | "image/pjpeg" => Some(Rotatable::Jpeg),
            "image/png" => Some(Rotatable::Png),
            _ => None,
        }
    }

    fn of_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0xff, 0xd8]) {
            Some(Rotatable::Jpeg)
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Rotatable::Png)
        } else {
            None
        }
    }
}

/// Turns the photo at `path` a quarter turn clockwise, or anticlockwise
/// with `clockwise` false. Blocks: run it off the main thread.
pub(crate) fn rotate_file(path: &Path, clockwise: bool) -> Result<(), RotateError> {
    // One turn at a time: two quick presses each read the file the other
    // writes.
    static TURNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turning = TURNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bytes = fs::read(path)?;
    let turned = match Rotatable::of_bytes(&bytes).ok_or(RotateError::Unsupported)? {
        Rotatable::Jpeg => {
            let quarters = if clockwise { 1 } else { 3 };
            turn_jpeg(&bytes, quarters).ok_or(RotateError::Unsupported)?
        }
        Rotatable::Png => turn_png(path, clockwise)?,
    };
    replace_file(path, &turned)
}

/// Writes `bytes` beside `path` and renames them over it, keeping its
/// permissions.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), RotateError> {
    let folder = path.parent().ok_or(RotateError::Unsupported)?;
    let name = path.file_name().ok_or(RotateError::Unsupported)?;
    let permissions = fs::metadata(path)?.permissions();
    let mut temporary = folder.join(format!(".{}.rotating", name.to_string_lossy()));
    let mut attempt = 0;
    while temporary.exists() {
        attempt += 1;
        temporary = folder.join(format!(".{}.rotating-{attempt}", name.to_string_lossy()));
    }
    let written = (|| -> std::io::Result<()> {
        let mut file = fs::File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(written?)
}

/// The PNG at `path` with its pixels turned a quarter.
fn turn_png(path: &Path, clockwise: bool) -> Result<Vec<u8>, RotateError> {
    use gtk::gdk_pixbuf::PixbufRotation;
    // Decoded whole, by the picture decoder process: a PNG is untrusted,
    // and no C loader reads it in the app.
    let limits = crate::picture_guard::Limits {
        max_pixels: u64::try_from(PNG_PIXEL_LIMIT).unwrap_or(0),
        max_jpeg_pixels: 0,
        time: crate::picture_guard::TIME_LIMIT,
    };
    let file = gtk::gio::File::for_path(path);
    let picture = crate::picture_guard::decode_to_fit(&file, i32::MAX, i32::MAX, limits, None)
        .map_err(|_| RotateError::Unsupported)?
        .pixbuf;
    let rotation = if clockwise {
        PixbufRotation::Clockwise
    } else {
        PixbufRotation::Counterclockwise
    };
    let turned = picture.rotate_simple(rotation).ok_or(RotateError::Unsupported)?;
    turned
        .save_to_bufferv("png", &[])
        .map_err(|_| RotateError::Unsupported)
}

/// EXIF orientation as (quarter turns clockwise, mirrored first).
fn orientation_parts(orientation: u16) -> (u8, bool) {
    match orientation {
        2 => (0, true),
        3 => (2, false),
        4 => (2, true),
        5 => (3, true),
        6 => (1, false),
        7 => (1, true),
        8 => (3, false),
        _ => (0, false),
    }
}

/// The EXIF orientation for `quarters` clockwise after `mirrored`.
fn orientation_of(quarters: u8, mirrored: bool) -> u16 {
    match (quarters % 4, mirrored) {
        (0, false) => 1,
        (1, false) => 6,
        (2, false) => 3,
        (3, false) => 8,
        (0, true) => 2,
        (1, true) => 7,
        (2, true) => 4,
        _ => 5,
    }
}

/// The orientation `orientation` turned `quarters` more clockwise.
pub(crate) fn turned_orientation(orientation: u16, quarters: u8) -> u16 {
    let (turns, mirrored) = orientation_parts(orientation);
    orientation_of(turns + quarters, mirrored)
}

/// The orientation tag.
const ORIENTATION: u16 = 0x0112;
/// TIFF's SHORT type.
const SHORT: u16 = 3;

/// The JPEG `bytes` turned `quarters` clockwise by its EXIF orientation;
/// `None` for a JPEG whose segments or EXIF cannot be read.
pub(crate) fn turn_jpeg(bytes: &[u8], quarters: u8) -> Option<Vec<u8>> {
    let mut position = 2;
    let mut insert_at = 2;
    // The markers before the picture: APP0 (JFIF) stays first.
    while position + 4 <= bytes.len() && bytes[position] == 0xff {
        let marker = bytes[position + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = usize::from(u16::from_be_bytes([bytes[position + 2], bytes[position + 3]]));
        if length < 2 || position + 2 + length > bytes.len() {
            return None;
        }
        let body = &bytes[position + 4..position + 2 + length];
        if marker == 0xe1 && body.starts_with(b"Exif\0\0") {
            let tiff = &body[6..];
            let turned = turn_tiff(tiff, quarters)?;
            let mut out = Vec::with_capacity(bytes.len() + 64);
            out.extend_from_slice(&bytes[..position]);
            out.extend(app1(&turned)?);
            out.extend_from_slice(&bytes[position + 2 + length..]);
            return Some(out);
        }
        if marker == 0xe0 && position == 2 {
            insert_at = position + 2 + length;
        }
        position += 2 + length;
    }
    // No EXIF: a block with only the orientation.
    let tiff = orientation_only_tiff(turned_orientation(1, quarters));
    let mut out = Vec::with_capacity(bytes.len() + 64);
    out.extend_from_slice(&bytes[..insert_at]);
    out.extend(app1(&tiff)?);
    out.extend_from_slice(&bytes[insert_at..]);
    Some(out)
}

/// An APP1 segment holding the TIFF block `tiff`.
fn app1(tiff: &[u8]) -> Option<Vec<u8>> {
    let length = u16::try_from(2 + 6 + tiff.len()).ok()?;
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(tiff);
    Some(segment)
}

/// A little-endian TIFF block whose IFD0 holds only `orientation`.
fn orientation_only_tiff(orientation: u16) -> Vec<u8> {
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&8_u32.to_le_bytes());
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    tiff.extend_from_slice(&ORIENTATION.to_le_bytes());
    tiff.extend_from_slice(&SHORT.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    tiff
}

/// Reads and writes a TIFF block's numbers in its byte order.
#[derive(Clone, Copy)]
struct Order(bool);

impl Order {
    fn u16(self, bytes: &[u8], at: usize) -> Option<u16> {
        let pair: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
        Some(if self.0 {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        })
    }

    fn u32(self, bytes: &[u8], at: usize) -> Option<u32> {
        let quad: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if self.0 {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    }

    fn put_u16(self, value: u16) -> [u8; 2] {
        if self.0 {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    }

    fn put_u32(self, value: u32) -> [u8; 4] {
        if self.0 {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    }
}

/// The TIFF block `tiff` with IFD0's orientation turned `quarters`.
fn turn_tiff(tiff: &[u8], quarters: u8) -> Option<Vec<u8>> {
    let order = match tiff.get(..4)? {
        b"II*\0" => Order(true),
        b"MM\0*" => Order(false),
        _ => return None,
    };
    let ifd = usize::try_from(order.u32(tiff, 4)?).ok()?;
    let count = usize::from(order.u16(tiff, ifd)?);
    let entries = ifd + 2;
    let next_at = entries + count * 12;
    let next = order.u32(tiff, next_at)?;
    let mut out = tiff.to_vec();
    for index in 0..count {
        let entry = entries + index * 12;
        if order.u16(tiff, entry)? != ORIENTATION {
            continue;
        }
        if order.u16(tiff, entry + 2)? != SHORT || order.u32(tiff, entry + 4)? != 1 {
            return None;
        }
        let turned = turned_orientation(order.u16(tiff, entry + 8)?, quarters);
        out[entry + 8..entry + 10].copy_from_slice(&order.put_u16(turned));
        return Some(out);
    }
    // No orientation: IFD0 with one more entry, at the end of the block.
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let moved = u32::try_from(out.len()).ok()?;
    let mut rows: Vec<[u8; 12]> = (0..count)
        .map(|index| {
            let entry = entries + index * 12;
            tiff[entry..entry + 12].try_into().expect("twelve bytes")
        })
        .collect();
    let mut orientation = [0_u8; 12];
    orientation[0..2].copy_from_slice(&order.put_u16(ORIENTATION));
    orientation[2..4].copy_from_slice(&order.put_u16(SHORT));
    orientation[4..8].copy_from_slice(&order.put_u32(1));
    orientation[8..10].copy_from_slice(&order.put_u16(turned_orientation(1, quarters)));
    rows.push(orientation);
    rows.sort_by_key(|row| order.u16(row, 0).unwrap_or(u16::MAX));
    out.extend_from_slice(&order.put_u16(u16::try_from(rows.len()).ok()?));
    for row in rows {
        out.extend_from_slice(&row);
    }
    out.extend_from_slice(&order.put_u32(next));
    out[4..8].copy_from_slice(&order.put_u32(moved));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orientation_in(bytes: &[u8]) -> Option<u32> {
        let exif = exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(bytes))
            .ok()?;
        exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
            .value
            .get_uint(0)
    }

    /// Four quarter turns come back, and a mirrored photo stays mirrored.
    ///
    /// parity: HOME-032
    #[test]
    fn orientations_turn_a_quarter_at_a_time() {
        assert_eq!(turned_orientation(1, 1), 6);
        assert_eq!(turned_orientation(6, 1), 3);
        assert_eq!(turned_orientation(3, 1), 8);
        assert_eq!(turned_orientation(8, 1), 1);
        assert_eq!(turned_orientation(1, 3), 8);
        for start in 1..=8 {
            assert_eq!(turned_orientation(turned_orientation(start, 1), 3), start);
            let (_, mirrored) = orientation_parts(start);
            assert_eq!(orientation_parts(turned_orientation(start, 1)).1, mirrored);
        }
    }

    /// A JPEG without EXIF gets one with the orientation, a JPEG with
    /// EXIF but no orientation keeps its other fields, and one with an
    /// orientation has it changed in place; the picture's own bytes never
    /// change.
    ///
    /// parity: HOME-032
    #[gtk::test]
    fn jpegs_turn_by_their_orientation_alone() {
        let plain = crate::test_support::pictures::jpeg_bytes(0x0011_2233, None);
        let scan = |bytes: &[u8]| {
            let start = bytes
                .windows(2)
                .position(|pair| pair == [0xff, 0xda])
                .expect("a scan");
            bytes[start..].to_vec()
        };
        let turned = turn_jpeg(&plain, 1).expect("turned");
        assert_eq!(orientation_in(&turned), Some(6));
        assert_eq!(scan(&turned), scan(&plain));

        let dated = crate::test_support::pictures::jpeg_bytes(0x0011_2233, Some("2025:12:24 10:00:00"));
        let turned = turn_jpeg(&dated, 3).expect("turned");
        assert_eq!(orientation_in(&turned), Some(8));
        assert_eq!(
            crate::folder_view::gallery::date_taken_in(&turned),
            crate::folder_view::gallery::date_taken_in(&dated),
            "the date taken is kept"
        );
        let again = turn_jpeg(&turned, 1).expect("turned again");
        assert_eq!(orientation_in(&again), Some(1));
        assert_eq!(again.len(), turned.len(), "changed in place");
        assert_eq!(scan(&again), scan(&dated));

        assert!(turn_jpeg(b"\xff\xd8\xff\xe1\x00\x10Exif\0\0XX*\0", 1).is_none());
    }

    /// A PNG's pixels turn and its file is replaced whole.
    ///
    /// parity: HOME-032
    #[gtk::test]
    fn a_png_turns_by_its_pixels() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("chart.png");
        fs::write(
            &path,
            crate::test_support::pictures::png_sized(30, 10, 0x0012_3456),
        )
        .expect("a PNG");
        rotate_file(&path, true).expect("turned");
        let turned = gtk::gdk_pixbuf::Pixbuf::from_file(&path).expect("a PNG");
        assert_eq!((turned.width(), turned.height()), (10, 30));
        let names: Vec<_> = fs::read_dir(folder.path()).expect("listed").collect();
        assert_eq!(names.len(), 1, "nothing is left beside it");
        fs::write(&path, b"not a picture").expect("a file");
        assert!(matches!(rotate_file(&path, true), Err(RotateError::Unsupported)));
    }
}
