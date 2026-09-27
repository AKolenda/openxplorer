// SPDX-License-Identifier: AGPL-3.0-only
//! The production [`Node`]: GIO/GVfs.
//!
//! Ports `GioNode` in `desktop/gio_backend.py`, including the MTP rules
//! measured on a Pixel 9 with GVfs 1.54 (see that file and
//! `desktop/tests/test_device_staging.py`): same-folder moves are renames via
//! `set_display_name`; a cross-folder move under a new name is refused
//! because GVfs keeps the old name; one-step overwrite is never used on MTP;
//! a copy within one device keeps the source's name.

use std::path::PathBuf;

use gio::prelude::*;

use crate::transfer::{Cancellation, Node, NodeInfo, TransferError, WriteGuard};

pub struct GioNode {
    file: gio::File,
}

impl GioNode {
    pub fn new(uri: &str) -> Self {
        Self {
            file: gio::File::for_uri(uri),
        }
    }

    pub fn from_file(file: gio::File) -> Self {
        Self { file }
    }

    pub fn file(&self) -> &gio::File {
        &self.file
    }
}

fn not_yet() -> TransferError {
    TransferError::NotSupported("The native GIO backend is not implemented yet.".into())
}

impl Node for GioNode {
    fn uri(&self) -> String {
        self.file.uri().to_string()
    }

    fn name(&self) -> String {
        self.file
            .basename()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.uri())
    }

    fn path(&self) -> Option<PathBuf> {
        self.file.path()
    }

    fn child(&self, name: &str) -> Box<dyn Node> {
        Box::new(Self::from_file(self.file.child(name)))
    }

    fn parent(&self) -> Option<Box<dyn Node>> {
        self.file
            .parent()
            .map(|p| Box::new(Self::from_file(p)) as Box<dyn Node>)
    }

    fn exists(&self, cancel: Option<&Cancellation>) -> bool {
        self.file.query_exists(cancel.map(Cancellation::cancellable))
    }

    fn info(&self, _cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        Err(not_yet())
    }

    fn is_directory(&self, _cancel: Option<&Cancellation>) -> Result<bool, TransferError> {
        Err(not_yet())
    }

    fn children(&self, _cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError> {
        Err(not_yet())
    }

    fn mkdir(&self, _cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn copy_file(
        &self,
        _target: &dyn Node,
        _cancel: &Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn move_native(&self, _target: &dyn Node, _cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn replace_native(
        &self,
        _target: &dyn Node,
        _cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn delete(&self) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn trash(&self, _cancel: &Cancellation) -> Result<(), TransferError> {
        Err(not_yet())
    }

    fn can_trash(&self, _cancel: Option<&Cancellation>) -> bool {
        false
    }

    fn delete_tree(
        &self,
        _cancel: &Cancellation,
        _assert_writable: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        Err(not_yet())
    }
}
