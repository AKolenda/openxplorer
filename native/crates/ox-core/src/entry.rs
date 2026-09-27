// SPDX-License-Identifier: AGPL-3.0-only
//! One row of a folder listing.
//!
//! Ports `desktop/entry_model.py` (`classify_entry`) and `entry_from_info`
//! and `enumerate_folder` in `desktop/gio_backend.py`. Navigability is kept
//! separate from mutability: an SMB share in a server listing can be opened
//! but not renamed or trashed.

use gio::prelude::*;

/// Attributes requested for every listed item.
pub const ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::is-hidden,\
standard::is-symlink,standard::size,standard::content-type,standard::target-uri,standard::is-virtual,\
standard::icon,time::modified,thumbnail::path,thumbnail::is-valid,access::can-rename,access::can-trash,\
access::can-delete,access::can-write,trash::orig-path,trash::deletion-date";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Special,
    /// An SMB share in a server listing.
    Mountable,
    /// A network shortcut (for example a discovered server).
    Shortcut,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub uri: String,
    pub name: String,
    pub kind: EntryKind,
    /// Opens as a folder when activated.
    pub is_dir: bool,
    /// A network share or shortcut rather than a real item.
    pub is_virtual: bool,
    /// Can be copied, moved, renamed and trashed.
    pub can_operate: bool,
    /// Where a virtual folder navigates to.
    pub target_uri: Option<String>,
    /// `None` for folders.
    pub size: Option<u64>,
    /// Human-readable type, for example "File folder" or "PNG image".
    pub type_label: String,
    pub content_type: Option<String>,
    /// Seconds since the Unix epoch; 0 when unknown.
    pub modified: u64,
    pub hidden: bool,
    pub symlink: bool,
    /// A valid cached thumbnail in the shared freedesktop cache.
    pub thumbnail_path: Option<std::path::PathBuf>,
    /// For items in `trash:///`: where Restore puts them back.
    pub trash_orig_path: Option<std::path::PathBuf>,
}

/// Builds an entry from a queried `FileInfo`.
pub fn entry_from_info(file: &gio::File, info: &gio::FileInfo) -> Entry {
    let kind = match info.file_type() {
        gio::FileType::Directory => EntryKind::Directory,
        gio::FileType::Regular => EntryKind::File,
        gio::FileType::SymbolicLink => EntryKind::Symlink,
        gio::FileType::Special => EntryKind::Special,
        gio::FileType::Mountable => EntryKind::Mountable,
        gio::FileType::Shortcut => EntryKind::Shortcut,
        _ => EntryKind::Unknown,
    };
    let content_type = info.content_type().map(|c| c.to_string());
    let is_dir = kind == EntryKind::Directory || content_type.as_deref() == Some("inode/directory");
    let type_label = if is_dir {
        "File folder".to_string()
    } else {
        content_type
            .as_deref()
            .map(|c| gio::content_type_get_description(c).to_string())
            .unwrap_or_else(|| "File".into())
    };
    Entry {
        uri: file.uri().to_string(),
        name: info.display_name().to_string(),
        kind,
        is_dir,
        is_virtual: matches!(kind, EntryKind::Mountable | EntryKind::Shortcut),
        can_operate: matches!(kind, EntryKind::Directory | EntryKind::File | EntryKind::Symlink),
        target_uri: info
            .attribute_string("standard::target-uri")
            .map(|s| s.to_string()),
        size: if is_dir {
            None
        } else {
            Some(info.size().max(0) as u64)
        },
        type_label,
        content_type,
        modified: info.attribute_uint64("time::modified"),
        hidden: info.is_hidden(),
        symlink: info.is_symlink(),
        thumbnail_path: None,
        trash_orig_path: None,
    }
}
