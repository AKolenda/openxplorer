// SPDX-License-Identifier: AGPL-3.0-only
//! Synchronous GIO/GVfs adapter for the transfer engine.
//!
//! Ports `GioNode` in `desktop/gio_backend.py`. Calls belong on a worker,
//! never the GTK main thread. Metadata queries and enumeration never follow
//! symbolic links (`query`); MTP device restrictions live in `move_item`.
//!
//! Permanent deletion of a local item, and cleanup of local staging, run
//! relative to pinned folder descriptors (`local_delete`), so a folder
//! swapped for a symbolic link during the deletion cannot redirect it.
//! Remote locations (SMB shares, phones) can only be deleted by path
//! (`remote_delete`), exactly as the Python app, Nautilus and Dolphin do.
//! MTP behaviour is regression-tested with simulated devices, not hardware.

mod local_delete;
mod move_item;
mod query;
mod remote_delete;
mod removal;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use gio::prelude::*;

use crate::location::split_location;
use crate::transfer::{clean_staging, Cancellation, ItemIdentity, Node, NodeInfo, TransferError, WriteGuard};

/// A file or folder addressed through GIO, including `GVfs` remote backends.
#[derive(Clone, Debug)]
pub struct GioNode {
    file: gio::File,
}

impl GioNode {
    /// Addresses a canonical URI. Validate user-entered locations before
    /// constructing the node, using [`crate::location::normalise`].
    pub fn new(uri: &str) -> Self {
        Self::from_file(gio::File::for_uri(uri))
    }

    /// Wraps a file obtained from GIO enumeration or a local path.
    pub fn from_file(file: gio::File) -> Self {
        Self { file }
    }

    /// The underlying file for other GIO operations.
    pub fn file(&self) -> &gio::File {
        &self.file
    }

    fn is_mtp(&self) -> bool {
        self.file.has_uri_scheme("mtp")
    }

    /// The path of a `file:` item. `GVfs` FUSE paths of remote items do not
    /// count: their backends cannot pin folders or change Unix modes.
    fn local_path(&self) -> Option<PathBuf> {
        if self.file.has_uri_scheme("file") {
            self.file.path()
        } else {
            None
        }
    }

    /// Refuses filesystem roots, whole shares and whole devices, like
    /// `require_item_uri` in `desktop/core.py`.
    fn require_item(&self) -> Result<(), TransferError> {
        if self.file.parent().is_none() {
            return Err(TransferError::failed(
                "Filesystem roots cannot be changed as items.",
            ));
        }
        crate::location::require_item_uri(&self.uri())?;
        Ok(())
    }
}

impl Node for GioNode {
    fn uri(&self) -> String {
        self.file.uri().to_string()
    }

    fn name(&self) -> OsString {
        match self.file.basename() {
            Some(name) => name.into_os_string(),
            None => OsString::from(self.uri()),
        }
    }

    fn path(&self) -> Option<PathBuf> {
        self.file.path()
    }

    fn child(&self, name: &OsStr) -> Box<dyn Node> {
        Box::new(Self::from_file(self.file.child(name)))
    }

    fn parent(&self) -> Option<Box<dyn Node>> {
        let parent = self.file.parent()?;
        Some(Box::new(Self::from_file(parent)))
    }

    fn exists(&self, cancel: Option<&Cancellation>) -> bool {
        // query_exists follows links on some backends; a dangling link is
        // still a taken destination name and must participate in conflicts.
        self.query_info(cancel).is_ok()
    }

    fn info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        self.query_info(cancel)
    }

    fn is_directory(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError> {
        check(cancel)?;
        // Follows links on purpose: a destination reached through a link to
        // a folder is a folder, as in the Python app.
        let info = self
            .file
            .query_info("standard::type", gio::FileQueryInfoFlags::NONE, raw(cancel))?;
        Ok(info.file_type() == gio::FileType::Directory)
    }

    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError> {
        self.list_children(cancel)
    }

    fn mkdir(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        check(cancel)?;
        self.file.make_directory(raw(cancel))?;
        Ok(())
    }

    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        cancel.check()?;
        let target_file = gio::File::for_uri(&target.uri());
        let mut report = |current: i64, total: i64| progress(byte_count(current), byte_count(total));
        // Without OVERWRITE the copy refuses an existing target, which keeps
        // the engine's "never overwrite" rule for staging and uploads.
        self.file.copy(
            &target_file,
            gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
            Some(&mut report),
        )?;
        Ok(())
    }

    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.move_item(target, cancel, move_item::Overwrite::Never)
    }

    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.move_item(target, cancel, move_item::Overwrite::Replace)
    }

    fn delete(&self) -> Result<(), TransferError> {
        self.file.delete(gio::Cancellable::NONE)?;
        Ok(())
    }

    fn trash(&self, cancel: &Cancellation) -> Result<(), TransferError> {
        self.trash_item(cancel)
    }

    fn can_trash(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError> {
        let queried = self.file.query_info(
            "access::can-trash",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            raw(cancel),
        );
        match queried {
            Ok(info) => Ok(info.boolean("access::can-trash")),
            // A share unmounted in the background must reach the caller, so
            // it can mount and ask again instead of offering a permanent
            // delete for a location that has a Trash.
            Err(error) if error.matches(gio::IOErrorEnum::NotMounted) => Err(error.into()),
            Err(_) => Ok(false),
        }
    }

    fn delete_tree(
        &self,
        cancel: &Cancellation,
        assert_writable: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        self.delete_item_tree(cancel, assert_writable)
    }

    fn delete_staging(&self, created: Option<ItemIdentity>) -> Result<(), TransferError> {
        // Remote staging can only be removed by path, as in the Python app.
        match self.local_path() {
            Some(path) => local_delete::delete_staging(&path, created),
            None => clean_staging(self),
        }
    }

    fn stage_as_sibling(&self) -> bool {
        self.is_mtp()
    }

    fn native_copy_keeps_name(&self, target_dir: &dyn Node) -> bool {
        let target_uri = target_dir.uri();
        let target_is_mtp = gio::File::for_uri(&target_uri).has_uri_scheme("mtp");
        self.is_mtp() && target_is_mtp && same_authority(&self.uri(), &target_uri)
    }

    fn refresh_listing(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        if self.is_mtp() {
            self.list_children(cancel)?;
        }
        Ok(())
    }
}

/// True when both URIs name the same host or device, like comparing
/// `uri.split('/')[2]` in `desktop/gio_backend.py`.
fn same_authority(first: &str, second: &str) -> bool {
    match (split_location(first), split_location(second)) {
        (Ok(first), Ok(second)) => first.netloc == second.netloc,
        _ => false,
    }
}

/// A byte count from GIO's progress callback; GIO never reports a negative
/// one, and a broken backend's is shown as zero.
fn byte_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn raw(cancel: Option<&Cancellation>) -> Option<&gio::Cancellable> {
    cancel.map(Cancellation::cancellable)
}

fn check(cancel: Option<&Cancellation>) -> Result<(), TransferError> {
    match cancel {
        Some(cancel) => cancel.check(),
        None => Ok(()),
    }
}
