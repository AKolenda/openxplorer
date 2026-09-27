// SPDX-License-Identifier: AGPL-3.0-only
//! Synchronous GIO/GVfs adapter for the transfer engine.
//!
//! Ports `GioNode` in `desktop/gio_backend.py`. Calls belong on a worker,
//! never the GTK main thread. MTP device restrictions live in `move_item`;
//! metadata queries and enumeration never follow symbolic links.
//!
//! Local permanent deletion pins directory descriptors and refuses symlink
//! ancestors. Remote folder permanent deletion is intentionally unavailable:
//! GIO cannot pin remote ancestors against concurrent path replacement.
//! MTP behavior is regression-tested with simulated devices, not hardware.

mod local_delete;
mod move_item;
mod query;
mod removal;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use gio::prelude::*;

use crate::transfer::{Cancellation, Node, NodeInfo, TransferError, WriteGuard};

/// A file or folder addressed through GIO, including GVfs remote backends.
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

    fn require_item(&self) -> Result<(), TransferError> {
        if self.file.parent().is_none() {
            return Err(TransferError::failed(
                "Filesystem roots cannot be changed as items.",
            ));
        }
        crate::location::require_item_uri(&self.uri())
            .map(|_| ())
            .map_err(|error| TransferError::failed(error.to_string()))
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
        self.file
            .parent()
            .map(|parent| Box::new(Self::from_file(parent)) as Box<dyn Node>)
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
        self.file.make_directory(raw(cancel)).map_err(Into::into)
    }

    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        cancel.check()?;
        let target_file = gio::File::for_uri(&target.uri());
        let mut report = |current: i64, total: i64| {
            progress(current.max(0) as u64, total.max(0) as u64);
        };
        self.file
            .copy(
                &target_file,
                gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
                Some(cancel.cancellable()),
                Some(&mut report),
            )
            .map_err(Into::into)
    }

    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.move_item(target, cancel, false)
    }

    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.move_item(target, cancel, true)
    }

    fn delete(&self) -> Result<(), TransferError> {
        self.file.delete(gio::Cancellable::NONE).map_err(Into::into)
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
        self.require_item()?;
        if self.file.has_uri_scheme("file") {
            if let Some(path) = self.file.path() {
                return local_delete::delete_tree(&path, cancel, assert_writable);
            }
        }
        self.delete_remote_item(cancel, assert_writable)
    }

    fn stage_as_sibling(&self) -> bool {
        self.is_mtp()
    }

    fn native_copy_keeps_name(&self, target_dir: &dyn Node) -> bool {
        let source_uri = self.uri();
        let target_uri = target_dir.uri();
        self.is_mtp()
            && gio::File::for_uri(&target_uri).has_uri_scheme("mtp")
            && authority(&source_uri) == authority(&target_uri)
    }

    fn refresh_listing(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        if self.is_mtp() {
            self.list_children(cancel)?;
        }
        Ok(())
    }
}

fn authority(uri: &str) -> Option<&str> {
    uri.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
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
