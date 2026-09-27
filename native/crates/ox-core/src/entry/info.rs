// SPDX-License-Identifier: AGPL-3.0-only
//! Builds an [`Entry`] from a queried `GFileInfo`.
//!
//! Ports `entry_from_info` in `desktop/gio_backend.py`, extended with the
//! thumbnail, Trash, access and icon attributes in [`super::ATTRIBUTES`].
//! Only attributes that are present are read: since GLib 2.76 the typed
//! getters (`g_file_info_get_size` and friends) log a critical warning for a
//! missing attribute, and backends such as smb-browse omit many of them.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use gio::prelude::*;

use super::classify::classify_entry;
use super::type_label::type_label;
use super::{Entry, EntryKind};
use crate::location::split_location;

/// Builds an entry for the item at `uri` from its queried `info`.
///
/// Takes the URI as text so the result does not depend on how a GVfs URI
/// mapper would rewrite it. When the backend reports no name at all, the
/// last segment of the URI is used.
pub fn entry_for_uri(uri: &str, info: &gio::FileInfo) -> Entry {
    build_entry(uri, info, || last_uri_segment(uri))
}

/// Builds an entry for `file` from its queried `info`; see
/// [`super::entry_from_info`]. When the backend reports no name at all,
/// the file's base name is used, as `gfile.get_basename()` is in Python.
pub(super) fn entry_for_file(file: &gio::File, info: &gio::FileInfo) -> Entry {
    let uri = file.uri();
    build_entry(&uri, info, || {
        file.basename().map(|name| name.to_string_lossy().into_owned())
    })
}

fn build_entry(uri: &str, info: &gio::FileInfo, fallback_name: impl FnOnce() -> Option<String>) -> Entry {
    let kind = file_kind(info);
    let content_type = string_attribute(info, "standard::content-type");
    let backend_target = string_attribute(info, "standard::target-uri");
    let is_virtual = info.boolean("standard::is-virtual");
    let class = classify_entry(
        kind,
        uri,
        content_type.as_deref(),
        backend_target.as_deref(),
        is_virtual,
    );
    let type_label = type_label(class.folder_type, content_type.as_deref());
    // A folder's size is not its contents' size, and a missing size is
    // unknown rather than zero bytes.
    let size = if class.is_dir || !info.has_attribute("standard::size") {
        None
    } else {
        Some(info.attribute_uint64("standard::size"))
    };
    let modified = info.attribute_uint64("time::modified");
    let thumbnail_is_valid = info.boolean("thumbnail::is-valid");
    let thumbnail_path = if thumbnail_is_valid {
        path_attribute(info, "thumbnail::path")
    } else {
        None
    };
    Entry {
        uri: uri.to_string(),
        name: display_name(uri, info, fallback_name),
        kind,
        is_dir: class.is_dir,
        is_virtual: class.is_virtual,
        can_operate: class.can_operate,
        target_uri: class.target_uri,
        size,
        type_label,
        content_type,
        modified,
        hidden: info.boolean("standard::is-hidden"),
        symlink: info.boolean("standard::is-symlink"),
        thumbnail_path,
        trash_orig_path: path_attribute(info, "trash::orig-path"),
        trash_deletion_date: deletion_date(info),
        can_rename: optional_boolean(info, "access::can-rename"),
        can_trash: optional_boolean(info, "access::can-trash"),
        can_delete: optional_boolean(info, "access::can-delete"),
        can_write: optional_boolean(info, "access::can-write"),
        icon_data: serialized_icon(info),
    }
}

fn file_kind(info: &gio::FileInfo) -> EntryKind {
    if info.has_attribute("standard::type") {
        EntryKind::from_file_type(info.file_type())
    } else {
        EntryKind::Unknown
    }
}

/// `display-name`, then `name`, then `fallback_name`, then the URI itself,
/// as in the Python backend. Empty values are skipped.
fn display_name(uri: &str, info: &gio::FileInfo, fallback_name: impl FnOnce() -> Option<String>) -> String {
    if let Some(name) = string_attribute(info, "standard::display-name") {
        return name;
    }
    if info.has_attribute("standard::name") {
        let name = info.name();
        if !name.as_os_str().is_empty() {
            return name.to_string_lossy().into_owned();
        }
    }
    fallback_name()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| uri.to_string())
}

/// The decoded last path segment of `uri`, if it has one.
fn last_uri_segment(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    let segment = parts.path.rsplit('/').find(|segment| !segment.is_empty())?;
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    Some(decoded.into_owned())
}

fn string_attribute(info: &gio::FileInfo, attribute: &str) -> Option<String> {
    info.attribute_string(attribute)
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
}

/// `Some(flag)` when the backend reported the attribute, `None` when it did
/// not (callers then let the operation itself decide).
fn optional_boolean(info: &gio::FileInfo, attribute: &str) -> Option<bool> {
    info.has_attribute(attribute).then(|| info.boolean(attribute))
}

/// Reads a path attribute (`thumbnail::path`, `trash::orig-path`).
///
/// These are byte strings and need not be UTF-8, so they are read through
/// GIO's escaped text form and unescaped back to the exact bytes.
fn path_attribute(info: &gio::FileInfo, attribute: &str) -> Option<PathBuf> {
    let bytes = match info.attribute_type(attribute) {
        gio::FileAttributeType::ByteString => {
            let escaped = info.attribute_as_string(attribute)?;
            unescape_byte_string(&escaped)
        }
        gio::FileAttributeType::String => info.attribute_string(attribute)?.as_bytes().to_vec(),
        _ => return None,
    };
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// Reverses GIO's `escape_byte_string`: every byte outside printable ASCII,
/// and every backslash, is written as `\xNN`.
fn unescape_byte_string(escaped: &str) -> Vec<u8> {
    let bytes = escaped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(byte) = escaped_byte_at(bytes, index) {
            out.push(byte);
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

/// The byte encoded by a `\xNN` sequence starting at `index`, if any.
fn escaped_byte_at(bytes: &[u8], index: usize) -> Option<u8> {
    let sequence = bytes.get(index..index + 4)?;
    if sequence[0] != b'\\' || sequence[1] != b'x' {
        return None;
    }
    let high = hex_value(sequence[2])?;
    let low = hex_value(sequence[3])?;
    Some(high * 16 + low)
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// `trash::deletion-date` as seconds since the Unix epoch. GIO stores it
/// as local time without a zone, the way the `.trashinfo` file does.
fn deletion_date(info: &gio::FileInfo) -> Option<u64> {
    if !info.has_attribute("trash::deletion-date") {
        return None;
    }
    let date = info.deletion_date()?;
    u64::try_from(date.to_unix()).ok()
}

/// `standard::icon` in a thread-safe form; see [`Entry::icon`].
fn serialized_icon(info: &gio::FileInfo) -> Option<glib::Variant> {
    if !info.has_attribute("standard::icon") {
        return None;
    }
    info.icon().and_then(|icon| icon.serialize())
}

#[cfg(test)]
mod tests {
    //! Real `GFileInfo` objects filled in by hand, without any filesystem
    //! query, as `desktop/tests/test_gio_serialization.py` does with fakes.

    use super::*;

    fn info(kind: gio::FileType, name: &str, content_type: Option<&str>) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_file_type(kind);
        info.set_display_name(name);
        if let Some(content_type) = content_type {
            info.set_content_type(content_type);
        }
        info
    }

    fn share(target: Option<&str>) -> gio::FileInfo {
        let info = info(gio::FileType::Mountable, "work", Some("inode/directory"));
        if let Some(target) = target {
            info.set_attribute_string("standard::target-uri", target);
        }
        info
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_mountable_network_share_has_directory_icon_flag_and_unknown_size
    #[test]
    fn mountable_network_share_has_directory_flag_and_unknown_size() {
        let e = entry_for_uri("smb://nas/work", &share(Some("smb://nas/work")));
        assert!(e.is_dir);
        assert_eq!(e.kind, EntryKind::Mountable);
        assert_eq!(e.type_label, "Network share");
        assert_eq!(e.size, None);
        assert_eq!(e.modified, 0);
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_gvfs_mountable_browse_uri_and_target_remain_distinct
    #[test]
    fn gvfs_mountable_browse_uri_and_target_remain_distinct() {
        // GVfs smburi.c names a server-browser child `._share`; its
        // standard::target-uri points to the actual share mount.
        let e = entry_for_uri("smb://nas/._work", &share(Some("smb://nas/work")));
        assert_eq!(e.uri, "smb://nas/._work");
        assert_eq!(e.target_uri.as_deref(), Some("smb://nas/work"));
        assert_eq!(e.navigation_uri(), "smb://nas/work");
        assert!(e.is_dir);
        assert!(!e.can_operate);
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_real_directory_metadata
    #[test]
    fn real_directory_metadata() {
        let info = info(gio::FileType::Directory, "Design", Some("inode/directory"));
        info.set_attribute_uint64("standard::size", 8192);
        info.set_attribute_uint64("time::modified", 12345);
        let e = entry_for_uri("smb://nas/work/Design", &info);
        assert!(e.is_dir);
        assert_eq!(e.size, None);
        assert_eq!(e.modified, 12345);
        assert_eq!(e.type_label, "File folder");
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_real_zero_byte_file_remains_zero_bytes
    #[test]
    fn real_zero_byte_file_remains_zero_bytes() {
        let info = info(gio::FileType::Regular, "README", Some("text/plain"));
        info.set_size(0);
        let e = entry_for_uri("smb://nas/work/README", &info);
        assert!(!e.is_dir);
        assert_eq!(e.size, Some(0));
        assert_eq!(e.type_label, "Text document");
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_missing_file_size_does_not_claim_zero
    #[test]
    fn missing_file_size_does_not_claim_zero() {
        let info = info(gio::FileType::Regular, "README", Some("text/plain"));
        assert_eq!(entry_for_uri("smb://nas/work/README", &info).size, None);
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_server_shortcut_resolves_to_target_server
    #[test]
    fn server_shortcut_resolves_to_target_server() {
        let info = info(gio::FileType::Shortcut, "alpha", Some("inode/directory"));
        info.set_attribute_string("standard::target-uri", "smb://ALPHA/");
        let e = entry_for_uri("smb://group/alpha", &info);
        assert!(e.is_dir);
        assert_eq!(e.target_uri.as_deref(), Some("smb://alpha/"));
        assert!(!e.can_operate);
    }

    /// Ported from desktop/tests/test_gio_serialization.py::test_serialization_requests_no_extra_filesystem_queries
    #[test]
    fn serialization_needs_only_the_uri_and_type() {
        // Only a URI and hand-filled metadata: no query, stat or mount.
        let e = entry_for_uri("smb://nas/work", &share(None));
        assert!(e.is_dir);
        assert_eq!(e.name, "work");
        assert_eq!(e.can_rename, None);
        assert_eq!(e.icon_data, None);
    }

    #[test]
    fn share_without_any_metadata_is_still_a_share() {
        let info = gio::FileInfo::new();
        info.set_file_type(gio::FileType::Mountable);
        let e = entry_for_uri("smb://nas/work", &info);
        assert!(e.is_dir);
        assert_eq!(e.name, "work");
        assert_eq!(e.type_label, "Network share");
    }

    #[test]
    fn name_falls_back_to_the_file_base_name() {
        let bare = gio::FileInfo::new();
        let file = gio::File::for_path("/tmp/Plan 1.txt");
        assert_eq!(entry_for_file(&file, &bare).name, "Plan 1.txt");
    }

    #[test]
    fn name_falls_back_from_display_name_to_name_to_uri() {
        let info = gio::FileInfo::new();
        info.set_file_type(gio::FileType::Regular);
        info.set_name("raw-name.txt");
        assert_eq!(
            entry_for_uri("file:///tmp/raw-name.txt", &info).name,
            "raw-name.txt"
        );
        let bare = gio::FileInfo::new();
        assert_eq!(
            entry_for_uri("file:///tmp/Read%20me.txt", &bare).name,
            "Read me.txt"
        );
        assert_eq!(entry_for_uri("smb://nas/", &bare).name, "smb://nas/");
    }

    #[test]
    fn valid_thumbnail_path_is_kept_byte_for_byte() {
        let info = info(gio::FileType::Regular, "photo.png", Some("image/png"));
        info.set_attribute_byte_string("thumbnail::path", "/home/José/.cache/thumbnails/normal/a\\b.png");
        info.set_attribute_boolean("thumbnail::is-valid", true);
        let e = entry_for_uri("file:///home/Jos%C3%A9/photo.png", &info);
        assert_eq!(
            e.thumbnail_path,
            Some(PathBuf::from("/home/José/.cache/thumbnails/normal/a\\b.png"))
        );
    }

    #[test]
    fn stale_thumbnail_is_ignored() {
        let info = info(gio::FileType::Regular, "photo.png", Some("image/png"));
        info.set_attribute_byte_string("thumbnail::path", "/tmp/thumb.png");
        info.set_attribute_boolean("thumbnail::is-valid", false);
        assert_eq!(entry_for_uri("file:///tmp/photo.png", &info).thumbnail_path, None);
    }

    #[test]
    fn trash_items_keep_their_origin_and_deletion_date() {
        let info = info(gio::FileType::Regular, "notes.txt", Some("text/plain"));
        info.set_attribute_byte_string("trash::orig-path", "/home/demo/notes.txt");
        info.set_attribute_string("trash::deletion-date", "2026-09-26T10:11:12");
        let e = entry_for_uri("trash:///notes.txt", &info);
        assert_eq!(e.trash_orig_path, Some(PathBuf::from("/home/demo/notes.txt")));
        let expected = glib::DateTime::from_iso8601("2026-09-26T10:11:12", Some(&glib::TimeZone::local()))
            .expect("valid ISO 8601 date")
            .to_unix();
        assert_eq!(e.trash_deletion_date, u64::try_from(expected).ok());
    }

    #[test]
    fn access_flags_distinguish_false_from_unknown() {
        let info = info(gio::FileType::Regular, "a.txt", Some("text/plain"));
        info.set_attribute_boolean("access::can-rename", false);
        info.set_attribute_boolean("access::can-trash", true);
        let e = entry_for_uri("file:///tmp/a.txt", &info);
        assert_eq!(e.can_rename, Some(false));
        assert_eq!(e.can_trash, Some(true));
        assert_eq!(e.can_delete, None);
        assert_eq!(e.can_write, None);
    }

    #[test]
    fn icon_survives_the_thread_safe_form() {
        let info = info(gio::FileType::Regular, "a.txt", Some("text/plain"));
        info.set_icon(&gio::ThemedIcon::new("text-plain"));
        let e = entry_for_uri("file:///tmp/a.txt", &info);
        let icon = e.icon().expect("the themed icon deserializes");
        let themed = icon
            .downcast::<gio::ThemedIcon>()
            .expect("a themed icon stays themed");
        assert!(themed.names().iter().any(|name| name == "text-plain"));
    }

    #[test]
    fn unescapes_gio_byte_strings() {
        assert_eq!(
            unescape_byte_string(r"/a\x5cb/\xc3\xa9\xff"),
            b"/a\\b/\xc3\xa9\xff".to_vec()
        );
        assert_eq!(unescape_byte_string(r"\x4"), b"\\x4".to_vec());
        assert_eq!(unescape_byte_string(r"\xzz"), b"\\xzz".to_vec());
    }
}
