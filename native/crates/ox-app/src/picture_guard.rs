// SPDX-License-Identifier: AGPL-3.0-only
//! Decoding pictures in the app without letting one picture freeze it or
//! use up its memory: the thumbnails the app makes of pictures itself,
//! the files it reads back from the shared thumbnail cache, the details
//! pane's preview and custom icons all decode through [`decode_to_fit`].
//! (A desktop thumbnailer's picture is checked by
//! [`crate::thumbnails`] before it is decoded, and the program itself
//! runs in a sandbox with its own limits.)
//!
//! A picture is fed to gdk-pixbuf in chunks. As soon as the loader has
//! read the size from the header, and before it allocates any pixels, a
//! raster picture of more than [`MAX_PIXELS`] is refused (the loader is
//! told to load nothing, as gdk-pixbuf's own `file_info` stops after the
//! header); a smaller one is decoded straight to the size it is shown at,
//! which the JPEG loader does while decoding. Vector pictures (SVG) are
//! drawn at the size shown whatever size they declare. An animated GIF
//! is fed only up to the end of its first frame, as Windows shows a GIF's
//! first frame, so thousands of frames are never held. Reading stops
//! once the time limit has passed, looked at between reads ([`TIME_LIMIT`]). Every failure is a [`Refusal`], and
//! callers show the file's type icon instead, as Windows Explorer does
//! for a picture it cannot preview.
//!
//! Decoding blocks: run it off the main thread.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Pixbuf, PixbufFormat, PixbufLoader};
use gtk::gio;
use gtk::prelude::*;

/// The most pixels a raster picture may have to be decoded for display:
/// 64 megapixels (8192 × 8192), more than any camera's photo, 256 MB
/// at four bytes a pixel.
pub(crate) const MAX_PIXELS: u64 = 8192 * 8192;

/// The most pixels of a file in the shared thumbnail cache, whose largest
/// standard size is 1024 × 1024 (xx-large).
pub(crate) const MAX_THUMBNAIL_FILE_PIXELS: u64 = 1024 * 1024;

/// How long one picture may take to read and decode. The limit is looked
/// at between the reads of [`READ_CHUNK`] bytes, so it is best effort: a
/// read that blocks (a server that stopped answering), or gdk-pixbuf
/// decoding one chunk, is not interrupted, and the picture is refused only
/// once that read or chunk is done.
pub(crate) const TIME_LIMIT: Duration = Duration::from_secs(10);

/// How much of a picture is read at a time.
const READ_CHUNK: usize = 64 * 1024;

/// The most pixels a JPEG may have to be decoded for display: 256
/// megapixels (16384 × 16384). gdk-pixbuf's JPEG loader decodes a large
/// one straight to an eighth of its sides or less when it is shown
/// smaller, so it never holds more than [`MAX_PIXELS`]' worth; other
/// formats are decoded whole first.
pub(crate) const MAX_JPEG_PIXELS: u64 = 16_384 * 16_384;

/// How much a picture may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The most pixels of a raster picture.
    pub max_pixels: u64,
    /// The most pixels of a JPEG, which is decoded scaled down.
    pub max_jpeg_pixels: u64,
    /// How long reading and decoding may take.
    pub time: Duration,
}

impl Limits {
    /// The limits of a picture shown in the app.
    pub(crate) const DISPLAY: Self = Self {
        max_pixels: MAX_PIXELS,
        max_jpeg_pixels: MAX_JPEG_PIXELS,
        time: TIME_LIMIT,
    };

    /// The limits of a file read from the thumbnail cache.
    pub(crate) const THUMBNAIL_FILE: Self = Self {
        max_pixels: MAX_THUMBNAIL_FILE_PIXELS,
        max_jpeg_pixels: MAX_THUMBNAIL_FILE_PIXELS,
        time: TIME_LIMIT,
    };
}

/// Why a picture was not decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Its header gives more pixels than the limit, or no pixels; the size
    /// is the header's.
    TooLarge {
        /// The width the header gives.
        width: i32,
        /// The height the header gives.
        height: i32,
    },
    /// It could not be read, or is not a picture gdk-pixbuf reads.
    Unreadable,
    /// gdk-pixbuf read it, but it is damaged or cut short.
    Damaged,
    /// It took longer than the time limit.
    TimedOut,
    /// The caller no longer wants it.
    Cancelled,
}

/// A decoded picture.
#[derive(Debug, Clone)]
pub(crate) struct Decoded {
    /// The picture at the size it is shown at, turned upright.
    pub pixbuf: Pixbuf,
    /// The picture's own width, from its header.
    pub width: i32,
    /// The picture's own height, from its header.
    pub height: i32,
}

/// What the loader reported while the picture was fed to it.
#[derive(Debug, Default)]
struct Header {
    /// The size the header gives.
    size: Cell<Option<(i32, i32)>>,
    /// The picture was refused at its header.
    refused: Cell<bool>,
}

/// The picture in `file`, decoded to fit `width` × `height`, or at its own
/// size when smaller, within `limits`. Blocks.
pub(crate) fn decode_to_fit(
    file: &gio::File,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    let started = Instant::now();
    let stream = file.read(cancellable).map_err(|_| refusal_for(cancellable))?;
    let read = |buffer: &mut [u8]| stream.read(buffer, cancellable).map_err(|_| ());
    decode_read_to_fit(read, started, width, height, limits, cancellable)
}

/// The picture `reader` reads, decoded as [`decode_to_fit`] decodes a
/// file's: for a file the caller opened itself, and checked is a plain
/// file, so no link or pipe is opened after the check. Blocks.
pub(crate) fn decode_reader_to_fit(
    reader: &mut impl std::io::Read,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    let read = |buffer: &mut [u8]| reader.read(buffer).map_err(|_| ());
    decode_read_to_fit(read, Instant::now(), width, height, limits, cancellable)
}

/// The picture `next_chunk` hands over a chunk at a time, decoded within
/// `limits` from `started`.
fn decode_read_to_fit(
    mut next_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
    started: Instant,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    let loader = PixbufLoader::new();
    let header = Rc::new(Header::default());
    watch_header(&loader, &header, (width.max(1), height.max(1)), limits);
    let mut buffer = vec![0; READ_CHUNK];
    let mut first_frame = FirstFrame::default();
    let mut ending = Ending::default();
    let fed = loop {
        if cancellable.is_some_and(CancellableExt::is_cancelled) {
            break Err(Refusal::Cancelled);
        }
        if started.elapsed() >= limits.time {
            break Err(Refusal::TimedOut);
        }
        let Ok(read) = next_chunk(&mut buffer) else {
            break Err(refusal_for(cancellable));
        };
        if read == 0 {
            break Ok(());
        }
        let (chunk, first_frame_done) = first_frame.take(&buffer[..read]);
        let (chunk, png_done) = ending.take(chunk);
        let done = first_frame_done || png_done;
        let written = loader.write(chunk).and_then(|()| {
            if first_frame_done {
                // The GIF trailer ends the picture after its first frame.
                loader.write(&[GIF_TRAILER])
            } else {
                Ok(())
            }
        });
        if header.refused.get() {
            break Err(Refusal::TooLarge { width: 0, height: 0 });
        }
        if written.is_err() {
            break Err(if header.size.get().is_some() {
                Refusal::Damaged
            } else {
                Refusal::Unreadable
            });
        }
        if done {
            break Ok(());
        }
    };
    let closed = loader.close();
    if header.refused.get() {
        let (width, height) = header.size.get().unwrap_or((0, 0));
        return Err(Refusal::TooLarge { width, height });
    }
    fed?;
    if started.elapsed() >= limits.time {
        return Err(Refusal::TimedOut);
    }
    let read_header = header.size.get().is_some();
    let damaged = || {
        if read_header {
            Refusal::Damaged
        } else {
            Refusal::Unreadable
        }
    };
    closed.map_err(|_| damaged())?;
    if !ending.is_complete() {
        return Err(Refusal::Damaged);
    }
    let pixbuf = loader.pixbuf().ok_or(Refusal::Unreadable)?;
    let (width, height) = header.size.get().ok_or(Refusal::Unreadable)?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    Ok(Decoded {
        pixbuf,
        width,
        height,
    })
}

/// Cancelled when `cancellable` says so, else unreadable.
fn refusal_for(cancellable: Option<&gio::Cancellable>) -> Refusal {
    if cancellable.is_some_and(CancellableExt::is_cancelled) {
        Refusal::Cancelled
    } else {
        Refusal::Unreadable
    }
}

/// Records the size `loader` reads from the header into `header`, and
/// tells the loader to load nothing when a raster picture has more pixels
/// than `limits` allow its format, else to decode to fit `fit` (vector
/// pictures always).
fn watch_header(loader: &PixbufLoader, header: &Rc<Header>, fit: (i32, i32), limits: Limits) {
    let header = Rc::clone(header);
    loader.connect_size_prepared(move |loader, width, height| {
        header.size.set(Some((width, height)));
        let format = loader.format();
        let is_vector = format.as_ref().is_some_and(PixbufFormat::is_scalable);
        let is_jpeg = format
            .as_ref()
            .is_some_and(|format| format.name().as_deref() == Some("jpeg"));
        let max_pixels = if is_jpeg {
            limits.max_jpeg_pixels.max(limits.max_pixels)
        } else {
            limits.max_pixels
        };
        if width <= 0 || height <= 0 || (!is_vector && pixels(width, height) > max_pixels) {
            header.refused.set(true);
            // A size of zero makes every gdk-pixbuf loader stop before it
            // allocates the picture, as gdk_pixbuf_get_file_info does.
            loader.set_size(0, 0);
            return;
        }
        if width > fit.0 || height > fit.1 {
            let (width, height) = fit_within(width, height, fit);
            loader.set_size(width, height);
        }
    });
}

/// The pixels of a `width` × `height` picture.
fn pixels(width: i32, height: i32) -> u64 {
    let side = |side: i32| u64::try_from(side).unwrap_or(0);
    side(width).saturating_mul(side(height))
}

/// `width` × `height` scaled to fit within `fit`, keeping the aspect
/// ratio and at least one pixel each way.
pub(crate) fn fit_within(width: i32, height: i32, fit: (i32, i32)) -> (i32, i32) {
    let (width, height) = (i64::from(width.max(1)), i64::from(height.max(1)));
    let (box_width, box_height) = (i64::from(fit.0.max(1)), i64::from(fit.1.max(1)));
    // The smaller of the two scales, compared without rounding.
    let (scaled_width, scaled_height) = if width * box_height >= height * box_width {
        (box_width, height * box_width / width)
    } else {
        (width * box_height / height, box_height)
    };
    let to_i32 = |side: i64| i32::try_from(side.max(1)).unwrap_or(i32::MAX);
    (to_i32(scaled_width), to_i32(scaled_height))
}

/// A PNG's signature.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Where a PNG ends: gdk-pixbuf draws a PNG cut short without an error,
/// so a PNG must reach its IEND chunk, as Windows shows no thumbnail of a
/// PNG cut short. What follows IEND (a tool's appended data) is not fed
/// to the loader, and the picture is shown, as Windows shows it. The
/// chunks are walked by their lengths, so bytes that only look like IEND
/// inside a chunk's data count for nothing. (A JPEG cut short is drawn as
/// far as it goes, gray below, as Windows draws it too.) Other formats
/// pass through whole.
#[derive(Debug, Default)]
struct Ending {
    /// The first bytes fed, up to a PNG signature's length.
    start: Vec<u8>,
    /// How many bytes were fed.
    fed: u64,
    /// Where the next chunk starts, while the file is a PNG not yet at
    /// its IEND.
    next_chunk: Option<u64>,
    /// The next chunk's length and type, as far as they were fed.
    chunk_header: Vec<u8>,
    /// Where IEND ends, once its header was fed.
    end_at: Option<u64>,
    /// IEND was fed whole.
    ended: bool,
}

impl Ending {
    /// The part of `chunk` to feed, and whether the PNG ends with it.
    fn take<'a>(&mut self, chunk: &'a [u8]) -> (&'a [u8], bool) {
        let base = self.fed;
        let length = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        let past = base.saturating_add(length);
        self.fed = past;
        let mut index = 0;
        while self.start.len() < PNG_SIGNATURE.len() && index < chunk.len() {
            self.start.push(chunk[index]);
            index += 1;
            if self.start == PNG_SIGNATURE {
                self.next_chunk = Some(base + index as u64);
            }
        }
        loop {
            if let Some(end) = self.end_at {
                if end > past {
                    return (chunk, false);
                }
                self.ended = true;
                let cut = usize::try_from(end.saturating_sub(base)).unwrap_or(chunk.len());
                return (&chunk[..cut.min(chunk.len())], true);
            }
            // Not a PNG, or its signature not yet whole: fed whole.
            let Some(next) = self.next_chunk else {
                return (chunk, false);
            };
            let wanted = next + self.chunk_header.len() as u64;
            if wanted >= past {
                return (chunk, false);
            }
            let from = usize::try_from(wanted - base).unwrap_or(chunk.len());
            let count = (8 - self.chunk_header.len()).min(chunk.len() - from);
            self.chunk_header.extend_from_slice(&chunk[from..from + count]);
            if self.chunk_header.len() < 8 {
                return (chunk, false);
            }
            let data = u64::from(u32::from_be_bytes([
                self.chunk_header[0],
                self.chunk_header[1],
                self.chunk_header[2],
                self.chunk_header[3],
            ]));
            let is_end = &self.chunk_header[4..8] == b"IEND";
            self.chunk_header.clear();
            // The header (8 bytes), the data and the CRC (4 bytes).
            let chunk_end = next + 8 + data + 4;
            if is_end {
                self.next_chunk = None;
                self.end_at = Some(chunk_end);
            } else {
                self.next_chunk = Some(chunk_end);
            }
        }
    }

    /// False for a PNG that did not reach the end of its IEND chunk.
    fn is_complete(&self) -> bool {
        self.start != PNG_SIGNATURE || self.ended
    }
}

/// The byte that ends a GIF.
const GIF_TRAILER: u8 = 0x3B;

/// Where a GIF's parser is, block by block (the `GIF89a` specification, sections 17 to 27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GifState {
    /// The first bytes, not yet known to be a GIF.
    #[default]
    Signature,
    /// Not a GIF, or a GIF already past its first frame: fed whole.
    Whole,
    /// Inside the header and logical screen descriptor, this many bytes
    /// left.
    Screen(usize),
    /// Inside a color table, `left` bytes left; an image's local table
    /// is followed by its LZW code size, the global one by a block.
    ColorTable {
        /// The bytes left.
        left: usize,
        /// The table is an image's own.
        local: bool,
    },
    /// Before a block: an extension, an image or the trailer.
    Block,
    /// After an extension's introducer, before its label.
    ExtensionLabel,
    /// Inside an image descriptor, this many bytes left (after the
    /// separator).
    ImageDescriptor(usize),
    /// The LZW minimum code size before an image's data.
    CodeSize,
    /// Before a data sub-block's size; `image` for an image's data.
    SubBlockSize {
        /// The sub-blocks are an image's data.
        image: bool,
    },
    /// Inside a data sub-block, this many bytes left.
    SubBlock {
        /// The sub-block is an image's data.
        image: bool,
        /// The bytes left.
        left: usize,
    },
}

/// Finds the end of a GIF's first frame in the bytes fed to the loader, so
/// only that frame is decoded. Other formats pass through whole.
#[derive(Debug, Default)]
struct FirstFrame {
    state: GifState,
    /// The signature bytes seen so far.
    signature: Vec<u8>,
    /// The image descriptor's packed field is next.
    descriptor_flags: Option<u8>,
}

impl FirstFrame {
    /// The part of `chunk` to feed, and whether the first frame ends with
    /// it (the trailer is then fed after it).
    fn take<'a>(&mut self, chunk: &'a [u8]) -> (&'a [u8], bool) {
        for (index, &byte) in chunk.iter().enumerate() {
            if self.state == GifState::Whole {
                return (chunk, false);
            }
            if self.step(byte) {
                self.state = GifState::Whole;
                return (&chunk[..=index], true);
            }
        }
        (chunk, false)
    }

    /// Reads one byte; true when it ends the first frame's data.
    fn step(&mut self, byte: u8) -> bool {
        self.state = match self.state {
            GifState::Whole => GifState::Whole,
            GifState::Signature => {
                self.signature.push(byte);
                if !b"GIF8".starts_with(&self.signature) && self.signature.len() <= 4 {
                    GifState::Whole
                } else if self.signature.len() == 4 {
                    // "GIF8" read; "7a" or "9a" and the 7-byte screen
                    // descriptor follow, whose packed byte is the 5th.
                    GifState::Screen(2 + 7)
                } else {
                    GifState::Signature
                }
            }
            GifState::Screen(left) => {
                // The packed field is 3 bytes before the end of the
                // screen descriptor.
                let left = left - 1;
                if left == 2 {
                    self.descriptor_flags = Some(byte);
                }
                if left > 0 {
                    GifState::Screen(left)
                } else {
                    color_table_after(self.descriptor_flags.take().unwrap_or(0), false)
                }
            }
            GifState::ColorTable { left, local } => {
                if left > 1 {
                    GifState::ColorTable {
                        left: left - 1,
                        local,
                    }
                } else if local {
                    GifState::CodeSize
                } else {
                    GifState::Block
                }
            }
            GifState::Block => match byte {
                0x21 => GifState::ExtensionLabel,
                0x2C => GifState::ImageDescriptor(9),
                // The trailer, or anything else: let the loader decide.
                _ => GifState::Whole,
            },
            GifState::ExtensionLabel => GifState::SubBlockSize { image: false },
            GifState::ImageDescriptor(left) => {
                let left = left - 1;
                if left == 0 {
                    // The last byte is the packed field.
                    color_table_after(byte, true)
                } else {
                    GifState::ImageDescriptor(left)
                }
            }
            GifState::CodeSize => GifState::SubBlockSize { image: true },
            GifState::SubBlockSize { image } => {
                if byte == 0 {
                    if image {
                        return true;
                    }
                    GifState::Block
                } else {
                    GifState::SubBlock {
                        image,
                        left: usize::from(byte),
                    }
                }
            }
            GifState::SubBlock { image, left } => {
                if left > 1 {
                    GifState::SubBlock {
                        image,
                        left: left - 1,
                    }
                } else {
                    GifState::SubBlockSize { image }
                }
            }
        };
        false
    }
}

/// The color table a packed field with `flags` announces (its top bit
/// set, 3 × 2^(n+1) bytes for its low three bits n), else the state that
/// follows without one: an image's LZW code size (`local`) or a block.
fn color_table_after(flags: u8, local: bool) -> GifState {
    if flags & 0x80 != 0 {
        GifState::ColorTable {
            left: 3 << ((flags & 0x07) + 1),
            local,
        }
    } else if local {
        GifState::CodeSize
    } else {
        GifState::Block
    }
}

/// Writes at `path` a PNG whose header says 30,000 × 30,000 pixels (900
/// megapixels, 3.6 GB decoded), followed by the start of a small valid
/// picture's data, as a decompression bomb begins; for tests.
#[cfg(test)]
pub(crate) fn huge_png(path: &std::path::Path) -> std::path::PathBuf {
    let pixbuf = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 64).expect("a picture");
    pixbuf.fill(0x3366_99ff);
    let mut bytes = pixbuf.save_to_bufferv("png", &[]).expect("a PNG");
    // IHDR's width and height follow the signature (8), the chunk's
    // length (4) and type (4); its CRC is written again after them.
    bytes[16..20].copy_from_slice(&30_000_u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&30_000_u32.to_be_bytes());
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    std::fs::write(path, bytes).expect("the huge PNG");
    path.to_owned()
}

/// The CRC-32 of a PNG chunk, for tests.
#[cfg(test)]
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gtk::gdk_pixbuf::Colorspace;

    use super::*;

    /// A picture `width` × `height` filled with one color, saved as
    /// `kind` ("png" or "jpeg") in `folder`.
    fn saved_picture(folder: &Path, name: &str, kind: &str, width: i32, height: i32) -> PathBuf {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, width, height).expect("a picture");
        pixbuf.fill(0x3366_99ff);
        let path = folder.join(name);
        pixbuf.savev(&path, kind, &[]).expect("the picture is saved");
        path
    }

    fn decode(path: &Path, limits: Limits) -> Result<Decoded, Refusal> {
        decode_to_fit(&gio::File::for_path(path), 256, 256, limits, None)
    }

    /// A picture whose header asks for more than the limit is refused
    /// from its header, at once, before gdk-pixbuf allocates its pixels.
    ///
    /// parity: VIEW-057, PROP-011
    #[test]
    fn a_picture_over_the_pixel_limit_is_refused_from_its_header() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = huge_png(&folder.path().join("Huge poster.png"));
        let started = Instant::now();
        assert_eq!(
            decode(&path, Limits::DISPLAY).map(|decoded| decoded.width),
            Err(Refusal::TooLarge {
                width: 30_000,
                height: 30_000
            })
        );
        assert!(started.elapsed() < Duration::from_secs(2), "refused at once");

        // A JPEG's size is read the same way.
        let small = saved_picture(folder.path(), "small.jpg", "jpeg", 64, 48);
        let mut bytes = std::fs::read(&small).expect("the JPEG");
        let frame = bytes
            .windows(2)
            .position(|marker| marker == [0xFF, 0xC0])
            .expect("a baseline frame header");
        // Length (2) and precision (1), then height and width.
        bytes[frame + 5..frame + 7].copy_from_slice(&30_000_u16.to_be_bytes());
        bytes[frame + 7..frame + 9].copy_from_slice(&30_000_u16.to_be_bytes());
        let jpeg = folder.path().join("Huge scan.jpg");
        std::fs::write(&jpeg, bytes).expect("the huge JPEG");
        assert_eq!(
            decode(&jpeg, Limits::DISPLAY).map(|decoded| decoded.width),
            Err(Refusal::TooLarge {
                width: 30_000,
                height: 30_000
            })
        );
    }

    /// A picture within the limit is decoded to fit the size asked, and
    /// its own size is reported from its header.
    #[test]
    fn a_picture_within_the_limit_is_decoded_to_fit() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = saved_picture(folder.path(), "Beach.png", "png", 1200, 600);
        let decoded = decode(&path, Limits::DISPLAY).expect("a preview");
        assert_eq!((decoded.width, decoded.height), (1200, 600));
        assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (256, 128));
        let small = saved_picture(folder.path(), "Icon.png", "png", 40, 30);
        let decoded = decode(&small, Limits::DISPLAY).expect("a preview");
        assert_eq!(
            (decoded.pixbuf.width(), decoded.pixbuf.height()),
            (40, 30),
            "never enlarged"
        );
        let limits = Limits {
            max_pixels: 1200 * 600 - 1,
            max_jpeg_pixels: 0,
            ..Limits::DISPLAY
        };
        assert_eq!(
            decode(&path, limits).map(|decoded| decoded.width),
            Err(Refusal::TooLarge {
                width: 1200,
                height: 600
            }),
            "one pixel over the limit"
        );
        // A JPEG, decoded scaled down, may have more pixels than another
        // format.
        let limits = Limits {
            max_pixels: 100 * 100 - 1,
            max_jpeg_pixels: 100 * 100,
            ..Limits::DISPLAY
        };
        let jpeg = saved_picture(folder.path(), "Pier.jpg", "jpeg", 100, 100);
        let png = saved_picture(folder.path(), "Pier.png", "png", 100, 100);
        assert!(decode(&jpeg, limits).is_ok(), "a JPEG within its own limit");
        assert_eq!(
            decode(&png, limits).map(|decoded| decoded.width),
            Err(Refusal::TooLarge {
                width: 100,
                height: 100
            })
        );
    }

    /// A PNG or JPEG cut short, or a file that is no picture at all, is
    /// refused rather than shown half drawn.
    #[test]
    fn a_truncated_or_damaged_picture_is_refused() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = saved_picture(folder.path(), "Forest.png", "png", 300, 200);
        let bytes = std::fs::read(&path).expect("the picture");
        for cut in [bytes.len() / 2, bytes.len() - 1] {
            std::fs::write(&path, &bytes[..cut]).expect("cut short");
            assert_eq!(
                decode(&path, Limits::DISPLAY).map(|_| ()),
                Err(Refusal::Damaged),
                "a PNG cut at {cut} of {} bytes",
                bytes.len()
            );
        }
        // Bytes a tool appended after IEND do not spoil it, as Windows
        // shows such a PNG too; they are not fed to the loader.
        let mut appended = bytes.clone();
        appended.extend_from_slice(b"appended by a tool, not a chunk at all");
        std::fs::write(&path, &appended).expect("appended");
        let decoded = decode(&path, Limits::DISPLAY).expect("a PNG with data after IEND");
        assert_eq!((decoded.width, decoded.height), (300, 200));
        // Its end is found however the reads split it.
        for size in [1, 7, 13, 4096] {
            let mut ending = Ending::default();
            let mut fed = 0;
            let mut done = false;
            for chunk in appended.chunks(size) {
                let (part, ends) = ending.take(chunk);
                fed += part.len();
                if ends {
                    done = true;
                    break;
                }
            }
            assert!(done && ending.is_complete(), "reads of {size}");
            assert_eq!(fed, bytes.len(), "reads of {size}: up to the end of IEND");
        }
        // Cut inside its header, it is no picture gdk-pixbuf can read.
        std::fs::write(&path, &bytes[..40]).expect("cut short");
        assert_eq!(
            decode(&path, Limits::DISPLAY).map(|_| ()),
            Err(Refusal::Unreadable)
        );
        // A JPEG cut short is drawn as far as it goes, or refused, at once.
        let path = saved_picture(folder.path(), "Harbour.jpg", "jpeg", 300, 200);
        let bytes = std::fs::read(&path).expect("the picture");
        std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("cut short");
        let started = Instant::now();
        if let Ok(decoded) = decode(&path, Limits::DISPLAY) {
            assert!(decoded.pixbuf.width() <= 256 && decoded.pixbuf.height() <= 256);
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        let noise = folder.path().join("Sunset.png");
        std::fs::write(&noise, b"\x89PNG\r\n\x1a\nnot really a picture").expect("noise");
        assert_eq!(
            decode(&noise, Limits::DISPLAY).map(|_| ()),
            Err(Refusal::Unreadable)
        );
        let missing = folder.path().join("Missing.png");
        assert_eq!(
            decode(&missing, Limits::DISPLAY).map(|_| ()),
            Err(Refusal::Unreadable)
        );
    }

    /// A broken SVG is refused; a valid one declaring a huge size is drawn
    /// at the size asked, as a vector picture costs only what is shown.
    #[test]
    fn svgs_are_drawn_at_the_size_shown_and_broken_ones_refused() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let broken = folder.path().join("Logo draft.svg");
        std::fs::write(
            &broken,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" <rect",
        )
        .expect("svg");
        let started = Instant::now();
        assert!(decode(&broken, Limits::DISPLAY).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        let huge = folder.path().join("Banner.svg");
        std::fs::write(
            &huge,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100000\" height=\"50000\">\
             <rect width=\"100000\" height=\"50000\" fill=\"#3a7\"/></svg>",
        )
        .expect("svg");
        let decoded = decode(&huge, Limits::DISPLAY).expect("a vector picture is drawn");
        assert_eq!((decoded.width, decoded.height), (100_000, 50_000));
        assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (256, 128));
    }

    /// Reading stops at the time limit, and when the caller cancels.
    #[test]
    fn decoding_stops_at_the_time_limit_and_when_cancelled() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = saved_picture(folder.path(), "Beach.png", "png", 300, 200);
        let no_time = Limits {
            time: Duration::ZERO,
            ..Limits::DISPLAY
        };
        assert_eq!(decode(&path, no_time).map(|_| ()), Err(Refusal::TimedOut));
        let cancelled = gio::Cancellable::new();
        cancelled.cancel();
        let file = gio::File::for_path(&path);
        assert_eq!(
            decode_to_fit(&file, 64, 64, Limits::DISPLAY, Some(&cancelled)).map(|_| ()),
            Err(Refusal::Cancelled)
        );
    }

    /// Packs LZW codes of growing width, least significant bit first, as
    /// GIF stores them.
    #[derive(Default)]
    struct Bits {
        bytes: Vec<u8>,
        value: u32,
        count: u32,
    }

    impl Bits {
        fn push(&mut self, code: u16, width: u32) {
            self.value |= u32::from(code) << self.count;
            self.count += width;
            while self.count >= 8 {
                self.bytes.push((self.value & 0xFF) as u8);
                self.value >>= 8;
                self.count -= 8;
            }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.count > 0 {
                self.bytes.push((self.value & 0xFF) as u8);
            }
            self.bytes
        }
    }

    /// The LZW data of `pixels` pixels of color 0, with a minimum code
    /// size of 2, as GIF encoders write it.
    fn lzw_of_one_color(pixels: usize) -> Vec<u8> {
        const CLEAR: u16 = 4;
        const END: u16 = 5;
        let mut bits = Bits::default();
        let mut width = 3;
        bits.push(CLEAR, width);
        // Runs of color 0: code 0 is one pixel, and each new code one
        // pixel longer than the last.
        let mut next = END + 1;
        let mut longest: Option<(u16, usize)> = None;
        let mut left = pixels;
        while left > 0 {
            let (code, length) = match longest {
                Some((code, length)) if length <= left => (code, length),
                _ => (0, 1),
            };
            bits.push(code, width);
            left -= length;
            if next < 4000 {
                longest = Some((next, length + 1));
                next += 1;
                if u32::from(next) > (1 << width) && width < 12 {
                    width += 1;
                }
            } else {
                bits.push(CLEAR, width);
                width = 3;
                next = END + 1;
                longest = None;
            }
        }
        bits.push(END, width);
        bits.finish()
    }

    /// A looping GIF of `frames` frames of `side` × `side` pixels.
    fn animated_gif(frames: usize, side: u16) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&side.to_le_bytes());
        gif.extend_from_slice(&side.to_le_bytes());
        // A global table of 4 colors.
        gif.extend_from_slice(&[0x81, 0, 0]);
        gif.extend_from_slice(&[0x33, 0x66, 0x99, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0x80, 0x80, 0x80]);
        gif.extend_from_slice(b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x00\x00\x00");
        let data = lzw_of_one_color(usize::from(side) * usize::from(side));
        for _ in 0..frames {
            gif.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0x02, 0x00, 0x00, 0x00]);
            gif.push(0x2C);
            gif.extend_from_slice(&[0, 0, 0, 0]);
            gif.extend_from_slice(&side.to_le_bytes());
            gif.extend_from_slice(&side.to_le_bytes());
            gif.push(0);
            gif.push(2);
            for block in data.chunks(255) {
                gif.push(u8::try_from(block.len()).expect("a sub-block"));
                gif.extend_from_slice(block);
            }
            gif.push(0);
        }
        gif.push(GIF_TRAILER);
        gif
    }

    /// Only an animated GIF's first frame is fed to the decoder, so a GIF
    /// of thousands of frames costs one frame, as Windows shows a GIF's
    /// first frame in its thumbnail.
    ///
    /// parity: VIEW-057
    #[test]
    fn an_animated_gif_is_decoded_to_its_first_frame_only() {
        let gif = animated_gif(3000, 500);
        let mut first_frame = FirstFrame::default();
        let (fed, done) = first_frame.take(&gif);
        assert!(done, "the first frame ends inside the file");
        assert!(fed.len() * 100 < gif.len(), "far less than the whole file is fed");

        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = folder.path().join("Party loop.gif");
        std::fs::write(&path, &gif).expect("the GIF");
        let started = Instant::now();
        let decoded = decode(&path, Limits::DISPLAY).expect("the first frame");
        assert!(started.elapsed() < Duration::from_secs(5), "one frame, not 3,000");
        assert_eq!((decoded.width, decoded.height), (500, 500));
        assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (256, 256));

        // A file fed in many small chunks finds the same end.
        let mut chunked = FirstFrame::default();
        let mut total = 0;
        for chunk in gif.chunks(7) {
            let (part, done) = chunked.take(chunk);
            total += part.len();
            if done {
                break;
            }
        }
        assert_eq!(total, fed.len());
        // Other formats pass through whole.
        let mut png = FirstFrame::default();
        assert_eq!(
            png.take(b"\x89PNG\r\n\x1a\nrest"),
            (&b"\x89PNG\r\n\x1a\nrest"[..], false)
        );
    }

    #[test]
    fn sizes_fit_within_their_box_keeping_their_shape() {
        assert_eq!(fit_within(1200, 600, (256, 256)), (256, 128));
        assert_eq!(fit_within(600, 1200, (220, 146)), (73, 146));
        assert_eq!(fit_within(30_000, 1, (256, 256)), (256, 1), "at least a pixel");
        assert_eq!(fit_within(1, 1, (256, 256)), (256, 256));
    }
}
