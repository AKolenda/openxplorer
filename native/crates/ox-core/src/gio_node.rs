// SPDX-License-Identifier: AGPL-3.0-only
//! Synchronous GIO/GVfs adapter for the transfer engine.
//!
//! Ports `GioNode` in `v2.0.0:desktop/gio_backend.py`. Calls block, so they belong
//! on a worker, never the GTK main thread. The transfer rules this adapter
//! enforces carry the feature ids of [`crate::transfer`]:
//!
//! | Module | Responsibility |
//! |---|---|
//! | `query` | Metadata and listings that never follow links (XFER-017) |
//! | `move_item` | Native moves (XFER-011), no-replace publishing (XFER-007) and MTP renames (XFER-024) |
//! | `removal` | Trash (XFER-014) and permanent deletion (XFER-015) |
//! | `local_delete` | Deletion relative to pinned folder descriptors |
//! | `remote_delete` | Deletion by path on shares and phones |
//! | `into_admin` | Copies into `admin://` that never let root open the source |
//!
//! Permanent deletion of a local item, and cleanup of local staging, run
//! relative to pinned folder descriptors, so a folder swapped for a
//! symbolic link during the deletion cannot redirect it. Remote locations
//! (SMB shares, phones) can only be deleted by path, exactly as the Python
//! app, Nautilus and Dolphin do. A whole folder under `admin://`, where
//! `gvfsd-admin` would delete by path as root, is not deleted here at all:
//! it goes to the pinned walk of [`crate::admin_delete`]. MTP behaviour is
//! regression-tested with simulated devices, not hardware.

mod into_admin;
mod local_delete;
mod move_item;
mod query;
mod remote_delete;
mod removal;

pub(crate) use local_delete::FolderDeletion;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::OnceLock;

use gio::prelude::*;

use crate::location::split_location;
use crate::transfer::{
    check_cancelled, clean_staging, Cancellation, FilesystemInfo, ItemIdentity, Node, NodeInfo, NodeKind,
    TransferError, WriteGuard,
};

/// A file or folder addressed through GIO, including `GVfs` remote backends.
#[derive(Clone, Debug)]
pub struct GioNode {
    file: gio::File,
    /// The name a copy of a Google Drive item takes, read once
    /// ([`Self::copy_name`]).
    copy_name: OnceLock<Option<OsString>>,
}

impl GioNode {
    /// OPS-047: refuses a native move from elsewhere into `admin://`,
    /// which `gvfsd-admin` would do by renaming the source's path as root.
    /// The engine copies it instead (XFER-013).
    fn refuse_move_into_admin(&self, target: &dyn Node) -> Result<(), TransferError> {
        let target = gio::File::for_uri(&target.uri());
        if into_admin::crosses_into_admin(&self.file, &target) {
            return Err(into_admin::no_native_move());
        }
        Ok(())
    }

    /// Addresses a canonical URI. Validate user-entered locations before
    /// constructing the node, using [`crate::location::normalise`].
    pub fn new(uri: &str) -> Self {
        Self::from_file(gio::File::for_uri(uri))
    }

    /// Wraps a file obtained from GIO enumeration or a local path.
    pub fn from_file(file: gio::File) -> Self {
        Self {
            file,
            copy_name: OnceLock::new(),
        }
    }

    /// The name `GVfs` gives a copy of an item whose address names it by
    /// ID (DEV-013): Google Drive's `standard::copy-name`, the item's
    /// title with any `/` made `-`, and its ID added when another item in
    /// its folder has the same title; its sanitised title for the drive's
    /// top folders, which have no copy name. `None` for every other item,
    /// which is named by its address, or when the backend does not say.
    ///
    /// Blocks on the first call for a Google Drive item, as every query
    /// of this adapter does.
    fn copy_name(&self) -> Option<&OsString> {
        let read = || {
            let uri = self.file.uri();
            if !crate::location::names_items_by_id(&uri) {
                return None;
            }
            let info = self
                .file
                .query_info(
                    "standard::copy-name,standard::display-name",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    gio::Cancellable::NONE,
                )
                .ok()?;
            // GVfs gives My Drive, Shared with me and Shared drives no copy
            // name; their titles are the name, with any "/" made "-" as
            // copy names have it.
            let name = info.attribute_as_string("standard::copy-name").or_else(|| {
                info.attribute_as_string("standard::display-name")
                    .map(|title| title.replace('/', "-").into())
            })?;
            let is_one_component = !name.is_empty() && !name.contains('/') && name != "." && name != "..";
            is_one_component.then(|| OsString::from(name.as_str()))
        };
        self.copy_name.get_or_init(read).as_ref()
    }

    /// True for an item on a phone or another MTP device.
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

    /// OPS-035: refuses filesystem roots, whole shares and whole devices, like
    /// `require_item_uri` in `v2.0.0:desktop/core.py`.
    fn require_item(&self) -> Result<(), TransferError> {
        if self.file.parent().is_none() {
            return Err(TransferError::failed(crate::i18n::gettext(
                "Filesystem roots cannot be changed as items.",
            )));
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
        // A Google Drive address ends in an ID; its copy takes the title.
        if let Some(name) = self.copy_name() {
            return name.clone();
        }
        // Only a URI without a path has no base name; its URI is the only
        // name left to show.
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
        check_cancelled(cancel)?;
        // Follows links on purpose: a destination reached through a link to
        // a folder is a folder, as in the Python app.
        let info = self.file.query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            gio_cancellable(cancel),
        )?;
        Ok(info.file_type() == gio::FileType::Directory)
    }

    fn children(&self, cancel: Option<&Cancellation>) -> Result<Vec<Box<dyn Node>>, TransferError> {
        self.list_children(cancel)
    }

    fn create_directory(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        check_cancelled(cancel)?;
        // XFER-002: `make_directory` is exclusive and fails on a taken name.
        self.file.make_directory(gio_cancellable(cancel))?;
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
        // OPS-047: `gvfsd-admin` never opens the source by its path.
        if into_admin::crosses_into_admin(&self.file, &target_file) {
            return into_admin::copy_into_admin(&self.file, &target_file, cancel, progress);
        }
        let mut report = |current: i64, total: i64| progress(byte_count(current), byte_count(total));
        // XFER-002: without OVERWRITE the copy refuses an existing target,
        // which keeps the engine's "never overwrite" rule for staging and
        // uploads. XFER-017: a link is copied as a link.
        self.file.copy(
            &target_file,
            gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
            Some(&mut report),
        )?;
        Ok(())
    }

    fn move_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.refuse_move_into_admin(target)?;
        self.move_item(target, cancel, move_item::Overwrite::Never)
    }

    fn publish(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.publish_item(target, cancel)
    }

    fn replace_native(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.refuse_move_into_admin(target)?;
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
            gio_cancellable(cancel),
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

    fn delete_tree(&self, cancel: &Cancellation, guard: Option<&WriteGuard>) -> Result<(), TransferError> {
        self.delete_item_tree(None, None, cancel, guard)
    }

    fn delete_tree_of_kind(
        &self,
        kind: NodeKind,
        cancel: &Cancellation,
        guard: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        self.delete_item_tree(Some(kind), None, cancel, guard)
    }

    fn delete_selected_tree(
        &self,
        selected: &NodeInfo,
        cancel: &Cancellation,
        guard: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        self.delete_item_tree(Some(selected.kind), selected.identity, cancel, guard)
    }

    fn delete_staging(&self, created: Option<ItemIdentity>) -> Result<(), TransferError> {
        // Remote staging can only be removed by path, as in the Python app.
        match self.local_path() {
            Some(path) => local_delete::delete_staging(&path, created),
            None => clean_staging(self),
        }
    }

    fn has_sibling_staging(&self) -> bool {
        self.is_mtp()
    }

    fn native_copy_keeps_name(&self, target_folder: &dyn Node) -> bool {
        let target_uri = target_folder.uri();
        let target_is_mtp = gio::File::for_uri(&target_uri).has_uri_scheme("mtp");
        self.is_mtp() && target_is_mtp && same_authority(&self.uri(), &target_uri)
    }

    fn refresh_listing(&self, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        if self.is_mtp() {
            self.list_children(cancel)?;
        }
        Ok(())
    }

    fn filesystem(&self, cancel: Option<&Cancellation>) -> Option<FilesystemInfo> {
        let filesystem = self
            .file
            .query_filesystem_info("filesystem::type,filesystem::free", gio_cancellable(cancel))
            .ok();
        let id = self
            .file
            .query_info(
                "id::filesystem",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio_cancellable(cancel),
            )
            .ok()
            .and_then(|info| info.attribute_string("id::filesystem"))
            .map(String::from);
        let kind = filesystem
            .as_ref()
            .and_then(|info| info.attribute_string("filesystem::type"))
            .map(String::from);
        let free = filesystem
            .as_ref()
            .filter(|info| info.has_attribute("filesystem::free"))
            .map(|info| info.attribute_uint64("filesystem::free"));
        // Nothing reported when both queries failed.
        (filesystem.is_some() || id.is_some()).then_some(FilesystemInfo { kind, free, id })
    }
}

/// True when both URIs name the same host or device, like comparing
/// `uri.split('/')[2]` in `v2.0.0:desktop/gio_backend.py`.
fn same_authority(first: &str, second: &str) -> bool {
    match (split_location(first), split_location(second)) {
        (Ok(first), Ok(second)) => first.authority == second.authority,
        _ => false,
    }
}

/// A size or byte count from GIO. GIO never reports a negative one; a
/// broken backend's is shown as zero.
fn byte_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// The GIO cancellable behind an optional cancellation, for GIO calls.
fn gio_cancellable(cancel: Option<&Cancellation>) -> Option<&gio::Cancellable> {
    cancel.map(Cancellation::cancellable)
}
