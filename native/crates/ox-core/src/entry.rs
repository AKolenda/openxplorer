// SPDX-License-Identifier: AGPL-3.0-only
//! One row of a folder listing.
//!
//! Ports `desktop/entry_model.py` (`classify_entry`) and `entry_from_info`,
//! `enumerate_folder`, `inspect` and `verify_pin` in
//! `desktop/gio_backend.py`. Navigability is kept separate from mutability:
//! an SMB share in a server listing can be opened but not renamed or
//! trashed.
//!
//! Beyond the Python backend, every entry also carries its cached thumbnail,
//! its Trash origin and deletion date, the backend's rename/trash/delete/
//! write permissions and its GIO icon.
//!
//! - [`classify`]: folder, share and shortcut rules.
//! - [`info`]: `GFileInfo` to [`Entry`].
//! - [`type_label`](mod@type_label): Type column wording.
//! - [`enumerate`](mod@enumerate): listing a folder on a worker thread.
//! - [`error`]: why a folder or item could not be read.
//! - [`inspect`](mod@inspect): one item, and Quick access pins.

pub mod classify;
pub mod enumerate;
pub mod error;
pub mod info;
pub mod inspect;
pub mod type_label;

use std::path::PathBuf;

pub use classify::{classify_entry, Classification};
pub use enumerate::{enumerate, enumerate_blocking, EnumerationSummary, EnumerationTask, DEFAULT_BATCH_SIZE};
pub use error::EnumerateError;
pub use info::entry_for_uri;
pub use inspect::{inspect, pin_target, verify_pin, PinTarget};
pub use type_label::type_label;

/// Attributes requested for every listed item.
pub const ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::is-hidden,\
standard::is-symlink,standard::size,standard::content-type,standard::target-uri,standard::is-virtual,\
standard::icon,time::modified,thumbnail::path,thumbnail::is-valid,access::can-rename,access::can-trash,\
access::can-delete,access::can-write,trash::orig-path,trash::deletion-date";

/// What GIO says an item is (`standard::type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A folder.
    Directory,
    /// A regular file.
    File,
    /// A symbolic link that was not followed.
    Symlink,
    /// A device node, socket or pipe.
    Special,
    /// An SMB share in a server listing.
    Mountable,
    /// A network shortcut (for example a discovered server).
    Shortcut,
    /// The backend did not say.
    Unknown,
}

impl EntryKind {
    /// Maps a GIO file type.
    pub fn from_file_type(file_type: gio::FileType) -> Self {
        match file_type {
            gio::FileType::Directory => Self::Directory,
            gio::FileType::Regular => Self::File,
            gio::FileType::SymbolicLink => Self::Symlink,
            gio::FileType::Special => Self::Special,
            gio::FileType::Mountable => Self::Mountable,
            gio::FileType::Shortcut => Self::Shortcut,
            _ => Self::Unknown,
        }
    }

    /// The name the Python backend and web interface use (`directory`,
    /// `file`, `mountable`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::File => "file",
            Self::Symlink => "symlink",
            Self::Special => "special",
            Self::Mountable => "mountable",
            Self::Shortcut => "shortcut",
            Self::Unknown => "unknown",
        }
    }
}

/// One listed item. Plain data, so it can be built on a worker thread and
/// sent to the main thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The item's own URI.
    pub uri: String,
    /// Display name.
    pub name: String,
    /// What GIO says the item is.
    pub kind: EntryKind,
    /// Opens as a folder when activated.
    pub is_dir: bool,
    /// A network share or shortcut rather than a real item.
    pub is_virtual: bool,
    /// Can be copied, moved, renamed and trashed.
    pub can_operate: bool,
    /// Where a virtual folder navigates to.
    pub target_uri: Option<String>,
    /// `None` for folders and when the backend reports no size.
    pub size: Option<u64>,
    /// Human-readable type, for example "File folder" or "PNG image".
    pub type_label: String,
    /// MIME type, when known.
    pub content_type: Option<String>,
    /// Seconds since the Unix epoch; 0 when unknown.
    pub modified: u64,
    /// Hidden by name or by the backend.
    pub hidden: bool,
    /// A symbolic link (listed with its target's type).
    pub symlink: bool,
    /// A valid cached thumbnail in the shared freedesktop cache.
    pub thumbnail_path: Option<PathBuf>,
    /// For items in `trash:///`: where Restore puts them back.
    pub trash_orig_path: Option<PathBuf>,
    /// For items in `trash:///`: when they were deleted, in seconds since
    /// the Unix epoch.
    pub trash_deletion_date: Option<u64>,
    /// `access::can-rename`; `None` when the backend does not report it.
    pub can_rename: Option<bool>,
    /// `access::can-trash`; `None` when the backend does not report it.
    pub can_trash: Option<bool>,
    /// `access::can-delete`; `None` when the backend does not report it.
    pub can_delete: Option<bool>,
    /// `access::can-write`; `None` when the backend does not report it.
    pub can_write: Option<bool>,
    /// `standard::icon`, serialized with `g_icon_serialize` because a
    /// `GIcon` cannot cross threads. Use [`Entry::icon`].
    pub icon_data: Option<glib::Variant>,
}

impl Entry {
    /// The GIO icon for the item's type, for when there is no thumbnail and
    /// no OpenXplorer artwork for its type.
    pub fn icon(&self) -> Option<gio::Icon> {
        self.icon_data.as_ref().and_then(gio::Icon::deserialize)
    }

    /// The URI to open when the item is activated: the validated target of
    /// a share or shortcut, otherwise the item itself.
    pub fn navigation_uri(&self) -> &str {
        self.target_uri.as_deref().unwrap_or(&self.uri)
    }
}

/// Builds an entry from a queried `FileInfo`.
///
/// Reads only the attributes in [`ATTRIBUTES`] that the backend reported;
/// it never queries, stats or mounts anything itself.
pub fn entry_from_info(file: &gio::File, info: &gio::FileInfo) -> Entry {
    info::entry_for_file(file, info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn entries_can_cross_threads() {
        assert_send::<Entry>();
        assert_send::<Vec<Entry>>();
        assert_send::<EnumerateError>();
        assert_send::<EnumerationTask>();
    }

    #[test]
    fn kinds_use_the_python_names() {
        assert_eq!(
            EntryKind::from_file_type(gio::FileType::Mountable).as_str(),
            "mountable"
        );
        assert_eq!(
            EntryKind::from_file_type(gio::FileType::SymbolicLink).as_str(),
            "symlink"
        );
        assert_eq!(
            EntryKind::from_file_type(gio::FileType::Unknown),
            EntryKind::Unknown
        );
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
}
