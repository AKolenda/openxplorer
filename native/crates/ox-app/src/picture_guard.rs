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
//! once the time limit has passed. Every failure is a [`Refusal`], and
//! callers show the file's type icon instead, as Windows Explorer does
//! for a picture it cannot preview.
//!
//! Decoding blocks: run it off the main thread.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
use gtk::gio;
use gtk::prelude::*;

/// The most pixels a raster picture may have to be decoded for display:
/// 64 megapixels (8192 × 8192), more than any camera's photo, 256 MB
/// at four bytes a pixel.
pub(crate) const MAX_PIXELS: u64 = 8192 * 8192;

/// The most pixels of a file in the shared thumbnail cache, whose largest
/// standard size is 1024 × 1024 (xx-large).
pub(crate) const MAX_THUMBNAIL_FILE_PIXELS: u64 = 1024 * 1024;

/// How long one picture may take to read and decode.
pub(crate) const TIME_LIMIT: Duration = Duration::from_secs(10);

/// How much of a picture is read at a time.
const READ_CHUNK: usize = 64 * 1024;

/// How much a picture may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The most pixels of a raster picture.
    pub max_pixels: u64,
    /// How long reading and decoding may take.
    pub time: Duration,
}

impl Limits {
    /// The limits of a picture shown in the app.
    pub(crate) const DISPLAY: Self = Self {
        max_pixels: MAX_PIXELS,
        time: TIME_LIMIT,
    };

    /// The limits of a file read from the thumbnail cache.
    pub(crate) const THUMBNAIL_FILE: Self = Self {
        max_pixels: MAX_THUMBNAIL_FILE_PIXELS,
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
    let loader = PixbufLoader::new();
    let header = Rc::new(Header::default());
    watch_header(&loader, &header, (width.max(1), height.max(1)), limits.max_pixels);
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
        let Ok(read) = stream.read(&mut buffer, cancellable) else {
            break Err(refusal_for(cancellable));
        };
        if read == 0 {
            break Ok(());
        }
        let (chunk, done) = first_frame.take(&buffer[..read]);
        ending.follow(chunk);
        let written = loader.write(chunk).and_then(|()| {
            if done {
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
/// tells the loader to load nothing when a raster picture has more than
/// `max_pixels`, else to decode to fit `fit` (vector pictures always).
fn watch_header(loader: &PixbufLoader, header: &Rc<Header>, fit: (i32, i32), max_pixels: u64) {
    let header = Rc::clone(header);
    loader.connect_size_prepared(move |loader, width, height| {
        header.size.set(Some((width, height)));
        let is_vector = loader.format().is_some_and(|format| format.is_scalable());
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

/// The end of what was fed to the loader: gdk-pixbuf draws a PNG cut
/// short without an error, so a PNG must end with its IEND chunk, as
/// Windows shows no thumbnail of a PNG cut short. (A JPEG cut short is
/// drawn as far as it goes, grey below, as Windows draws it too.)
#[derive(Debug, Default)]
struct Ending {
    /// The first bytes fed, up to a PNG signature's length.
    start: Vec<u8>,
    /// The last 12 bytes fed: IEND's length, type and CRC.
    tail: Vec<u8>,
}

impl Ending {
    fn follow(&mut self, chunk: &[u8]) {
        let wanted = PNG_SIGNATURE.len().saturating_sub(self.start.len());
        self.start.extend_from_slice(&chunk[..wanted.min(chunk.len())]);
        self.tail.extend_from_slice(chunk);
        let extra = self.tail.len().saturating_sub(12);
        self.tail.drain(..extra);
    }

    /// False for a PNG that does not end with IEND.
    fn is_complete(&self) -> bool {
        self.start != PNG_SIGNATURE || self.tail.get(4..8) == Some(b"IEND".as_slice())
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
    /// Inside a colour table, `left` bytes left; an image's local table
    /// is followed by its LZW code size, the global one by a block.
    ColourTable {
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
                    colour_table_after(self.descriptor_flags.take().unwrap_or(0), false)
                }
            }
            GifState::ColourTable { left, local } => {
                if left > 1 {
                    GifState::ColourTable {
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
                    colour_table_after(byte, true)
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

/// The colour table a packed field with `flags` announces (its top bit
/// set, 3 × 2^(n+1) bytes for its low three bits n), else the state that
/// follows without one: an image's LZW code size (`local`) or a block.
fn colour_table_after(flags: u8, local: bool) -> GifState {
    if flags & 0x80 != 0 {
        GifState::ColourTable {
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

    /// A picture `width` × `height` filled with one colour, saved as
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

    /// The LZW data of `pixels` pixels of colour 0, with a minimum code
    /// size of 2, as GIF encoders write it.
    fn lzw_of_one_colour(pixels: usize) -> Vec<u8> {
        const CLEAR: u16 = 4;
        const END: u16 = 5;
        let mut bits = Bits::default();
        let mut width = 3;
        bits.push(CLEAR, width);
        // Runs of colour 0: code 0 is one pixel, and each new code one
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
        // A global table of 4 colours.
        gif.extend_from_slice(&[0x81, 0, 0]);
        gif.extend_from_slice(&[0x33, 0x66, 0x99, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0x80, 0x80, 0x80]);
        gif.extend_from_slice(b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x00\x00\x00");
        let data = lzw_of_one_colour(usize::from(side) * usize::from(side));
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
