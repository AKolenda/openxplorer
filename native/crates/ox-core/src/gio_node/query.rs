// SPDX-License-Identifier: AGPL-3.0-only
//! Metadata and enumeration without following symbolic links.
//!
//! Ports `GioNode.info` and `GioNode.children` in `desktop/gio_backend.py`.

use std::time::{Duration, SystemTime};

use gio::prelude::*;

use super::{byte_count, gio_cancellable, GioNode};
use crate::transfer::{check_cancelled, Cancellation, ItemIdentity, Node, NodeInfo, NodeKind, TransferError};

/// The attributes [`GioNode::query_info`] reads.
const INFO_ATTRIBUTES: &str =
    "standard::type,standard::size,unix::mode,time::modified,time::modified-usec,unix::device,unix::inode";

/// The permission bits of `unix::mode`, without the file type bits.
const PERMISSION_BITS: u32 = 0o7777;

impl GioNode {
    /// Kind, size and mode of this item, not of a link's target (XFER-017).
    ///
    /// # Errors
    ///
    /// [`TransferError::Cancelled`] when `cancel` was cancelled; the
    /// backend's error otherwise, with [`TransferError::NotFound`] only for
    /// a definite absence.
    pub(super) fn query_info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        check_cancelled(cancel)?;
        let info = self.file.query_info(
            INFO_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio_cancellable(cancel),
        )?;
        let mode = info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode") & PERMISSION_BITS);
        Ok(NodeInfo {
            kind: node_kind(info.file_type()),
            size: byte_count(info.size()),
            mode,
            modified: modified_time(&info),
            identity: identity(&info),
        })
    }

    /// The items of this folder. XFER-017: a link to a folder is refused,
    /// so the engine never walks into a link's target.
    ///
    /// # Errors
    ///
    /// [`TransferError::Failed`] when this item is not a real folder, and
    /// every error of [`Self::query_info`] and of the enumeration.
    pub(super) fn list_children(
        &self,
        cancel: Option<&Cancellation>,
    ) -> Result<Vec<Box<dyn Node>>, TransferError> {
        if self.query_info(cancel)?.kind != NodeKind::Directory {
            return Err(TransferError::failed(
                "Only real folders can be enumerated during a transfer.",
            ));
        }
        let files = enumerate_files(&self.file, cancel)?;
        let children = files
            .into_iter()
            .map(|file| Box::new(Self::from_file(file)) as Box<dyn Node>)
            .collect();
        Ok(children)
    }
}

/// The modification time GIO reports, to the microsecond.
fn modified_time(info: &gio::FileInfo) -> Option<SystemTime> {
    if !info.has_attribute("time::modified") {
        return None;
    }
    let seconds = info.attribute_uint64("time::modified");
    let microseconds = info.attribute_uint32("time::modified-usec");
    let since_epoch = Duration::from_secs(seconds) + Duration::from_micros(u64::from(microseconds));
    SystemTime::UNIX_EPOCH.checked_add(since_epoch)
}

/// The local object GIO reports (`st_dev`, `st_ino`), on local file systems.
fn identity(info: &gio::FileInfo) -> Option<ItemIdentity> {
    (info.has_attribute("unix::device") && info.has_attribute("unix::inode")).then(|| ItemIdentity {
        device: u64::from(info.attribute_uint32("unix::device")),
        inode: info.attribute_uint64("unix::inode"),
    })
}

/// The kind of item GIO reports, without following links.
fn node_kind(file_type: gio::FileType) -> NodeKind {
    match file_type {
        gio::FileType::Directory => NodeKind::Directory,
        gio::FileType::Regular => NodeKind::File,
        gio::FileType::SymbolicLink => NodeKind::Symlink,
        _ => NodeKind::Special,
    }
}

/// Every item in `folder`, including hidden ones, without following links.
/// The listing is closed on success, cancellation and error alike.
///
/// # Errors
///
/// The folder cannot be listed or the listing cannot be closed, or
/// [`TransferError::Cancelled`].
pub(super) fn enumerate_files(
    folder: &gio::File,
    cancel: Option<&Cancellation>,
) -> Result<Vec<gio::File>, TransferError> {
    let enumerator = folder.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio_cancellable(cancel),
    )?;
    let listed = collect_files(&enumerator, cancel);
    // Preserve the enumeration error if closing fails as well.
    let (closed, close_error) = enumerator.close(gio::Cancellable::NONE);
    let files = listed?;
    if let Some(error) = close_error {
        return Err(error.into());
    }
    if !closed {
        return Err(TransferError::failed("The folder listing could not be closed."));
    }
    Ok(files)
}

/// Reads every item of an open listing.
fn collect_files(
    enumerator: &gio::FileEnumerator,
    cancel: Option<&Cancellation>,
) -> Result<Vec<gio::File>, TransferError> {
    let mut files = Vec::new();
    loop {
        check_cancelled(cancel)?;
        let Some(info) = enumerator.next_file(gio_cancellable(cancel))? else {
            return Ok(files);
        };
        files.push(enumerator.child(&info));
    }
}
