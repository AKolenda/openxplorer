// SPDX-License-Identifier: AGPL-3.0-only
//! Whether dragged items are on the drive of the folder they are dropped
//! on, which decides what a plain drag does (DND-017): Windows Explorer
//! moves items dragged within a drive and copies them to another drive.
//!
//! A drive is a local filesystem, told apart by its device number, which
//! is also what decides whether a rename can move an item. Only local
//! `file:` items count: a network folder, a mount of the session's GIO
//! daemons and a document portal path share one device number for many
//! shares or files, so a drop there, or from there, stays a copy.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gio, glib};

/// True when every item of `uris` is on the local drive of `folder`.
/// An empty list, an item that cannot be read, or any item or folder that
/// is not a local file is false, so the drop copies.
pub(super) fn on_same_drive(uris: &[String], folder: &str) -> bool {
    let runtime = glib::user_runtime_dir();
    let Some(drive) = drive_of(folder, &runtime, true) else {
        return false;
    };
    !uris.is_empty()
        && uris
            .iter()
            .all(|uri| drive_of(uri, &runtime, false) == Some(drive))
}

/// The device number of the local file `uri`, or `None` when it is not a
/// plain local file. A dragged symbolic link is on the drive of the folder
/// holding it (`follow` false); a destination folder is where its link
/// points (`follow` true).
fn drive_of(uri: &str, runtime: &Path, follow: bool) -> Option<u64> {
    let path = local_file_path(uri, runtime)?;
    let metadata = if follow {
        std::fs::metadata(&path)
    } else {
        std::fs::symlink_metadata(&path)
    };
    metadata.ok().map(|metadata| metadata.dev())
}

/// The path of `uri` when it is a `file:` address outside the session's
/// runtime folder, which holds the GIO daemons' mounts and the document
/// portal.
fn local_file_path(uri: &str, runtime: &Path) -> Option<PathBuf> {
    let scheme = uri.split(':').next()?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let path = gio::File::for_uri(uri).path()?;
    (!path.starts_with(runtime)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `file:` URI of `path`.
    fn uri_of(path: &Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    /// parity: DND-017
    #[test]
    fn items_in_one_folder_are_on_its_drive_and_an_empty_drop_is_not() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("Notes.txt");
        std::fs::write(&item, "notes").expect("the item is written");
        let destination = folder.path().join("Documents");
        std::fs::create_dir(&destination).expect("the folder is made");

        assert!(on_same_drive(&[uri_of(&item)], &uri_of(&destination)));
        assert!(!on_same_drive(&[], &uri_of(&destination)));
    }

    /// parity: DND-017
    #[test]
    fn network_missing_and_runtime_items_are_never_on_the_drive() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let destination = uri_of(folder.path());
        let missing = uri_of(&folder.path().join("Gone.txt"));
        let runtime = glib::user_runtime_dir();
        let portal = uri_of(&runtime.join("doc/1234/Notes.txt"));

        assert!(!on_same_drive(
            &["smb://nas/share/Notes.txt".to_owned()],
            &destination
        ));
        assert!(!on_same_drive(&[missing], &destination));
        assert!(!on_same_drive(&[uri_of(folder.path())], "smb://nas/share/"));
        assert_eq!(local_file_path(&portal, &runtime), None);
    }

    /// parity: DND-017
    #[test]
    fn a_dragged_link_is_on_the_drive_of_the_folder_holding_it() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let link = folder.path().join("Link");
        std::os::unix::fs::symlink("/proc/self", &link).expect("the link is made");

        assert!(on_same_drive(&[uri_of(&link)], &uri_of(folder.path())));
    }
}
