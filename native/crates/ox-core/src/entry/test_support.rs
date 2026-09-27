// SPDX-License-Identifier: AGPL-3.0-only
//! `GFileInfo` fixtures shared by the entry tests.
//!
//! Real `GFileInfo` objects filled in by hand, without any filesystem
//! query, as `desktop/tests/test_gio_serialization.py` does with fakes.

/// The MIME type GIO reports for folders, shares and servers.
pub(super) const FOLDER_MIME_TYPE: &str = "inode/directory";

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
