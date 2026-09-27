// SPDX-License-Identifier: AGPL-3.0-only
//! Permanent deletion on remote GIO locations: SMB shares, phones (MTP) and
//! other `GVfs` backends. Local folders use the descriptor-pinned walk in
//! `local_delete` instead.
//!
//! Ports `GioNode._delete_recursive` in `desktop/gio_backend.py`.
//!
//! Rules enforced here (XFER-015):
//! - Every item is inspected without following symbolic links. Only real
//!   folders are entered; links, shortcuts, mountables and special files
//!   are deleted as items, so a link's target is never touched.
//! - The write guard is asked about every item before it is removed, and
//!   the nesting limit applies, exactly as for local deletion.
//! - The first failure stops the deletion; items not yet reached stay in
//!   place. Folders are emptied first because remote backends only remove
//!   empty folders.
//!
//! Deletion is by path. `GVfs` offers no handle that pins a remote folder, so
//! a rename on the server during the deletion can redirect later steps.
//! This is an accepted limit shared with the Python app, Nautilus and
//! Dolphin (KIO); SMB servers also resolve their own symbolic links, which
//! every client then sees as folders.

use gio::prelude::*;

use super::query::enumerate_files;
use crate::transfer::{nesting_error, Cancellation, TransferError, WriteGuard, MAX_DEPTH};

/// Permanently deletes `file` and, for a folder, everything inside it.
///
/// # Errors
///
/// The first failure, the guard's refusal, the nesting limit or
/// [`TransferError::Cancelled`]; items not yet reached stay in place.
pub(super) fn delete_tree(
    file: &gio::File,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
) -> Result<(), TransferError> {
    RemoteDeletion { cancel, guard }.delete(file, 0)
}

/// One user-confirmed permanent deletion.
struct RemoteDeletion<'a> {
    cancel: &'a Cancellation,
    guard: Option<&'a WriteGuard>,
}

impl RemoteDeletion<'_> {
    /// Deletes `file` at nesting `depth`, children first.
    fn delete(&self, file: &gio::File, depth: usize) -> Result<(), TransferError> {
        self.cancel.check()?;
        if let Some(guard) = self.guard {
            guard(&file.uri())?;
            // The guard inspects the location and can take a while: stop
            // before touching the item if the user cancelled meanwhile.
            self.cancel.check()?;
        }
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        // Only a real folder is entered: a link is deleted as a link.
        if self.is_real_folder(file)? {
            for child in enumerate_files(file, Some(self.cancel))? {
                self.delete(&child, depth + 1)?;
            }
        }
        self.cancel.check()?;
        file.delete(Some(self.cancel.cancellable()))?;
        Ok(())
    }

    /// True only for a folder itself, never for a link to one.
    fn is_real_folder(&self, file: &gio::File) -> Result<bool, TransferError> {
        let info = file.query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(self.cancel.cancellable()),
        )?;
        Ok(info.file_type() == gio::FileType::Directory)
    }
}

#[cfg(test)]
mod tests {
    //! The deletion runs here on `file://` trees, which exercise the same
    //! GIO calls a remote backend receives. SMB and phone runs are part of
    //! the manual validation in `native/VALIDATION.md`.

    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::*;

    fn delete(path: &Path, cancel: &Cancellation, guard: Option<&WriteGuard>) -> Result<(), TransferError> {
        delete_tree(&gio::File::for_path(path), cancel, guard)
    }

    /// Port of `_delete_recursive` in `desktop/gio_backend.py`: hidden items
    /// and empty folders go too, children before their folder.
    ///
    /// parity: XFER-015
    #[test]
    fn nested_folders_are_deleted_depth_first() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let selected = temp.path().join("selected");
        fs::create_dir_all(selected.join("nested/empty")).expect("create the tree");
        fs::write(selected.join("nested/data"), b"delete me").expect("write");
        fs::write(selected.join(".hidden"), b"delete me too").expect("write");
        delete(&selected, &Cancellation::new(), None).expect("the tree is deleted");
        assert!(!selected.exists());
    }

    /// parity: XFER-015, XFER-017
    #[test]
    fn a_link_to_a_folder_is_removed_without_touching_its_target() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).expect("create the target");
        fs::write(outside.join("kept"), b"kept").expect("write");
        let selected = temp.path().join("selected");
        fs::create_dir(&selected).expect("create the selected folder");
        symlink(&outside, selected.join("link")).expect("create the link");
        delete(&selected, &Cancellation::new(), None).expect("the tree is deleted");
        assert!(!selected.exists());
        assert_eq!(fs::read(outside.join("kept")).expect("read"), b"kept");
    }

    /// parity: XFER-015, XFER-020
    #[test]
    fn a_protected_descendant_stops_the_deletion_before_it_is_removed() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let selected = temp.path().join("selected");
        let protected = selected.join("sub/.snapshot");
        fs::create_dir_all(&protected).expect("create the tree");
        fs::write(protected.join("version"), b"backup").expect("write");
        let guard = |uri: &str| {
            if uri.contains(".snapshot") {
                Err(TransferError::failed("Protected snapshot."))
            } else {
                Ok(())
            }
        };
        let result = delete(&selected, &Cancellation::new(), Some(&guard));
        assert_eq!(result, Err(TransferError::failed("Protected snapshot.")));
        assert_eq!(fs::read(protected.join("version")).expect("read"), b"backup");
    }

    /// parity: XFER-015
    #[test]
    fn nesting_deeper_than_the_limit_is_refused() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let selected = temp.path().join("selected");
        let deepest = (0..=MAX_DEPTH).fold(selected.clone(), |path, _| path.join("d"));
        fs::create_dir_all(&deepest).expect("create the deep tree");
        let result = delete(&selected, &Cancellation::new(), None);
        assert_eq!(result, Err(nesting_error()));
        assert!(deepest.exists());
    }

    /// parity: XFER-015
    #[test]
    fn cancellation_stops_the_deletion_and_keeps_the_rest() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let selected = temp.path().join("selected");
        fs::create_dir(&selected).expect("create the selected folder");
        fs::write(selected.join("data"), b"kept").expect("write");
        let cancel = Cancellation::new();
        let requested = cancel.clone();
        let cancelling_guard = move |_uri: &str| {
            requested.cancel();
            Ok(())
        };
        let result = delete(&selected, &cancel, Some(&cancelling_guard));
        assert_eq!(result, Err(TransferError::Cancelled));
        assert_eq!(fs::read(selected.join("data")).expect("read"), b"kept");
    }
}
