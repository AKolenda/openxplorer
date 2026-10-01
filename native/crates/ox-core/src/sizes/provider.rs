// SPDX-License-Identifier: AGPL-3.0-only
//! What a folder-size scan reads about each item, and the trait through
//! which it reads it.
//!
//! Ports the provider contract of `v2.0.0:desktop/folder_sizes.py`: the entry
//! dictionaries `LocalSizeProvider` and `GioSizeProvider` produce, and
//! their `inspect` and `children` methods. The contract lets the scan be
//! tested without GIO or a NAS.

use std::fmt;
use std::ops::ControlFlow;

use crate::entry::EntryError;
use crate::transfer::Cancellation;

/// What kind of item a [`SizeEntry`] is, as read without following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeEntryKind {
    /// A folder, or an SMB share in a server listing.
    Folder,
    /// A regular file.
    File,
    /// A symbolic link, never followed.
    Symlink,
    /// A pipe, socket, device or an item of unknown type, never opened.
    Other,
    /// Listed, but its metadata could not be read.
    Unreadable,
}

/// The device and inode of a local file, so hard links to one file are
/// counted once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    /// `st_dev`.
    pub device: u64,
    /// `st_ino`.
    pub inode: u64,
}

/// The metadata a folder-size scan reads about one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeEntry {
    /// The item's URI; a folder is scanned by it.
    pub uri: String,
    /// The item's file name; snapshot collections are recognised by it.
    pub name: String,
    /// What the item is.
    pub kind: SizeEntryKind,
    /// The logical size in bytes (`st_size`, `standard::size`), or `None`
    /// when the backend does not report one.
    pub size: Option<u64>,
    /// The filesystem the item is on (`st_dev`, `id::filesystem`), when
    /// known.
    pub filesystem: Option<String>,
    /// The file's device and inode, when the backend reports them.
    pub identity: Option<FileIdentity>,
    /// True when another filesystem is mounted on this folder.
    pub is_mount_point: bool,
}

impl SizeEntry {
    /// A listed item named `name` whose metadata could not be read.
    pub fn unreadable(name: impl Into<String>) -> Self {
        Self {
            uri: String::new(),
            name: name.into(),
            kind: SizeEntryKind::Unreadable,
            size: None,
            filesystem: None,
            identity: None,
            is_mount_point: false,
        }
    }
}

/// Reads item metadata for a folder-size scan, never file contents, and
/// never through a link.
pub trait SizeProvider: fmt::Debug {
    /// The item at `uri` itself.
    ///
    /// # Errors
    ///
    /// Why the item could not be read, including [`EntryError::NotMounted`]
    /// for a share that must be mounted first.
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<SizeEntry, EntryError>;

    /// Passes each item in the folder `folder_uri` to `visit`, until
    /// `visit` breaks or the folder ends. An item whose metadata cannot be
    /// read is passed as [`SizeEntryKind::Unreadable`].
    ///
    /// # Errors
    ///
    /// Why the folder could not be listed, also part-way through, and
    /// [`EntryError::Cancelled`] once `cancel` is cancelled.
    fn visit_children(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError>;
}
