// SPDX-License-Identifier: AGPL-3.0-only
//! Test-only local provider: the Rust counterpart of
//! `desktop/tests/local_provider.py`. Never used by the application.
//!
//! It exercises the transfer orchestration against temporary real files. It
//! cannot validate the production GIO/GVfs adapter or SMB behaviour.
//!
//! The Python tests change behaviour by subclassing `LocalNode`. Here a
//! [`Provider`] (in `local/provider.rs`) plays that role: it overrides only
//! the methods a test needs, and every other method defers to
//! [`Provider::base`] (the "superclass") and finally to the plain local
//! behaviour on [`LocalNode`].

mod provider;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gio::prelude::*;
use ox_core::transfer::{Cancellation, Node, NodeFactory, NodeInfo, NodeKind, TransferError, WriteGuard};
use rustix::fs::{renameat_with, RenameFlags, CWD};

pub use provider::{local, Provider};

/// The `file://` URI of `path`.
pub fn file_uri(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// The local path behind any URI this double produces
/// (`<scheme>://<authority>/<escaped path>`), whatever its scheme.
pub fn path_from_uri(uri: &str) -> PathBuf {
    let (_, rest) = uri.split_once("://").expect("test URIs have an authority");
    let slash = rest.find('/').expect("test URIs have an absolute path");
    let local_uri = format!("file://{}", &rest[slash..]);
    gio::File::for_uri(&local_uri)
        .path()
        .expect("test URIs name local paths")
}

/// The local path of a node from this double.
pub fn local_path_of(node: &dyn Node) -> PathBuf {
    node.path().unwrap_or_else(|| path_from_uri(&node.uri()))
}

/// Stops before the next step when the user cancelled; without a
/// cancellation there is nothing to check.
fn check_cancelled(cancel: Option<&Cancellation>) -> Result<(), TransferError> {
    match cancel {
        Some(cancel) => cancel.check(),
        None => Ok(()),
    }
}

/// What `file_type` is, as the engine distinguishes items.
fn node_kind(file_type: fs::FileType) -> NodeKind {
    if file_type.is_dir() {
        NodeKind::Directory
    } else if file_type.is_symlink() {
        NodeKind::Symlink
    } else if file_type.is_file() {
        NodeKind::File
    } else {
        NodeKind::Special
    }
}

/// One local file or folder, with behaviour from its [`Provider`].
#[derive(Clone)]
pub struct LocalNode {
    path: PathBuf,
    provider: Arc<dyn Provider>,
}

impl LocalNode {
    /// The node at `path`.
    pub fn new(path: impl Into<PathBuf>, provider: Arc<dyn Provider>) -> Self {
        Self {
            path: path.into(),
            provider,
        }
    }

    /// A factory resolving `file://` URIs to nodes of `provider`.
    pub fn factory(provider: Arc<dyn Provider>) -> NodeFactory {
        Arc::new(move |uri: &str| {
            let path = gio::File::for_uri(uri)
                .path()
                .ok_or_else(|| TransferError::failed(format!("Not a local test URI: {uri}")))?;
            Ok(Box::new(LocalNode::new(path, provider.clone())) as Box<dyn Node>)
        })
    }

    /// The real local path, whatever [`Node::path`] reports.
    pub fn local_path(&self) -> &Path {
        &self.path
    }

    /// Another node with the same provider (Python `type(self)(path=...)`).
    pub fn at(&self, path: impl Into<PathBuf>) -> LocalNode {
        LocalNode::new(path, self.provider.clone())
    }

    /// `os.path.lexists`: true for a dangling link too. Cancellation is not
    /// checked because this answer cannot carry an error.
    pub fn local_exists(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok()
    }

    /// `lstat`: the kind, size and permission bits without following links.
    pub fn local_info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        check_cancelled(cancel)?;
        let metadata = fs::symlink_metadata(&self.path)?;
        Ok(NodeInfo {
            kind: node_kind(metadata.file_type()),
            size: metadata.len(),
            mode: Some(metadata.permissions().mode() & 0o7777),
        })
    }

    /// Creates the folder exclusively: an existing item is never reused.
    pub fn local_create_directory(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        check_cancelled(cancel)?;
        fs::create_dir(&self.path)?;
        Ok(())
    }

    /// Copies a link as a link, or a file in 8 KiB chunks into a new file
    /// (`open('xb')`), checking cancellation before each chunk.
    pub fn local_copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        cancel.check()?;
        let target_path = local_path_of(target);
        if fs::symlink_metadata(&self.path)?.file_type().is_symlink() {
            let link = fs::read_link(&self.path)?;
            std::os::unix::fs::symlink(link, &target_path)?;
            return Ok(());
        }
        self.copy_contents(&target_path, cancel, progress)
    }

    /// Copies this file's bytes into the new file `target_path`.
    fn copy_contents(
        &self,
        target_path: &Path,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        let total = fs::metadata(&self.path)?.len();
        let mut input = fs::File::open(&self.path)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target_path)?;
        let mut buffer = [0u8; 8192];
        let mut current = 0u64;
        loop {
            cancel.check()?;
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            current += read as u64;
            progress(current, total);
        }
        Ok(())
    }

    /// A rename that never overwrites: `renameat2(RENAME_NOREPLACE)`, like
    /// the Python double. The kernel refuses a taken name atomically, so
    /// even a racing writer is never overwritten.
    pub fn local_move_native(
        &self,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        check_cancelled(cancel)?;
        let target_path = local_path_of(target);
        renameat_with(CWD, &self.path, CWD, &target_path, RenameFlags::NOREPLACE)?;
        Ok(())
    }

    /// `os.replace`: overwrites the target.
    pub fn local_replace_native(
        &self,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        check_cancelled(cancel)?;
        fs::rename(&self.path, local_path_of(target))?;
        Ok(())
    }

    /// Removes one file, link or empty folder.
    pub fn local_delete(&self) -> Result<(), TransferError> {
        if fs::symlink_metadata(&self.path)?.is_dir() {
            fs::remove_dir(&self.path)?;
        } else {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

impl Node for LocalNode {
    fn uri(&self) -> String {
        self.provider.uri(self)
    }

    fn name(&self) -> OsString {
        self.path.file_name().map(OsStr::to_os_string).unwrap_or_default()
    }

    fn path(&self) -> Option<PathBuf> {
        self.provider.path(self)
    }

    fn child(&self, name: &OsStr) -> Box<dyn Node> {
        Box::new(self.at(self.path.join(name)))
    }

    fn parent(&self) -> Option<Box<dyn Node>> {
        self.path
            .parent()
            .map(|parent| Box::new(self.at(parent)) as Box<dyn Node>)
    }

    fn exists(&self, cancel: Option<&Cancellation>) -> bool {
        self.provider.exists(self, cancel)
    }

    fn info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        self.provider.info(self, cancel)
    }

    fn is_directory(&self, cancel: Option<&Cancellation>) -> Result<bool, TransferError> {
        check_cancelled(cancel)?;
        Ok(self.path.is_dir())
    }

    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError> {
        let mut children: Vec<Box<dyn Node>> = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            check_cancelled(cancel)?;
            children.push(Box::new(self.at(entry?.path())));
        }
        Ok(children)
    }

    fn create_directory(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.provider.create_directory(self, cancel)
    }

    fn copy_file(
        &self,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        self.provider.copy_file(self, target, cancel, progress)
    }

    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.provider.move_native(self, target, cancel)
    }

    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.provider.replace_native(self, target, cancel)
    }

    fn delete(&self) -> Result<(), TransferError> {
        self.provider.delete(self)
    }

    fn trash(&self, cancel: &Cancellation) -> Result<(), TransferError> {
        cancel.check()?;
        Err(TransferError::NotSupported(
            "Test provider deliberately does not support Trash; no delete fallback".into(),
        ))
    }

    fn can_trash(&self, _cancel: Option<&Cancellation>) -> Result<bool, TransferError> {
        Ok(false)
    }

    fn delete_tree(&self, cancel: &Cancellation, guard: Option<&WriteGuard>) -> Result<(), TransferError> {
        cancel.check()?;
        if let Some(guard) = guard {
            guard(&self.uri())?;
        }
        let is_real_directory = fs::symlink_metadata(&self.path).is_ok_and(|metadata| metadata.is_dir());
        if is_real_directory {
            for child in self.children(Some(cancel))? {
                child.delete_tree(cancel, guard)?;
            }
        }
        self.delete()
    }

    fn has_sibling_staging(&self) -> bool {
        self.provider.has_sibling_staging()
    }

    fn native_copy_keeps_name(&self, target_folder: &dyn Node) -> bool {
        self.provider.native_copy_keeps_name(self, target_folder)
    }

    fn refresh_listing(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.provider.refresh_listing(self, cancel)
    }
}
