// SPDX-License-Identifier: AGPL-3.0-only
//! Cached thumbnails from the shared freedesktop thumbnail cache.
//!
//! Beyond the Python app, which shows no thumbnails. They are not part of a
//! listing ([`super::ATTRIBUTES`]): GIO hashes the URI and looks in the
//! cache directories for every row it is asked about. A view asks only for
//! the rows it binds, the way Dolphin (KIO `PreviewJob`) and Nautilus do:
//! `query_info_future(THUMBNAIL_ATTRIBUTES, ..., glib::Priority::LOW)` when
//! a row or tile is bound, dropping the future when it is unbound, then
//! [`thumbnail_path`] or [`cached_thumbnail`] on the result.
//!
//! [`thumbnail_file`] names where a thumbnail of a given size belongs in
//! the cache, as the freedesktop Thumbnail Managing Standard lays it out,
//! for the views to write the ones the desktop has not made.

use std::path::{Path, PathBuf};

use super::attributes::path_attribute;

/// Attributes to query for [`thumbnail_path`] and [`cached_thumbnail`].
pub const THUMBNAIL_ATTRIBUTES: &str = "thumbnail::path,thumbnail::is-valid,thumbnail::failed";

/// What the thumbnail cache holds for an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedThumbnail {
    /// A thumbnail made from the current version of the item.
    Valid(PathBuf),
    /// A thumbnailer tried and failed: no thumbnail is to be made again.
    Failed,
    /// None yet, or one made from an older version.
    Missing,
}

/// The thumbnail sizes of the standard, by their cache folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailFlavor {
    /// At most 128 pixels square (`normal`).
    Normal,
    /// At most 256 pixels square (`large`).
    Large,
}

impl ThumbnailFlavor {
    /// The smallest size that shows `pixels` device pixels sharply.
    pub fn for_pixels(pixels: i32) -> Self {
        if pixels <= Self::Normal.pixels() {
            Self::Normal
        } else {
            Self::Large
        }
    }

    /// The longest edge, in pixels.
    pub const fn pixels(self) -> i32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
        }
    }

    /// The cache folder's name, also the D-Bus thumbnailer's flavor.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
        }
    }
}

/// Where the thumbnail of the item at `uri` in `flavor` belongs under the
/// user's cache folder `cache_dir`: `thumbnails/<flavor>/<MD5 of the
/// URI>.png`.
pub fn thumbnail_file(cache_dir: &Path, uri: &str, flavor: ThumbnailFlavor) -> PathBuf {
    let hash = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri).unwrap_or_default();
    cache_dir
        .join("thumbnails")
        .join(flavor.as_str())
        .join(format!("{hash}.png"))
}

/// What the cache holds for the item `info` describes.
///
/// `info` must have been queried with [`THUMBNAIL_ATTRIBUTES`].
pub fn cached_thumbnail(info: &gio::FileInfo) -> CachedThumbnail {
    if let Some(path) = thumbnail_path(info) {
        return CachedThumbnail::Valid(path);
    }
    if info.boolean("thumbnail::failed") {
        CachedThumbnail::Failed
    } else {
        CachedThumbnail::Missing
    }
}

/// The item's cached thumbnail, when GIO reports one that is still valid
/// (made from the current version of the item).
///
/// `info` must have been queried with [`THUMBNAIL_ATTRIBUTES`].
pub fn thumbnail_path(info: &gio::FileInfo) -> Option<PathBuf> {
    if !info.boolean("thumbnail::is-valid") {
        return None;
    }
    path_attribute(info, "thumbnail::path")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the cached thumbnail was made from the current version of
    /// the item (`thumbnail::is-valid`).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Freshness {
        Current,
        Stale,
    }

    fn thumbnail_info(path: &str, freshness: Freshness) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string("thumbnail::path", path);
        info.set_attribute_boolean("thumbnail::is-valid", freshness == Freshness::Current);
        info
    }

    #[test]
    fn valid_thumbnail_path_is_kept_byte_for_byte() {
        let path = "/home/José/.cache/thumbnails/normal/a\\b.png";
        assert_eq!(
            thumbnail_path(&thumbnail_info(path, Freshness::Current)),
            Some(PathBuf::from(path))
        );
    }

    #[test]
    fn stale_thumbnail_is_ignored() {
        let info = thumbnail_info("/tmp/thumb.png", Freshness::Stale);
        assert_eq!(thumbnail_path(&info), None);
    }

    #[test]
    fn missing_thumbnail_is_none() {
        assert_eq!(thumbnail_path(&gio::FileInfo::new()), None);
        assert_eq!(cached_thumbnail(&gio::FileInfo::new()), CachedThumbnail::Missing);
    }

    /// parity: VIEW-057
    #[test]
    fn thumbnails_are_named_by_the_hash_of_their_uri() {
        let file = thumbnail_file(
            Path::new("/c"),
            "file:///home/jens/photos/me.png",
            ThumbnailFlavor::Large,
        );
        assert_eq!(
            file,
            Path::new("/c/thumbnails/large/c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
        assert_eq!(ThumbnailFlavor::for_pixels(96), ThumbnailFlavor::Normal);
        assert_eq!(ThumbnailFlavor::for_pixels(192), ThumbnailFlavor::Large);
        let failed = gio::FileInfo::new();
        failed.set_attribute_boolean("thumbnail::failed", true);
        assert_eq!(cached_thumbnail(&failed), CachedThumbnail::Failed);
    }
}
