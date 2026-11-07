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

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{thumbnail_file, ThumbnailFlavor};

use crate::picture_guard::{self, Limits, Refusal};

/// How much of a picture is read at a time.
const READ_CHUNK: usize = 64 * 1024;

/// What the thumbnail is made of.
#[derive(Debug, Clone)]
pub(super) struct Source {
    /// The picture's URI, as the thumbnail names it.
    pub uri: String,
    /// Its modification time in seconds, which keeps the thumbnail valid.
    pub modified: u64,
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
    /// The picture was read and refused ([`picture_guard`]): its header
    /// asks for more pixels than the limit, it is damaged or cut short, or
    /// it took too long. Another thumbnailer would read the same picture,
    /// so none is asked, and the item keeps its icon.
    Refused,
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
) -> Result<PathBuf, Failure> {
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
            Refusal::TooLarge { .. } | Refusal::Damaged | Refusal::TimedOut => Failure::Refused,
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
) -> Result<PathBuf, Failure> {
    let target = thumbnail_file(cache_dir, &source.uri, flavor);
    let Some(folder) = target.parent() else {
        return Err(Failure::Unwritable);
    };
    // A thumbnail of a thumbnail would grow the cache from itself.
    if inside_cache(cache_dir, &source.uri) {
        return Err(Failure::Unwritable);
    }
    if create_folder(folder).is_none() {
        return Err(Failure::Unwritable);
    }
    let temporary = folder.join(format!(".{}.openxplorer.tmp", glib::random_int()));
    let modified = source.modified.to_string();
    let options = [
        ("tEXt::Thumb::URI", source.uri.as_str()),
        ("tEXt::Thumb::MTime", modified.as_str()),
        ("tEXt::Software", "OpenXplorer"),
    ];
    let saved = pixbuf.savev(&temporary, "png", &options).is_ok()
        && fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).is_ok()
        && fs::rename(&temporary, &target).is_ok();
    if !saved {
        let _ = fs::remove_file(&temporary);
        return Err(Failure::Unwritable);
    }
    Ok(target)
}

/// The thumbnail of `uri` in `flavor` under `cache_dir`, if one exists
/// that was made from the version modified at `modified`. Blocks.
pub(super) fn find_thumbnail(
    cache_dir: &Path,
    uri: &str,
    modified: u64,
    flavor: ThumbnailFlavor,
) -> Option<PathBuf> {
    let path = thumbnail_file(cache_dir, uri, flavor);
    // Read within a thumbnail's limits: another program may have left a
    // huge or damaged file there.
    let file = gio::File::for_path(&path);
    let thumbnail = picture_guard::decode_to_fit(&file, i32::MAX, i32::MAX, Limits::THUMBNAIL_FILE, None)
        .ok()?
        .pixbuf;
    let is_current = thumbnail.option("tEXt::Thumb::URI").as_deref() == Some(uri)
        && thumbnail.option("tEXt::Thumb::MTime").as_deref() == Some(modified.to_string().as_str());
    is_current.then_some(path)
}

/// Whether `uri` names an item of the thumbnail cache under `cache_dir`: a
/// thumbnail of a thumbnail would grow the cache from itself, so such an
/// item keeps its icon, and neither the app nor a program of the desktop's
/// is asked for a picture of it.
pub(super) fn inside_cache(cache_dir: &Path, uri: &str) -> bool {
    glib::filename_to_uri(cache_dir.join("thumbnails"), None)
        .is_ok_and(|cache| uri.starts_with(cache.as_str()))
}

/// The folder `folder` and its parents, enterable by its owner only, as
/// the standard asks of the cache folders.
fn create_folder(folder: &Path) -> Option<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
        .ok()
}

/// The PNG in `bytes`, decoded to fit `edge` pixels square as
/// [`scaled_picture`] decodes a file: a desktop thumbnailer's picture,
/// which the app read into memory itself and checked the size of (see
/// [`super::thumbnailers`]). Only gdk-pixbuf's PNG loader reads it.
pub(super) fn scaled_png(bytes: &[u8], edge: i32) -> Option<Pixbuf> {
    let loader = scaling(PixbufLoader::with_type("png").ok()?, edge);
    for chunk in bytes.chunks(READ_CHUNK) {
        if loader.write(chunk).is_err() {
            let _ = loader.close();
            return None;
        }
    }
    finish(&loader)
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

/// `loader`, made to decode its picture to fit `edge` pixels square, or
/// at its own size when smaller.
fn scaling(loader: PixbufLoader, edge: i32) -> PixbufLoader {
    loader.connect_size_prepared(move |loader, width, height| {
        if width > edge || height > edge {
            let (width, height) = fit(width, height, edge);
            loader.set_size(width, height);
        }
    });
    loader
}

/// The picture `loader` was given, turned upright.
fn finish(loader: &PixbufLoader) -> Option<Pixbuf> {
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    Some(pixbuf.apply_embedded_orientation().unwrap_or(pixbuf))
}

/// `width` × `height` scaled to fit `edge` square, keeping the aspect
/// ratio and at least one pixel.
fn fit(width: i32, height: i32, edge: i32) -> (i32, i32) {
    let longest = width.max(height).max(1);
    let scale = |side: i32| (i64::from(side) * i64::from(edge) / i64::from(longest)).max(1);
    let to_i32 = |side: i64| i32::try_from(side).unwrap_or(edge);
    (to_i32(scale(width)), to_i32(scale(height)))
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
        };

        let written = write_thumbnail(&cache, &source, ThumbnailFlavor::Normal, &gio::Cancellable::new())
            .expect("a thumbnail");

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
        let mode = fs::metadata(&written).expect("the file").permissions().mode();
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
        };
        assert_eq!(
            save_thumbnail(&cache, &inside, ThumbnailFlavor::Normal, &thumbnail),
            Err(Failure::Unwritable)
        );

        let cancelled = gio::Cancellable::new();
        cancelled.cancel();
        let other_cache = folder.path().join("cancelled-cache");
        assert_eq!(
            write_thumbnail(&other_cache, &source, ThumbnailFlavor::Normal, &cancelled),
            Err(Failure::Cancelled)
        );
        assert!(!other_cache.exists(), "a cancelled read leaves no cache file");
    }
}
