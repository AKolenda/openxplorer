// SPDX-License-Identifier: AGPL-3.0-only
//! Metadata and enumeration without following symbolic links.
//!
//! Ports `GioNode.info` and `GioNode.children` in `desktop/gio_backend.py`.

use gio::prelude::*;

use super::{check, raw, GioNode};
use crate::transfer::{Cancellation, Node, NodeInfo, NodeKind, TransferError};

/// The attributes [`GioNode::query_info`] reads.
const INFO_ATTRIBUTES: &str = "standard::type,standard::size,unix::mode";

impl GioNode {
    /// Kind, size and mode of this item, not of a link's target.
    pub(super) fn query_info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        check(cancel)?;
        let info = self.file.query_info(
            INFO_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            raw(cancel),
        )?;
        let mode = info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode") & 0o7777);
        Ok(NodeInfo {
            kind: node_kind(info.file_type()),
            // GIO reports a negative size only for a broken backend.
            size: u64::try_from(info.size()).unwrap_or(0),
            mode,
        })
    }

    /// The items of this folder. A link to a folder is refused, so the
    /// engine never walks into a link's target.
    pub(super) fn list_children(
        &self,
        cancel: Option<&Cancellation>,
    ) -> Result<Vec<Box<dyn Node>>, TransferError> {
        if self.query_info(cancel)?.kind != NodeKind::Directory {
            return Err(TransferError::failed(
                "Only real folders can be enumerated during a transfer.",
            ));
        }
        let children = enumerate_files(&self.file, cancel)?
            .into_iter()
            .map(|file| Box::new(Self::from_file(file)) as Box<dyn Node>)
            .collect();
        Ok(children)
    }
}

/// The kind of item GIO reports, without following links.
pub(super) fn node_kind(file_type: gio::FileType) -> NodeKind {
    match file_type {
        gio::FileType::Directory => NodeKind::Directory,
        gio::FileType::Regular => NodeKind::File,
        gio::FileType::SymbolicLink => NodeKind::Symlink,
        _ => NodeKind::Special,
    }
}

/// Every item in `folder`, including hidden ones, without following links.
/// The listing is closed on success, cancellation and error alike.
pub(super) fn enumerate_files(
    folder: &gio::File,
    cancel: Option<&Cancellation>,
) -> Result<Vec<gio::File>, TransferError> {
    let enumerator = folder.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        raw(cancel),
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

fn collect_files(
    enumerator: &gio::FileEnumerator,
    cancel: Option<&Cancellation>,
) -> Result<Vec<gio::File>, TransferError> {
    let mut files = Vec::new();
    loop {
        check(cancel)?;
        let Some(info) = enumerator.next_file(raw(cancel))? else {
            return Ok(files);
        };
        files.push(enumerator.child(&info));
    }
}
