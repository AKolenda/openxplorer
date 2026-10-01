// SPDX-License-Identifier: AGPL-3.0-only
//! Builds an [`Entry`] from a queried `GFileInfo`.
//!
//! Ports `entry_from_info` in `v2.0.0:desktop/gio_backend.py`, extended with the
//! Trash, access and icon attributes in [`super::ATTRIBUTES`]. It never
//! queries, stats or mounts anything itself: only the attributes the
//! backend already reported are read.

use gio::prelude::*;

use super::attributes::{optional_boolean, path_attribute, string_attribute, time_attribute};
use super::classify::{classify_entry, Classification, ItemMetadata};
use super::type_label::type_label;
use super::{Entry, EntryKind};

/// Builds an entry for `file` from its queried `info`.
///
/// Reads only the attributes in [`super::ATTRIBUTES`] that the backend
/// reported. When the backend reports no name at all, the file's base name
/// is used, as `gfile.get_basename()` is in Python.
pub fn entry_from_info(file: &gio::File, info: &gio::FileInfo) -> Entry {
    let uri = file.uri();
    let base_name = || file.basename().map(|name| name.to_string_lossy().into_owned());
    build_entry(&uri, info, base_name)
}

/// The body of [`entry_from_info`], shared with the test fixtures' entries
/// for a URI given as text; `fallback_name` is only called when the backend
/// reported no name.
pub(super) fn build_entry(
    uri: &str,
    info: &gio::FileInfo,
    fallback_name: impl FnOnce() -> Option<String>,
) -> Entry {
    let kind = file_kind(info);
    let content_type = string_attribute(info, "standard::content-type");
    let classification = classify_info(uri, info, kind, content_type.as_deref());
    let type_label = type_label(classification.folder_type, content_type.as_deref());
    let is_dir = classification.is_dir();
    // A folder's own size is not the size of its contents, so none is shown.
    let size = if is_dir { None } else { reported_size(info) };
    Entry {
        uri: uri.to_owned(),
        name: display_name(uri, info, fallback_name),
        kind,
        is_dir,
        is_virtual: classification.is_virtual,
        can_operate: classification.can_operate,
        target_uri: classification.target_uri,
        size,
        type_label,
        content_type,
        modified: time_attribute(info, "time::modified"),
        is_hidden: info.boolean("standard::is-hidden"),
        is_symlink: info.boolean("standard::is-symlink"),
        trash_orig_path: path_attribute(info, "trash::orig-path"),
        trash_deletion_date: deletion_date(info),
        can_rename: optional_boolean(info, "access::can-rename"),
        can_trash: optional_boolean(info, "access::can-trash"),
        can_delete: optional_boolean(info, "access::can-delete"),
        can_write: optional_boolean(info, "access::can-write"),
        serialized_icon: serialized_icon(info),
    }
}

/// Classifies the item at `uri` from its kind, content type and the
/// backend's target and virtual flag.
fn classify_info(
    uri: &str,
    info: &gio::FileInfo,
    kind: EntryKind,
    content_type: Option<&str>,
) -> Classification {
    let backend_target = string_attribute(info, "standard::target-uri");
    classify_entry(&ItemMetadata {
        kind,
        uri,
        content_type,
        target_uri: backend_target.as_deref(),
        has_virtual_flag: info.boolean("standard::is-virtual"),
    })
}

/// `standard::type`, or [`EntryKind::Unknown`] when the backend did not
/// report it.
fn file_kind(info: &gio::FileInfo) -> EntryKind {
    if info.has_attribute("standard::type") {
        EntryKind::from(info.file_type())
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
        .unwrap_or_else(|| uri.to_owned())
}

/// `standard::size`; `None` when the backend reports none, which is unknown
/// rather than zero bytes.
fn reported_size(info: &gio::FileInfo) -> Option<u64> {
    if !info.has_attribute("standard::size") {
        return None;
    }
    Some(info.attribute_uint64("standard::size"))
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
    info.icon()?.serialize()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::entry::test_support::{
        entry_for_uri, file_info, smb_share_info, with_target, FOLDER_MIME_TYPE,
    };
    use crate::format::date_text;

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_mountable_network_share_has_directory_icon_flag_and_unknown_size`
    ///
    /// parity: VIEW-002, NET-003
    #[test]
    fn mountable_network_share_has_directory_flag_and_unknown_size() {
        let info = with_target(smb_share_info(), "smb://nas/work");
        let share = entry_for_uri("smb://nas/work", &info);
        assert!(share.is_dir);
        assert_eq!(share.kind, EntryKind::Mountable);
        assert_eq!(share.type_label, "Network share");
        assert_eq!(share.size, None);
        assert_eq!(share.modified, None);
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_gvfs_mountable_browse_uri_and_target_remain_distinct`
    ///
    /// parity: NET-003
    #[test]
    fn gvfs_mountable_browse_uri_and_target_remain_distinct() {
        // GVfs smburi.c names a server-browser child `._share`; its
        // standard::target-uri points to the actual share mount.
        let info = with_target(smb_share_info(), "smb://nas/work");
        let share = entry_for_uri("smb://nas/._work", &info);
        assert_eq!(share.uri, "smb://nas/._work");
        assert_eq!(share.target_uri.as_deref(), Some("smb://nas/work"));
        assert_eq!(share.navigation_uri(), "smb://nas/work");
        assert!(share.is_dir);
        assert!(!share.can_operate);
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_real_directory_metadata`
    ///
    /// parity: VIEW-002
    #[test]
    fn real_directory_shows_no_size_but_keeps_its_date() {
        let info = file_info(gio::FileType::Directory, "Design", Some(FOLDER_MIME_TYPE));
        info.set_attribute_uint64("standard::size", 8192);
        info.set_attribute_uint64("time::modified", 12345);
        let folder = entry_for_uri("smb://nas/work/Design", &info);
        assert!(folder.is_dir);
        assert_eq!(folder.size, None);
        assert_eq!(folder.modified, Some(12345));
        assert_eq!(folder.type_label, "File folder");
    }

    /// The Python app sent 0 for a missing time, and the web interface
    /// showed `—` for every 0 (`dateText` in `v2.0.0:desktop/ui/app.js`).
    ///
    /// parity: VIEW-001, VIEW-002
    #[test]
    fn modification_time_of_zero_is_shown_as_unknown() {
        let info = file_info(gio::FileType::Regular, "old.txt", Some("text/plain"));
        info.set_attribute_uint64("time::modified", 0);
        let file = entry_for_uri("file:///tmp/old.txt", &info);
        assert_eq!(file.modified, None);
        assert_eq!(date_text(file.modified), "—");
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_real_zero_byte_file_remains_zero_bytes`
    ///
    /// parity: VIEW-002
    #[test]
    fn real_zero_byte_file_remains_zero_bytes() {
        let info = file_info(gio::FileType::Regular, "README", Some("text/plain"));
        info.set_size(0);
        let file = entry_for_uri("smb://nas/work/README", &info);
        assert!(!file.is_dir);
        assert_eq!(file.size, Some(0));
        assert_eq!(file.type_label, "Text document");
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_missing_file_size_does_not_claim_zero`
    ///
    /// parity: VIEW-002
    #[test]
    fn missing_file_size_does_not_claim_zero() {
        let info = file_info(gio::FileType::Regular, "README", Some("text/plain"));
        assert_eq!(entry_for_uri("smb://nas/work/README", &info).size, None);
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_server_shortcut_resolves_to_target_server`
    ///
    /// parity: NET-003
    #[test]
    fn server_shortcut_resolves_to_target_server() {
        let shortcut = file_info(gio::FileType::Shortcut, "alpha", Some(FOLDER_MIME_TYPE));
        let info = with_target(shortcut, "smb://ALPHA/");
        let server = entry_for_uri("smb://group/alpha", &info);
        assert!(server.is_dir);
        assert_eq!(server.target_uri.as_deref(), Some("smb://alpha/"));
        assert!(!server.can_operate);
    }

    /// Ported from `v2.0.0:desktop/tests/test_gio_serialization.py::GioSerializationTests::test_serialization_requests_no_extra_filesystem_queries`
    ///
    /// parity: VIEW-002
    #[test]
    fn serialization_needs_only_the_uri_and_type() {
        // Only a URI and hand-filled metadata: no query, stat or mount.
        let share = entry_for_uri("smb://nas/work", &smb_share_info());
        assert!(share.is_dir);
        assert_eq!(share.name, "work");
        assert_eq!(share.can_rename, None);
        assert_eq!(share.serialized_icon, None);
    }

    /// parity: NET-003
    #[test]
    fn share_without_any_metadata_is_still_a_share() {
        let info = gio::FileInfo::new();
        info.set_file_type(gio::FileType::Mountable);
        let share = entry_for_uri("smb://nas/work", &info);
        assert!(share.is_dir);
        assert_eq!(share.name, "work");
        assert_eq!(share.type_label, "Network share");
    }

    #[test]
    fn entry_from_info_uses_the_file_uri() {
        let info = gio::FileInfo::new();
        info.set_file_type(gio::FileType::Directory);
        let file = gio::File::for_path("/tmp/ox-entry-test");
        let entry = entry_from_info(&file, &info);
        assert_eq!(entry.uri, "file:///tmp/ox-entry-test");
        assert!(entry.is_dir);
        assert_eq!(entry.navigation_uri(), "file:///tmp/ox-entry-test");
    }

    /// parity: VIEW-002
    #[test]
    fn name_falls_back_to_the_file_base_name() {
        let bare = gio::FileInfo::new();
        let file = gio::File::for_path("/tmp/Plan 1.txt");
        assert_eq!(entry_from_info(&file, &bare).name, "Plan 1.txt");
    }

    /// parity: VIEW-002
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
    fn trash_items_keep_their_origin_and_deletion_date() {
        let info = file_info(gio::FileType::Regular, "notes.txt", Some("text/plain"));
        info.set_attribute_byte_string("trash::orig-path", "/home/demo/notes.txt");
        info.set_attribute_string("trash::deletion-date", "2026-09-26T10:11:12");
        let item = entry_for_uri("trash:///notes.txt", &info);
        assert_eq!(item.trash_orig_path, Some(PathBuf::from("/home/demo/notes.txt")));
        let local_zone = glib::TimeZone::local();
        let expected = glib::DateTime::from_iso8601("2026-09-26T10:11:12", Some(&local_zone))
            .expect("valid ISO 8601 date")
            .to_unix();
        assert_eq!(item.trash_deletion_date, u64::try_from(expected).ok());
    }

    #[test]
    fn trash_origin_is_kept_byte_for_byte() {
        let info = file_info(gio::FileType::Regular, "a\\b.txt", Some("text/plain"));
        info.set_attribute_byte_string("trash::orig-path", "/home/José/a\\b.txt");
        let item = entry_for_uri("trash:///a%5Cb.txt", &info);
        assert_eq!(item.trash_orig_path, Some(PathBuf::from("/home/José/a\\b.txt")));
    }

    #[test]
    fn access_flags_distinguish_false_from_unknown() {
        let info = file_info(gio::FileType::Regular, "a.txt", Some("text/plain"));
        info.set_attribute_boolean("access::can-rename", false);
        info.set_attribute_boolean("access::can-trash", true);
        let file = entry_for_uri("file:///tmp/a.txt", &info);
        assert_eq!(file.can_rename, Some(false));
        assert_eq!(file.can_trash, Some(true));
        assert_eq!(file.can_delete, None);
        assert_eq!(file.can_write, None);
    }

    #[test]
    fn icon_survives_the_thread_safe_form() {
        let info = file_info(gio::FileType::Regular, "a.txt", Some("text/plain"));
        info.set_icon(&gio::ThemedIcon::new("text-plain"));
        let file = entry_for_uri("file:///tmp/a.txt", &info);
        let icon = file.icon().expect("the themed icon deserializes");
        let themed = icon
            .downcast::<gio::ThemedIcon>()
            .expect("a themed icon stays themed");
        assert!(themed.names().iter().any(|name| name == "text-plain"));
    }
}
