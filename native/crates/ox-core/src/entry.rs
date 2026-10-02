// SPDX-License-Identifier: AGPL-3.0-only
//! One row of a folder listing.
//!
//! Ports `v2.0.0:desktop/entry_model.py` (`classify_entry`) and `entry_from_info`,
//! `enumerate_folder`, `inspect` and `verify_pin` in
//! `v2.0.0:desktop/gio_backend.py`. Navigability is kept separate from mutability:
//! an SMB share in a server listing can be opened but not renamed or
//! trashed.
//!
//! Beyond the Python backend, every entry also carries its Trash origin and
//! deletion date, the backend's rename/trash/delete/write permissions and
//! its GIO icon. Cached thumbnails are looked up separately, for the rows a
//! view shows (see [`THUMBNAIL_ATTRIBUTES`]).
//!
//! The submodules, in the order a listing uses them:
//!
//! - `enumerate`: reading a folder with GIO's asynchronous enumerator.
//! - `info`: `GFileInfo` to [`Entry`], with `attributes` reading the
//!   optional attributes and `classify` and `type_label` deciding what the
//!   row is and what its Type column says.
//! - `inspect`: one item on its own, and Quick access pins.
//! - `thumbnail`: the lazy thumbnail lookup.
//! - `error`: why a folder could not be listed, or an item inspected or
//!   pinned.

mod attributes;
mod classify;
mod enumerate;
mod error;
mod info;
mod inspect;
#[cfg(test)]
mod test_support;
mod thumbnail;
mod type_label;

pub use enumerate::enumerate_folder;
pub use error::EntryError;
pub use info::entry_from_info;
pub use inspect::{inspect, pin_target, verify_pin, PinTarget};
pub use thumbnail::{
    cached_thumbnail, thumbnail_file, thumbnail_path, CachedThumbnail, ThumbnailFlavor, THUMBNAIL_ATTRIBUTES,
};

use std::path::PathBuf;

/// Attributes requested for every listed item.
///
/// Thumbnails are deliberately left out. For `thumbnail::*` GIO hashes the
/// URI and looks in the thumbnail cache for every row, rows never scrolled
/// into view included, which made listing 20,000 local files about three
/// times slower. Views ask for [`THUMBNAIL_ATTRIBUTES`] when they show a row.
pub const ATTRIBUTES: &str = concat!(
    "standard::name,standard::display-name,standard::type,standard::is-hidden,",
    "standard::is-symlink,standard::size,standard::content-type,standard::target-uri,",
    "standard::is-virtual,standard::icon,time::modified,time::created,owner::user,unix::mode,",
    "access::can-rename,access::can-trash,access::can-delete,access::can-write,",
    "trash::orig-path,trash::deletion-date",
);

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

impl From<gio::FileType> for EntryKind {
    fn from(file_type: gio::FileType) -> Self {
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
}

impl EntryKind {
    /// Every kind, so that a stored [`EntryKind::as_str`] name can be read
    /// back. Kept next to `as_str`, which names each variant, so a new kind
    /// is added to both.
    pub(crate) const ALL: [Self; 7] = [
        Self::Directory,
        Self::File,
        Self::Symlink,
        Self::Special,
        Self::Mountable,
        Self::Shortcut,
        Self::Unknown,
    ];

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
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate GIO attribute or classifier result that views read on its own"
)]
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
    /// `time::modified` in seconds since the Unix epoch; `None` when the
    /// backend reports no time, or 0, which the Python app also showed as
    /// unknown.
    pub modified: Option<u64>,
    /// Hidden by name, by the folder's `.hidden` list or by the backend
    /// (`standard::is-hidden`).
    pub is_hidden: bool,
    /// A symbolic link, listed with its target's type
    /// (`standard::is-symlink`).
    pub is_symlink: bool,
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
    pub serialized_icon: Option<glib::Variant>,
    /// `time::created` in seconds since the Unix epoch, where the file
    /// system records it.
    pub created: Option<u64>,
    /// `owner::user`, the name of the item's owner.
    pub owner: Option<String>,
    /// `unix::mode`: the item's type and permission bits.
    pub unix_mode: Option<u32>,
}

impl Entry {
    /// The GIO icon for the item's type, for when there is no thumbnail and
    /// no Explorer-style artwork for its type.
    pub fn icon(&self) -> Option<gio::Icon> {
        self.serialized_icon.as_ref().and_then(gio::Icon::deserialize)
    }

    /// The URI to open when the item is activated: the validated target of
    /// a share or shortcut, otherwise the item itself.
    pub fn navigation_uri(&self) -> &str {
        self.target_uri.as_deref().unwrap_or(&self.uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn entries_and_errors_can_cross_threads() {
        assert_send::<Entry>();
        assert_send::<Vec<Entry>>();
        assert_send::<EntryError>();
    }

    #[test]
    fn kinds_use_the_python_names() {
        assert_eq!(EntryKind::from(gio::FileType::Mountable).as_str(), "mountable");
        assert_eq!(EntryKind::from(gio::FileType::SymbolicLink).as_str(), "symlink");
        assert_eq!(EntryKind::from(gio::FileType::Unknown), EntryKind::Unknown);
    }

    /// Regression: the listing asked GIO for thumbnails of every row, which
    /// made large folders about three times slower to list.
    #[test]
    fn listing_attributes_leave_thumbnails_to_the_lazy_lookup() {
        assert!(!ATTRIBUTES.contains("thumbnail::"), "{ATTRIBUTES}");
        assert!(THUMBNAIL_ATTRIBUTES.contains("thumbnail::path"));
        assert!(THUMBNAIL_ATTRIBUTES.contains("thumbnail::is-valid"));
    }

    #[test]
    fn listing_attributes_are_a_well_formed_list() {
        let names: Vec<&str> = ATTRIBUTES.split(',').collect();
        assert!(names.iter().all(|name| name.contains("::")), "{ATTRIBUTES}");
        assert!(names.contains(&"trash::orig-path"));
        assert!(names.contains(&"access::can-write"));
    }
}
