// SPDX-License-Identifier: AGPL-3.0-only
//! Where a Recycle Bin item is kept on disk, so that deleting it for good
//! need not go through the Trash backend of `GVfs` (OPS-042, OPS-043).
//!
//! gvfsd-trash removes one item per request, and while anything watches
//! `trash:///` (the Recycle Bin view, the sidebar's full or empty icon) it
//! reads the whole Trash folder again after each change, so emptying a
//! Recycle Bin of 50,000 items took minutes, slower the more it held. The
//! freedesktop Trash specification describes the folders the backend
//! reads: an item is `$trash/files/NAME`, described by
//! `$trash/info/NAME.trashinfo`, and it leaves the Trash for good when
//! both are removed. Every program that implements the specification may
//! do that itself; the backend notices the change like any other.
//!
//! The safety rules:
//!
//! - The backend says where the item is (`standard::target-uri`); that
//!   place is used only when it is a local file directly in the `files`
//!   folder of a Trash folder the specification names for this user (the
//!   home Trash, a drive's `.Trash-$UID` or `.Trash/$UID`), and the
//!   matching `.trashinfo` is a regular file there. Anything else is
//!   deleted through the backend, as before.
//! - The item is removed with the permanent delete of the transfer engine
//!   (XFER-015): through pinned folder descriptors, never following a
//!   link, never leaving its drive.
//! - The item goes first and its `.trashinfo` last, so an item that could
//!   be deleted only in part stays listed, and can still be restored or
//!   deleted again.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::error::OpsError;
use crate::gio_node::FolderDeletion;
use crate::location::TRASH_URI;
use crate::transfer::Cancellation;

/// The `files` and `info` folders of a Trash folder.
const FILES: &str = "files";
const INFO: &str = "info";
/// The suffix of an item's description in `info`.
const TRASHINFO: &str = ".trashinfo";

/// From this many items on, where they are kept is read from one listing
/// of the Recycle Bin rather than asked of the backend one by one.
pub(super) const LISTED_FROM: usize = 64;

/// A Recycle Bin item as the Trash folder holding it keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredItem {
    /// The item itself, `$trash/files/NAME`.
    pub(super) file: PathBuf,
    /// Its description, `$trash/info/NAME.trashinfo`.
    pub(super) info: PathBuf,
}

impl StoredItem {
    /// The Trash folder holding the item.
    fn trash(&self) -> Option<&Path> {
        self.file.parent()?.parent()
    }
}

/// Where the Recycle Bin item `item` is kept, when the backend reports a
/// place [`stored_at`] accepts; `None` sends the item through the backend.
pub(super) fn stored_item(item: &gio::File, cancel: &Cancellation) -> Option<StoredItem> {
    let info = item
        .query_info(
            "standard::target-uri",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )
        .ok()?;
    stored_target(&info.attribute_string("standard::target-uri")?)
}

/// [`stored_at`] for the place `target_uri` the backend reported.
fn stored_target(target_uri: &str) -> Option<StoredItem> {
    let path = gio::File::for_uri(target_uri).path()?;
    stored_at(&path, &glib::user_data_dir(), rustix::process::getuid().as_raw())
}

/// The item at `path` with its description, when `path` is directly in
/// the `files` folder of a Trash folder of the user `uid` (the home Trash
/// under `data_home`, a drive's `.Trash-$UID` or `.Trash/$UID`) and its
/// `.trashinfo` is a regular file, not a link.
pub(super) fn stored_at(path: &Path, data_home: &Path, uid: u32) -> Option<StoredItem> {
    if !path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
        return None;
    }
    let name = path.file_name()?;
    let files = path.parent()?;
    if files.file_name()? != FILES {
        return None;
    }
    let trash = files.parent()?;
    if !is_trash_folder(trash, data_home, uid) {
        return None;
    }
    let mut info_name = OsString::from(name);
    info_name.push(TRASHINFO);
    let info = trash.join(INFO).join(info_name);
    let described = std::fs::symlink_metadata(&info).ok()?;
    described.file_type().is_file().then(|| StoredItem {
        file: path.to_path_buf(),
        info,
    })
}

/// Whether `trash` is a Trash folder the freedesktop Trash specification
/// names for the user `uid`: `$XDG_DATA_HOME/Trash`, `$topdir/.Trash-$uid`
/// or `$topdir/.Trash/$uid`.
fn is_trash_folder(trash: &Path, data_home: &Path, uid: u32) -> bool {
    if trash == data_home.join("Trash") {
        return true;
    }
    let Some(name) = trash.file_name() else {
        return false;
    };
    let uid = uid.to_string();
    if name == format!(".Trash-{uid}").as_str() {
        return true;
    }
    name == uid.as_str()
        && trash
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == ".Trash")
}

/// Where each item of the Recycle Bin is kept, by its `trash:///` URI,
/// from one listing; `None` when the Recycle Bin cannot be listed, and an
/// item missing from the map is asked about by itself.
///
/// # Errors
///
/// Only a cancellation.
pub(super) fn listed_targets(cancel: &Cancellation) -> Result<HashMap<String, String>, OpsError> {
    cancel.check()?;
    let bin = gio::File::for_uri(TRASH_URI);
    let Ok(listing) = bin.enumerate_children(
        "standard::name,standard::target-uri",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    ) else {
        cancel.check()?;
        return Ok(HashMap::new());
    };
    let mut targets = HashMap::new();
    loop {
        cancel.check()?;
        // What the listing read before it failed stays valid.
        let Ok(Some(info)) = listing.next_file(Some(cancel.cancellable())) else {
            break;
        };
        if let Some(target) = info.attribute_string("standard::target-uri") {
            targets.insert(listing.child(&info).uri().to_string(), target.to_string());
        }
    }
    // Closing only releases the listing; what was read stays valid.
    let _ = listing.close(gio::Cancellable::NONE);
    cancel.check()?;
    Ok(targets)
}

/// Deletes Recycle Bin items from their Trash folders on disk, opening
/// each Trash folder's `files` and `info` once for all of its items.
#[derive(Default)]
pub(super) struct StoreDeleter {
    /// Where each item is kept, by URI, when it was listed.
    targets: HashMap<String, String>,
    /// The opened `files` and `info` of each Trash folder, `None` where
    /// one could not be opened.
    folders: HashMap<PathBuf, Option<(FolderDeletion, FolderDeletion)>>,
}

impl StoreDeleter {
    /// A deleter that knows where the listed items of `targets` are kept.
    pub(super) fn with_targets(targets: HashMap<String, String>) -> Self {
        Self {
            targets,
            folders: HashMap::new(),
        }
    }

    /// Deletes the Recycle Bin item `item`, at `uri`, for good from its
    /// Trash folder: the item, with everything in it, through the
    /// transfer engine's pinned permanent delete (XFER-015), then its
    /// description. `None` when the item is not kept where [`stored_at`]
    /// accepts, or its Trash folder cannot be opened: then the backend
    /// deletes it.
    ///
    /// # Errors
    ///
    /// The item's failure, which leaves its description, so it stays
    /// listed; a description that cannot be removed.
    pub(super) fn delete(
        &mut self,
        uri: &str,
        item: &gio::File,
        cancel: &Cancellation,
    ) -> Option<Result<(), OpsError>> {
        let stored = match self.targets.get(uri) {
            Some(target) => stored_target(target),
            None => stored_item(item, cancel),
        }?;
        let trash = stored.trash()?.to_path_buf();
        let opened = self.folders.entry(trash.clone()).or_insert_with(|| {
            let files = FolderDeletion::open(&trash.join(FILES)).ok()?;
            let info = FolderDeletion::open(&trash.join(INFO)).ok()?;
            Some((files, info))
        });
        let (files, info) = opened.as_ref()?;
        Some(delete_stored(&stored, files, info, cancel))
    }
}

/// Deletes `stored` with the opened `files` and `info` of its Trash
/// folder: the item first, then its description.
fn delete_stored(
    stored: &StoredItem,
    files: &FolderDeletion,
    info: &FolderDeletion,
    cancel: &Cancellation,
) -> Result<(), OpsError> {
    let (Some(name), Some(description)) = (stored.file.file_name(), stored.info.file_name()) else {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Choose a file or folder to delete.",
        )));
    };
    files.delete(name, cancel)?;
    match info.remove_file(description) {
        Err(crate::transfer::TransferError::NotFound(_)) | Ok(()) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: u32 = 1000;

    fn delete_for_test(stored: &StoredItem) -> Result<(), OpsError> {
        let trash = stored.trash().unwrap();
        let files = FolderDeletion::open(&trash.join(FILES)).unwrap();
        let info = FolderDeletion::open(&trash.join(INFO)).unwrap();
        delete_stored(stored, &files, &info, &Cancellation::new())
    }

    fn trash_with(root: &Path, trash: &Path, name: &str) -> PathBuf {
        let _ = root;
        std::fs::create_dir_all(trash.join(FILES)).unwrap();
        std::fs::create_dir_all(trash.join(INFO)).unwrap();
        let file = trash.join(FILES).join(name);
        std::fs::write(&file, b"old").unwrap();
        std::fs::write(
            trash.join(INFO).join(format!("{name}{TRASHINFO}")),
            b"[Trash Info]\n",
        )
        .unwrap();
        file
    }

    #[test]
    fn items_of_the_trash_folders_the_specification_names_are_found_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let home = trash_with(root.path(), &data.join("Trash"), "report.txt");
        let drive = trash_with(root.path(), &root.path().join("usb/.Trash-1000"), "photo.jpg");
        let shared = trash_with(root.path(), &root.path().join("disk/.Trash/1000"), "notes.txt");

        for file in [home, drive, shared] {
            let stored = stored_at(&file, &data, UID).expect("a Trash item");
            assert_eq!(stored.file, file);
            let mut info = file.file_name().unwrap().to_owned();
            info.push(TRASHINFO);
            assert_eq!(
                stored.info,
                file.parent().unwrap().parent().unwrap().join(INFO).join(info)
            );
        }
    }

    #[test]
    fn anything_else_is_left_to_the_backend() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        // Another user's Trash, a folder merely called Trash, and an item
        // not directly in files.
        let other = trash_with(root.path(), &root.path().join("usb/.Trash-1001"), "a.txt");
        let named = trash_with(root.path(), &root.path().join("Documents/Trash"), "b.txt");
        let wrong_owner = trash_with(root.path(), &root.path().join("disk/Trash/1000"), "c.txt");
        let home = data.join("Trash");
        let nested = trash_with(root.path(), &home, "folder");
        std::fs::remove_file(&nested).unwrap();
        std::fs::create_dir(&nested).unwrap();
        let inside = nested.join("inside.txt");
        std::fs::write(&inside, b"x").unwrap();
        // No description, or one that is a link.
        let undescribed = home.join(FILES).join("d.txt");
        std::fs::write(&undescribed, b"x").unwrap();
        let linked = home.join(FILES).join("e.txt");
        std::fs::write(&linked, b"x").unwrap();
        std::os::unix::fs::symlink(
            home.join(INFO).join("folder.trashinfo"),
            home.join(INFO).join("e.txt.trashinfo"),
        )
        .unwrap();

        for path in [other, named, wrong_owner, inside, undescribed, linked] {
            assert_eq!(stored_at(&path, &data, UID), None, "{}", path.display());
        }
        assert_eq!(stored_at(Path::new("relative/files/a"), &data, UID), None);
        assert_eq!(stored_at(&home.join("files/../files/a"), &data, UID), None);
    }

    #[test]
    fn a_stored_item_goes_with_everything_in_it_then_its_description() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let home = data.join("Trash");
        let folder = trash_with(root.path(), &home, "Old project");
        std::fs::remove_file(&folder).unwrap();
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("inner/a.txt"), b"a").unwrap();
        // A link inside is removed as a link; what it points at stays.
        let outside = root.path().join("keep.txt");
        std::fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
        let stored = stored_at(&folder, &data, UID).unwrap();

        delete_for_test(&stored).unwrap();

        assert!(!folder.exists());
        assert!(!stored.info.exists());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }

    #[test]
    fn read_only_folders_inside_a_trashed_folder_go_too() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let folder = trash_with(root.path(), &data.join("Trash"), "Read-only inside");
        std::fs::remove_file(&folder).unwrap();
        let locked = folder.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("read-only.txt"), b"kept").unwrap();
        std::fs::set_permissions(
            locked.join("read-only.txt"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let stored = stored_at(&folder, &data, UID).unwrap();

        let result = delete_for_test(&stored);

        if locked.exists() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        result.unwrap();
        assert!(!folder.exists());
        assert!(!stored.info.exists());
    }

    #[test]
    fn an_item_deleted_only_in_part_keeps_its_description() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let folder = trash_with(root.path(), &data.join("Trash"), "Too deep");
        std::fs::remove_file(&folder).unwrap();
        // Deeper than any deletion walks.
        let mut deepest = folder.clone();
        for _ in 0..=crate::transfer::MAX_DEPTH {
            deepest.push("d");
        }
        std::fs::create_dir_all(&deepest).unwrap();
        std::fs::write(deepest.join("deep.txt"), b"deep").unwrap();
        let stored = stored_at(&folder, &data, UID).unwrap();

        let result = delete_for_test(&stored);

        assert!(result.is_err());
        assert!(deepest.join("deep.txt").exists());
        assert!(stored.info.exists(), "the item stays listed");
    }
}
