// SPDX-License-Identifier: AGPL-3.0-only
//! Cached thumbnails from the shared freedesktop thumbnail cache.
//!
//! Beyond the Python app, which shows no thumbnails. They are not part of a
//! listing ([`super::ATTRIBUTES`]): GIO hashes the URI and looks in the
//! cache directories for every row it is asked about. A view asks only for
//! the rows it binds, the way Dolphin (KIO `PreviewJob`) and Nautilus do:
//! `query_info_future(THUMBNAIL_ATTRIBUTES, ..., glib::Priority::LOW)` when
//! a row or tile is bound, dropping the future when it is unbound, then
//! [`thumbnail_path`] on the result.

use std::path::PathBuf;

use super::attributes::path_attribute;

/// Attributes to query for [`thumbnail_path`].
pub const THUMBNAIL_ATTRIBUTES: &str = "thumbnail::path,thumbnail::is-valid";

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

    fn thumbnail_info(path: &str, is_valid: bool) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string("thumbnail::path", path);
        info.set_attribute_boolean("thumbnail::is-valid", is_valid);
        info
    }

    #[test]
    fn valid_thumbnail_path_is_kept_byte_for_byte() {
        let path = "/home/José/.cache/thumbnails/normal/a\\b.png";
        assert_eq!(
            thumbnail_path(&thumbnail_info(path, true)),
            Some(PathBuf::from(path))
        );
    }

    #[test]
    fn stale_thumbnail_is_ignored() {
        assert_eq!(thumbnail_path(&thumbnail_info("/tmp/thumb.png", false)), None);
    }

    #[test]
    fn missing_thumbnail_is_none() {
        assert_eq!(thumbnail_path(&gio::FileInfo::new()), None);
    }
}
