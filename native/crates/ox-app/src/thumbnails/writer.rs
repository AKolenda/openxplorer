// SPDX-License-Identifier: AGPL-3.0-only
//! Making a picture's thumbnail with gdk-pixbuf when no thumbnailer
//! service runs, written to the shared cache as the freedesktop Thumbnail
//! Managing Standard asks: a PNG no larger than its flavor (and never
//! larger than the picture), carrying `Thumb::URI` and `Thumb::MTime`,
//! written to a temporary file and renamed into place, readable by its
//! owner only, in folders only its owner may enter. Other desktop
//! programs, Files and Dolphin included, then find it too.

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{thumbnail_file, ThumbnailFlavor};

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

/// Makes the thumbnail of `source` in `flavor` under the cache folder
/// `cache_dir`; `None` when the picture cannot be read or the thumbnail
/// not written. Blocks: run it off the main thread.
pub(super) fn write_thumbnail(
    cache_dir: &Path,
    source: &Source,
    flavor: ThumbnailFlavor,
    cancellation: &gio::Cancellable,
) -> Option<PathBuf> {
    let target = thumbnail_file(cache_dir, &source.uri, flavor);
    let folder = target.parent()?;
    // A thumbnail of a thumbnail would grow the cache from itself.
    if source.uri.starts_with(
        &glib::filename_to_uri(cache_dir.join("thumbnails"), None)
            .ok()?
            .to_string(),
    ) {
        return None;
    }
    let pixbuf = scaled_picture(&source.uri, flavor.pixels(), cancellation)?;
    if cancellation.is_cancelled() {
        return None;
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
        .ok()?;
    let temporary = folder.join(format!(".{}.openxplorer.tmp", glib::random_int()));
    let modified = source.modified.to_string();
    let options = [
        ("tEXt::Thumb::URI", source.uri.as_str()),
        ("tEXt::Thumb::MTime", modified.as_str()),
        ("tEXt::Software", "OpenXplorer"),
    ];
    let saved = pixbuf.savev(&temporary, "png", &options).is_ok()
        && !cancellation.is_cancelled()
        && fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).is_ok()
        && fs::rename(&temporary, &target).is_ok();
    if !saved {
        let _ = fs::remove_file(&temporary);
        return None;
    }
    Some(target)
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
    let thumbnail = Pixbuf::from_file(&path).ok()?;
    let is_current = thumbnail.option("tEXt::Thumb::URI").as_deref() == Some(uri)
        && thumbnail.option("tEXt::Thumb::MTime").as_deref() == Some(modified.to_string().as_str());
    is_current.then_some(path)
}

/// The picture at `uri`, turned upright and decoded to fit `edge` pixels
/// square, or at its own size when smaller: a large photo is never held
/// in memory at full size.
fn scaled_picture(uri: &str, edge: i32, cancellation: &gio::Cancellable) -> Option<Pixbuf> {
    let stream = gio::File::for_uri(uri).read(Some(cancellation)).ok()?;
    let loader = PixbufLoader::new();
    loader.connect_size_prepared(move |loader, width, height| {
        if width > edge || height > edge {
            let (width, height) = fit(width, height, edge);
            loader.set_size(width, height);
        }
    });
    let mut buffer = vec![0; READ_CHUNK];
    loop {
        let Ok(read) = stream.read(&mut buffer, Some(cancellation)) else {
            let _ = loader.close();
            return None;
        };
        if read == 0 {
            break;
        }
        if loader.write(&buffer[..read]).is_err() {
            let _ = loader.close();
            return None;
        }
    }
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
    /// keys that keep it valid, and is no larger than its flavor.
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

        let cancelled = gio::Cancellable::new();
        cancelled.cancel();
        let other_cache = folder.path().join("cancelled-cache");
        assert!(write_thumbnail(&other_cache, &source, ThumbnailFlavor::Normal, &cancelled).is_none());
        assert!(!other_cache.exists(), "a cancelled read leaves no cache file");
    }
}
