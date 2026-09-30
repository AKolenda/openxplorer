// SPDX-License-Identifier: AGPL-3.0-only
//! The incoming item beside the one it would replace, for the
//! name-conflict dialog (OPS-028).
//!
//! Dolphin's and Windows' conflict dialogs show both items' size and
//! date, and say when the files are the same. [`compare_dates`] reads both
//! without following links; for the one item the dialog shows,
//! [`compare`] also compares two files of the same size up to
//! [`MAX_COMPARED_BYTES`] byte for byte, on a worker thread, when both
//! are on a local disk rather than a network share seen through `GVfs`.
//! "Replace older" uses only the dates to replace files whose existing
//! copy is older. An item copied into its own folder is the existing item
//! itself, which Replace may not overwrite.

use std::fs;
use std::path::{Path, PathBuf};

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use ox_core::format::{date_time_text, pretty_bytes};

/// The largest files compared byte for byte.
const MAX_COMPARED_BYTES: u64 = 16 * 1024 * 1024;

/// What the dialog shows about one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Side {
    /// A folder rather than a file.
    pub(super) is_folder: bool,
    /// The size of a file.
    pub(super) size: Option<u64>,
    /// Last modified, in seconds since the Unix epoch.
    pub(super) modified: Option<u64>,
}

impl Side {
    /// "Folder", or the file's size, then its date.
    fn text(self) -> String {
        let kind = if self.is_folder {
            "Folder".to_owned()
        } else {
            self.size.map_or_else(|| "File".to_owned(), pretty_bytes)
        };
        format!("{kind} · Modified {}", date_time_text(self.modified))
    }
}

/// The incoming item and the existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Comparison {
    pub(super) incoming: Side,
    pub(super) existing: Side,
    /// Both are files with the same bytes.
    pub(super) identical: bool,
    /// The incoming item is the existing one: a copy into its own
    /// folder, which may not replace it.
    pub(super) same_item: bool,
}

impl Comparison {
    /// The dialog's lines: each side, then how they differ.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Incoming: {}", self.incoming.text()),
            format!("Existing: {}", self.existing.text()),
        ];
        let verdict = if self.same_item {
            Some("This is the item itself, so it cannot replace itself.")
        } else if self.identical {
            Some("The two files are identical.")
        } else if self.existing_is_older() {
            Some("The existing item is older.")
        } else if self.existing.modified > self.incoming.modified {
            Some("The existing item is newer.")
        } else {
            None
        };
        lines.extend(verdict.map(str::to_owned));
        lines
    }

    /// True when both are files and the existing one was modified before
    /// the incoming one, so "Replace older" replaces it.
    pub(super) fn existing_is_older(&self) -> bool {
        let both_files = !self.incoming.is_folder && !self.existing.is_folder && !self.same_item;
        match (self.existing.modified, self.incoming.modified) {
            (Some(existing), Some(incoming)) => both_files && existing < incoming,
            _ => false,
        }
    }
}

/// `file`'s side, read without following a link.
async fn side_of(file: &gio::File) -> Option<Side> {
    let attributes = "standard::type,standard::size,time::modified";
    let info = file
        .query_info_future(
            attributes,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    let is_folder = info.file_type() == gio::FileType::Directory;
    let size = (!is_folder).then(|| u64::try_from(info.size()).unwrap_or_default());
    let modified = info
        .modification_date_time()
        .and_then(|time| u64::try_from(time.to_unix()).ok());
    Some(Side {
        is_folder,
        size,
        modified,
    })
}

/// True when the local files at `first` and `second` hold the same bytes.
fn same_bytes(first: &Path, second: &Path) -> bool {
    match (fs::read(first), fs::read(second)) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

/// True when `path` is on a local disk: not a `GVfs` share mounted through
/// FUSE, nor any other file system GIO calls remote.
fn is_on_local_disk(path: &Path) -> bool {
    let fuse_folders = [
        glib::user_runtime_dir().join("gvfs"),
        glib::home_dir().join(".gvfs"),
    ];
    if fuse_folders.iter().any(|folder| path.starts_with(folder)) {
        return false;
    }
    gio::File::for_path(path)
        .query_filesystem_info(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE, gio::Cancellable::NONE)
        .is_ok_and(|info| !info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE))
}

/// The incoming item at `uri` beside the item of its name in
/// `destination_folder`, by type, size and date only; `None` when either
/// cannot be read.
pub(super) async fn compare_dates(uri: &str, destination_folder: &str) -> Option<Comparison> {
    let incoming_file = gio::File::for_uri(uri);
    let existing_file = gio::File::for_uri(destination_folder).child(incoming_file.basename()?);
    Some(Comparison {
        incoming: side_of(&incoming_file).await?,
        existing: side_of(&existing_file).await?,
        identical: false,
        same_item: incoming_file.equal(&existing_file),
    })
}

/// [`compare_dates`], and whether two files of the same size on a local
/// disk hold the same bytes.
pub(super) async fn compare(uri: &str, destination_folder: &str) -> Option<Comparison> {
    let mut comparison = compare_dates(uri, destination_folder).await?;
    let (incoming, existing) = (comparison.incoming, comparison.existing);
    let comparable = !comparison.same_item
        && !incoming.is_folder
        && !existing.is_folder
        && incoming.size == existing.size
        && incoming.size.is_some_and(|size| size <= MAX_COMPARED_BYTES);
    let incoming_file = gio::File::for_uri(uri);
    let existing_path: Option<PathBuf> = incoming_file
        .basename()
        .and_then(|name| gio::File::for_uri(destination_folder).child(name).path());
    if let (true, Some(first), Some(second)) = (comparable, incoming_file.path(), existing_path) {
        comparison.identical = gio::spawn_blocking(move || {
            is_on_local_disk(&first) && is_on_local_disk(&second) && same_bytes(&first, &second)
        })
        .await
        .unwrap_or(false);
    }
    Some(comparison)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64, modified: u64) -> Side {
        Side {
            is_folder: false,
            size: Some(size),
            modified: Some(modified),
        }
    }

    /// parity: OPS-028
    #[test]
    fn the_comparison_says_which_side_is_older_or_that_they_are_identical() {
        let older = Comparison {
            incoming: file(2048, 200),
            existing: file(1024, 100),
            identical: false,
            same_item: false,
        };
        let same = Comparison {
            incoming: file(10, 100),
            existing: file(10, 100),
            identical: true,
            same_item: false,
        };
        let folder = Side {
            is_folder: true,
            size: None,
            modified: Some(50),
        };
        let folders = Comparison {
            incoming: folder,
            existing: folder,
            identical: false,
            same_item: false,
        };

        assert!(older.lines()[0].starts_with("Incoming: 2.0 KB · Modified "));
        assert_eq!(older.lines()[2], "The existing item is older.");
        assert!(older.existing_is_older());
        assert_eq!(same.lines()[2], "The two files are identical.");
        assert!(!same.existing_is_older());
        assert!(folders.lines()[0].starts_with("Incoming: Folder · Modified "));
        assert!(!folders.existing_is_older(), "Replace older is for files");
        let itself = Comparison {
            same_item: true,
            ..same
        };
        assert_eq!(
            itself.lines()[2],
            "This is the item itself, so it cannot replace itself."
        );
    }
}
