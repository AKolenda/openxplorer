// SPDX-License-Identifier: AGPL-3.0-only
//! Making a picture's thumbnail with gdk-pixbuf when no thumbnailer
//! service runs, written to the shared cache as the freedesktop Thumbnail
//! Managing Standard asks: a PNG no larger than its flavor (and never
//! larger than the picture), carrying `Thumb::URI` and `Thumb::MTime`,
//! written to a temporary file and renamed into place, readable by its
//! owner only, in folders only its owner may enter. Other desktop
//! programs, Files and Dolphin included, then find it too.
//!
//! A desktop thumbnailer's picture ([`super::thumbnailers`]) is written
//! the same way, sharing [`save_thumbnail`]: such a program writes a plain
//! PNG, so the app adds the keys that keep a thumbnail valid.
//!
//! A failure says whether any thumbnailer of the desktop's is worth
//! asking at all ([`Failure`]): a picture the app cannot read leaves room
//! for one, while a picture it read and the cache refused does not.

use std::path::{Path, PathBuf};

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{thumbnail_file, ThumbnailFlavor};
use ox_core::thumbnail_cache::{self, Version};

use crate::picture_guard::{self, Limits, Refusal};

/// What the thumbnail is made of.
#[derive(Debug, Clone)]
pub(super) struct Source {
    /// The picture's URI, as the thumbnail names it.
    pub uri: String,
    /// Its modification time in seconds, which keeps the thumbnail valid.
    pub modified: u64,
    /// Its size in bytes, when known, which keeps the thumbnail valid too.
    pub size: Option<u64>,
    /// Its content type, written with the thumbnail.
    pub mime_type: Option<String>,
    /// The edge, in device pixels, the thumbnail is shown at: the picture
    /// handed back ([`Made::picture`]) is no larger, so the memory of the
    /// previews shown keeps no more pixels than the screen shows.
    pub shown: i32,
}

impl Source {
    /// The version of the file the thumbnail is made from.
    pub(super) fn version(&self) -> Version {
        Version {
            uri: self.uri.clone(),
            modified: self.modified,
            size: self.size,
        }
    }
}

/// A thumbnail written to the cache, and its picture, which the view
/// shows without reading the file back.
#[derive(Debug, Clone)]
pub(super) struct Made {
    /// Where it was written.
    #[cfg_attr(not(test), expect(dead_code, reason = "the tests look where it went"))]
    pub path: PathBuf,
    /// Its picture, no larger than it is shown ([`Source::shown`]).
    pub picture: gtk::gdk::Texture,
}

/// Why no thumbnail was written, which tells the caller whether another
/// thumbnailer is worth asking: a picture of a type gdk-pixbuf does not
/// read may still have one of the desktop's, but the cache refuses the
/// same thumbnail whatever makes it, and a dropped request has no reader
/// left to show it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    /// The picture could not be read or decoded: the app has no loader
    /// for its type, or its bytes are no picture.
    Unreadable,
    /// The picture was read and refused ([`picture_guard`]): it is
    /// damaged or cut short. Another thumbnailer would read the same
    /// picture, so none is asked, and the item keeps its icon.
    Refused,
    /// The picture's header asks for more pixels than the app decodes
    /// itself ([`picture_guard::MAX_PIXELS`]), as a large scan or panorama
    /// may: only a desktop thumbnailer whose memory is limited is asked to
    /// make it, in a process of its own.
    TooLarge,
    /// Reading or decoding the picture took longer than its time limit,
    /// which a busy disk or a slow mount may cause: no other thumbnailer
    /// is asked now, and it may be tried again later.
    TimedOut,
    /// The picture was read, but the cache would not take its thumbnail:
    /// the item is a thumbnail of the cache itself, whose pictures of
    /// pictures would grow it from within, or the folder could not be
    /// written. Another thumbnailer fails the same way, so none is asked.
    Unwritable,
    /// The request was dropped, or its reader cancelled, before the
    /// picture was read or the thumbnail written.
    Cancelled,
}

/// Makes the thumbnail of `source` in `flavor` under the cache folder
/// `cache_dir`. Blocks: run it off the main thread.
pub(super) fn write_thumbnail(
    cache_dir: &Path,
    source: &Source,
    flavor: ThumbnailFlavor,
    cancellation: &gio::Cancellable,
) -> Result<Made, Failure> {
    super::trace::event("generate", &source.uri);
    let pixbuf = scaled_picture(&source.uri, flavor.pixels(), cancellation).map_err(|refusal| {
        // A read that stopped because it was cancelled is no reason to
        // fall back to another thumbnailer, nor is a picture the guard
        // refused.
        if cancellation.is_cancelled() {
            return Failure::Cancelled;
        }
        match refusal {
            Refusal::Unreadable => Failure::Unreadable,
            Refusal::Cancelled => Failure::Cancelled,
            Refusal::TooLarge { .. } => Failure::TooLarge,
            Refusal::Damaged => Failure::Refused,
            Refusal::TimedOut => Failure::TimedOut,
        }
    })?;
    if cancellation.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    save_thumbnail(cache_dir, source, flavor, &pixbuf)
}

/// Writes the picture `pixbuf` as the thumbnail of `source` in `flavor`
/// under the cache folder `cache_dir`, as the Thumbnail Managing Standard
/// asks. Blocks.
///
/// [`write_thumbnail`] hands over the picture it decoded, and
/// [`super::thumbnailers`] the one a desktop thumbnailer wrote: either way
/// the cache gets the same keys, so Files and Dolphin find the thumbnail
/// valid too.
pub(super) fn save_thumbnail(
    cache_dir: &Path,
    source: &Source,
    flavor: ThumbnailFlavor,
    pixbuf: &Pixbuf,
) -> Result<Made, Failure> {
    let target = thumbnail_file(cache_dir, &source.uri, flavor);
    // A thumbnail of a thumbnail would grow the cache from itself.
    if inside_cache(cache_dir, &source.uri) {
        return Err(Failure::Unwritable);
    }
    let version = source.version();
    let options: Vec<(String, String)> = version
        .keys(source.mime_type.as_deref())
        .into_iter()
        .map(|(key, value)| (format!("tEXt::{key}"), value))
        .collect();
    let options: Vec<(&str, &str)> = options
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    // Encoded here, then written privately from the start and renamed into
    // place ([`thumbnail_cache::write_entry`]): another program never
    // reads half a thumbnail, nor one readable by others.
    let png = pixbuf
        .save_to_bufferv("png", &options)
        .map_err(|_| Failure::Unwritable)?;
    thumbnail_cache::write_entry(&target, &png).map_err(|_| Failure::Unwritable)?;
    super::trace::event("written", &source.uri);
    Ok(Made {
        path: target,
        picture: texture_to_fit(pixbuf, source.shown),
    })
}

/// `picture` as a texture no larger than `edge` pixels square, shrunk with
/// gdk-pixbuf's bilinear filter when larger: a picture the app decoded
/// itself, so no untrusted bytes are read here.
fn texture_to_fit(picture: &Pixbuf, edge: i32) -> gtk::gdk::Texture {
    let longest = picture.width().max(picture.height());
    if edge <= 0 || longest <= edge {
        return gtk::gdk::Texture::for_pixbuf(picture);
    }
    let side = |side: i32| {
        i32::try_from(i64::from(side) * i64::from(edge) / i64::from(longest))
            .unwrap_or(1)
            .max(1)
    };
    picture
        .scale_simple(
            side(picture.width()),
            side(picture.height()),
            gtk::gdk_pixbuf::InterpType::Bilinear,
        )
        .map_or_else(
            || gtk::gdk::Texture::for_pixbuf(picture),
            |shrunk| gtk::gdk::Texture::for_pixbuf(&shrunk),
        )
}

/// Whether `uri` names an item of the thumbnail cache under `cache_dir`: a
/// thumbnail of a thumbnail would grow the cache from itself, so such an
/// item keeps its icon, and neither the app nor a program of the desktop's
/// is asked for a picture of it.
pub(super) fn inside_cache(cache_dir: &Path, uri: &str) -> bool {
    glib::filename_to_uri(cache_dir.join("thumbnails"), None)
        .is_ok_and(|cache| uri.starts_with(cache.as_str()))
}

/// The PNG in `bytes`, decoded to fit `edge` pixels square as
/// [`scaled_picture`] decodes a file: a desktop thumbnailer's picture,
/// which the app read into memory itself and checked the size of (see
/// [`super::thumbnailers`]). It was made from a file that may be damaged, so it is
/// decoded by the picture decoder process, never in the app.
pub(super) fn scaled_png(bytes: &[u8], edge: i32) -> Option<Pixbuf> {
    picture_guard::decode_bytes_to_fit(bytes.to_vec(), edge, edge, Limits::DISPLAY, None)
        .ok()
        .map(|decoded| decoded.pixbuf)
}

/// The picture at `uri`, turned upright and decoded to fit `edge` pixels
/// square, or at its own size when smaller: a large photo is never held
/// in memory at full size, and a picture over the pixel limit, damaged,
/// slow or animated is refused or cut to its first frame
/// ([`picture_guard`]).
fn scaled_picture(uri: &str, edge: i32, cancellation: &gio::Cancellable) -> Result<Pixbuf, Refusal> {
    let file = gio::File::for_uri(uri);
    picture_guard::decode_to_fit(&file, edge, edge, Limits::DISPLAY, Some(cancellation))
        .map(|decoded| decoded.pixbuf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture's thumbnail lands where the standard puts it, with the
    /// keys that keep it valid, and is no larger than its flavor; a
    /// picture of the cache's own folder and a cancelled read are refused
    /// as such, which is what tells the caller not to ask another
    /// thumbnailer.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_picture_gets_a_standard_thumbnail() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let picture = folder.path().join("photo.png");
        let pixels = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 200).expect("a picture");
        pixels.fill(0x3366_99ff);
        pixels.savev(&picture, "png", &[]).expect("the picture");
        let cache = folder.path().join("cache");
        let source = Source {
            uri: gio::File::for_path(&picture).uri().to_string(),
            modified: 1_700_000_000,
            size: Some(4096),
            mime_type: Some("image/png".to_owned()),
            shown: 96,
        };

        let made = write_thumbnail(&cache, &source, ThumbnailFlavor::Normal, &gio::Cancellable::new())
            .expect("a thumbnail");
        let written = made.path;
        // The file is as large as its flavor; the picture handed back is
        // as large as it is shown.
        assert_eq!((made.picture.width(), made.picture.height()), (96, 48));

        assert_eq!(
            written,
            thumbnail_file(&cache, &source.uri, ThumbnailFlavor::Normal)
        );
        let thumbnail = Pixbuf::from_file(&written).expect("a PNG");
        assert_eq!((thumbnail.width(), thumbnail.height()), (128, 64));
        assert_eq!(
            thumbnail.option("tEXt::Thumb::URI").as_deref(),
            Some(source.uri.as_str())
        );
        assert_eq!(
            thumbnail.option("tEXt::Thumb::MTime").as_deref(),
            Some("1700000000")
        );
        assert_eq!(thumbnail.option("tEXt::Thumb::Size").as_deref(), Some("4096"));
        assert_eq!(
            thumbnail.option("tEXt::Thumb::Mimetype").as_deref(),
            Some("image/png")
        );
        // And the app's own reading of the keys, which needs no decoder,
        // finds it current.
        let keys = thumbnail_cache::png_keys(&std::fs::read(&written).expect("read")).expect("keys");
        assert!(keys.describe(&source.version()));
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&written).expect("the file").permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);

        // A picture of the cache's own folder is read, but its thumbnail
        // is refused: that is what tells the caller no other thumbnailer
        // can help.
        let inner = cache
            .join("thumbnails")
            .join(ThumbnailFlavor::Normal.as_str())
            .join("x.png");
        let inside = Source {
            uri: gio::File::for_path(&inner).uri().to_string(),
            modified: 1_700_000_000,
            size: None,
            mime_type: None,
            shown: 128,
        };
        assert_eq!(
            save_thumbnail(&cache, &inside, ThumbnailFlavor::Normal, &thumbnail).map(|made| made.path),
            Err(Failure::Unwritable)
        );

        let cancelled = gio::Cancellable::new();
        cancelled.cancel();
        let other_cache = folder.path().join("cancelled-cache");
        assert_eq!(
            write_thumbnail(&other_cache, &source, ThumbnailFlavor::Normal, &cancelled).map(|made| made.path),
            Err(Failure::Cancelled)
        );
        assert!(!other_cache.exists(), "a cancelled read leaves no cache file");
    }
}
