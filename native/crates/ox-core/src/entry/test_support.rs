// SPDX-License-Identifier: AGPL-3.0-only
//! `GFileInfo` fixtures shared by the entry tests.
//!
//! Real `GFileInfo` objects filled in by hand, without any filesystem
//! query, as `v2.0.0:desktop/tests/test_gio_serialization.py` does with fakes, and
//! entries built from them for a URI given as text.

use super::info::build_entry;
use super::Entry;
use crate::location::split_location;

/// The classifier's folder MIME type, so fixtures and tests name the one
/// constant it compares against.
pub(super) use super::classify::FOLDER_MIME_TYPE;

/// Info for an item of `kind` shown as `display_name`, with `content_type`
/// when the backend reports one.
pub(super) fn file_info(
    kind: gio::FileType,
    display_name: &str,
    content_type: Option<&str>,
) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    info.set_file_type(kind);
    info.set_display_name(display_name);
    if let Some(content_type) = content_type {
        info.set_content_type(content_type);
    }
    info
}

/// `info` with the backend's `standard::target-uri` set to `target`.
pub(super) fn with_target(info: gio::FileInfo, target: &str) -> gio::FileInfo {
    info.set_attribute_string("standard::target-uri", target);
    info
}

/// An SMB share named `work` as gvfsd-smb-browse lists it.
pub(super) fn smb_share_info() -> gio::FileInfo {
    file_info(gio::FileType::Mountable, "work", Some(FOLDER_MIME_TYPE))
}

/// [`super::entry_from_info`] for the item at `uri`, given as text.
///
/// A `gio::File` would let GIO's virtual file system rewrite fixture URIs
/// such as `smb://nas/._work`, so the URI is used exactly as written. When
/// the backend reports no name at all, the last segment of the URI is used.
pub(super) fn entry_for_uri(uri: &str, info: &gio::FileInfo) -> Entry {
    build_entry(uri, info, || last_uri_segment(uri))
}

/// The decoded last path segment of `uri`, if it has one.
fn last_uri_segment(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    let segment = parts.path.rsplit('/').find(|segment| !segment.is_empty())?;
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    Some(decoded.into_owned())
}
