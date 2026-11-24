// SPDX-License-Identifier: AGPL-3.0-only
//! The images of an icon: PNG files checked before anything decodes them,
//! and Windows bitmaps decoded to RGBA here, then written as PNG.
//!
//! An icon image is either a whole PNG file or a device-independent
//! bitmap: a `BITMAPINFOHEADER`, a palette for 1, 4 and 8 bits per pixel,
//! the color rows and a 1-bit transparency mask (the AND mask), both
//! stored bottom row first and twice as high in the header as the image,
//! since the header counts both. Only uncompressed bitmaps of 1, 4, 8, 24
//! and 32 bits per pixel are read, and only up to 256 pixels square, the
//! largest icon Windows draws.

use std::io::Write;

use super::pe::MAX_IMAGE_BYTES;
use super::IconFileError;

/// The largest edge of an icon image, in pixels.
pub(super) const MAX_EDGE: u32 = 256;

/// The eight bytes every PNG file starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// An image as rows of red, green, blue and alpha bytes, top row first,
/// not premultiplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// An icon image, ready to be written as PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A PNG file whose header was checked, kept as it is.
    Png {
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// The file.
        bytes: Vec<u8>,
    },
    /// A bitmap decoded to RGBA.
    Bitmap(Rgba),
}

impl Decoded {
    /// The image's width and height.
    pub fn size(&self) -> (u32, u32) {
        match self {
            Decoded::Png { width, height, .. } => (*width, *height),
            Decoded::Bitmap(image) => (image.width, image.height),
        }
    }

    /// The image as a PNG file.
    pub fn to_png(&self) -> Vec<u8> {
        match self {
            Decoded::Png { bytes, .. } => bytes.clone(),
            Decoded::Bitmap(image) => encode_png(image),
        }
    }
}

/// Reads one icon image: a PNG whose header is checked, or a bitmap
/// decoded to RGBA.
///
/// # Errors
///
/// [`IconFileError::Damaged`] for anything that is neither, is cut short,
/// uses a format left out (compressed or 16-bit bitmaps) or is larger
/// than [`MAX_EDGE`] pixels or [`MAX_IMAGE_BYTES`].
pub fn decode(data: &[u8]) -> Result<Decoded, IconFileError> {
    if data.len() > MAX_IMAGE_BYTES {
        return Err(IconFileError::OverLimit);
    }
    if data.starts_with(&PNG_SIGNATURE) {
        let (width, height) = png_size(data).ok_or(IconFileError::Damaged)?;
        return Ok(Decoded::Png {
            width,
            height,
            bytes: data.to_vec(),
        });
    }
    decode_bitmap(data).map(Decoded::Bitmap)
}

/// A big-endian `u32` at `at`.
fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let field = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([field[0], field[1], field[2], field[3]]))
}

/// The size a PNG file's header gives, once the header is checked: the
/// first chunk is a 13-byte `IHDR` whose checksum matches, of 1 to 256
/// pixels each way, with a bit depth and color type PNG allows.
fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if be_u32(data, 8)? != 13 || data.get(12..16)? != b"IHDR" {
        return None;
    }
    let header = data.get(16..29)?;
    let checksum = be_u32(data, 29)?;
    if crc32fast::hash(data.get(12..29)?) != checksum {
        return None;
    }
    let width = be_u32(header, 0)?;
    let height = be_u32(header, 4)?;
    let (depth, color) = (header[8], header[9]);
    let allowed = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    };
    let fits = (1..=MAX_EDGE).contains(&width) && (1..=MAX_EDGE).contains(&height);
    (allowed && fits).then_some((width, height))
}

/// A little-endian field of `N` bytes at `at`.
fn le<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], IconFileError> {
    let end = at.checked_add(N).ok_or(IconFileError::Damaged)?;
    let field = bytes.get(at..end).ok_or(IconFileError::Damaged)?;
    field.try_into().map_err(|_| IconFileError::Damaged)
}

/// The bytes of one stored row of `width` pixels at `bits` per pixel:
/// rows are padded to whole 32-bit words.
fn stride(width: usize, bits: usize) -> usize {
    (width * bits).div_ceil(32) * 4
}

/// Decodes a bitmap icon image to RGBA.
fn decode_bitmap(data: &[u8]) -> Result<Rgba, IconFileError> {
    let header_size = u32::from_le_bytes(le(data, 0)?) as usize;
    // BITMAPINFOHEADER and its later versions, up to BITMAPV5HEADER.
    if !(40..=124).contains(&header_size) {
        return Err(IconFileError::Damaged);
    }
    let width = i32::from_le_bytes(le(data, 4)?);
    let stored_height = i32::from_le_bytes(le(data, 8)?);
    let bits = u16::from_le_bytes(le(data, 14)?);
    let compression = u32::from_le_bytes(le(data, 16)?);
    let colors_used = u32::from_le_bytes(le(data, 32)?);
    // The header's height counts the color rows and the mask's; a
    // negative one (rows stored top first) is not used for icons.
    let width = u32::try_from(width).map_err(|_| IconFileError::Damaged)?;
    let stored_height = u32::try_from(stored_height).map_err(|_| IconFileError::Damaged)?;
    let height = stored_height / 2;
    if !(1..=MAX_EDGE).contains(&width) || !(1..=MAX_EDGE).contains(&height) || stored_height % 2 != 0 {
        return Err(IconFileError::Damaged);
    }
    if compression != 0 || !matches!(bits, 1 | 4 | 8 | 24 | 32) {
        return Err(IconFileError::Damaged);
    }
    let (width, height, bits) = (width as usize, height as usize, usize::from(bits));
    let palette_size = if bits <= 8 {
        let full = 1usize << bits;
        match colors_used as usize {
            0 => full,
            used if used <= full => used,
            _ => return Err(IconFileError::Damaged),
        }
    } else {
        0
    };
    let palette_start = header_size;
    let colors_start = palette_start + palette_size * 4;
    let color_stride = stride(width, bits);
    let mask_start = colors_start + color_stride * height;
    let mask_stride = stride(width, 1);
    let palette = data
        .get(palette_start..colors_start)
        .ok_or(IconFileError::Damaged)?;
    let colors = data.get(colors_start..mask_start).ok_or(IconFileError::Damaged)?;
    // Some files leave the mask out; their pixels are then opaque, or
    // keep the alpha of a 32-bit image.
    let mask = data.get(mask_start..mask_start + mask_stride * height);

    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        // Rows are stored bottom first.
        let row = &colors[(height - 1 - y) * color_stride..][..color_stride];
        for x in 0..width {
            let rgba = match bits {
                32 => [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], row[x * 4 + 3]],
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                _ => {
                    let per_byte = 8 / bits;
                    let byte = row[x / per_byte];
                    let shift = 8 - bits * (x % per_byte + 1);
                    let index = usize::from(byte >> shift) & ((1 << bits) - 1);
                    let entry = palette
                        .get(index * 4..index * 4 + 4)
                        .ok_or(IconFileError::Damaged)?;
                    [entry[2], entry[1], entry[0], 255]
                }
            };
            pixels[(y * width + x) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    // A 32-bit image without any alpha is an old one whose transparency
    // is the mask's, as for the other depths.
    let has_alpha = bits == 32 && pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    if let (Some(mask), false) = (mask, has_alpha) {
        for y in 0..height {
            let row = &mask[(height - 1 - y) * mask_stride..][..mask_stride];
            for x in 0..width {
                let transparent = row[x / 8] & (0x80 >> (x % 8)) != 0;
                pixels[(y * width + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }
    Ok(Rgba {
        width: u32::try_from(width).map_err(|_| IconFileError::Damaged)?,
        height: u32::try_from(height).map_err(|_| IconFileError::Damaged)?,
        pixels,
    })
}

/// Writes `image` as an 8-bit RGBA PNG file.
///
/// # Panics
///
/// If `image.pixels` does not hold `width * height` pixels.
pub fn encode_png(image: &Rgba) -> Vec<u8> {
    let row_bytes = image.width as usize * 4;
    assert_eq!(
        image.pixels.len(),
        row_bytes * image.height as usize,
        "one RGBA pixel each"
    );
    let mut rows = Vec::with_capacity((row_bytes + 1) * image.height as usize);
    for row in image.pixels.chunks_exact(row_bytes.max(1)) {
        // Filter type 0: the row as it is.
        rows.push(0);
        rows.extend_from_slice(row);
    }
    let mut compressor = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    compressor
        .write_all(&rows)
        .expect("compressing into memory cannot fail");
    let compressed = compressor.finish().expect("compressing into memory cannot fail");

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    // 8 bits per channel, color type 6 (RGBA), deflate, no interlace.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = PNG_SIGNATURE.to_vec();
    for (kind, body) in [(b"IHDR", header), (b"IDAT", compressed), (b"IEND", Vec::new())] {
        png.extend_from_slice(&u32::try_from(body.len()).expect("an icon is small").to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(&body);
        let checksum = crc32fast::hash(&png[start..]);
        png.extend_from_slice(&checksum.to_be_bytes());
    }
    png
}
