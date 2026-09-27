// SPDX-License-Identifier: AGPL-3.0-only
//! Metadata and enumeration without following links.

use gio::prelude::*;

use super::{check, raw, GioNode};
use crate::transfer::{Cancellation, Node, NodeInfo, NodeKind, TransferError};

impl GioNode {
    pub(super) fn query_info(&self, cancel: Option<&Cancellation>) -> Result<NodeInfo, TransferError> {
        check(cancel)?;
        self.require_utf8_name()?;
        let info = self.file.query_info(
            "standard::type,standard::size,unix::mode",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            raw(cancel),
        )?;
        let kind = match info.file_type() {
            gio::FileType::Directory => NodeKind::Directory,
            gio::FileType::Regular => NodeKind::File,
            gio::FileType::SymbolicLink => NodeKind::Symlink,
            _ => NodeKind::Special,
        };
        let mode = info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode") & 0o7777);
        Ok(NodeInfo {
            kind,
            size: info.size().max(0) as u64,
            mode,
        })
    }

    pub(super) fn list_children(
        &self,
        cancel: Option<&Cancellation>,
    ) -> Result<Vec<Box<dyn Node>>, TransferError> {
        if self.query_info(cancel)?.kind != NodeKind::Directory {
            return Err(TransferError::failed(
                "Only real folders can be enumerated during a transfer.",
            ));
        }
        let enumerator = self.file.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            raw(cancel),
        )?;
        let result = self.collect_children(&enumerator, cancel);
        // Closing must happen on success, cancellation and enumeration error.
        // Preserve the enumeration error if both operations fail.
        let (closed, close_error) = enumerator.close(gio::Cancellable::NONE);
        let children = result?;
        if let Some(error) = close_error {
            return Err(error.into());
        }
        if !closed {
            return Err(TransferError::failed("The folder listing could not be closed."));
        }
        Ok(children)
    }

    fn collect_children(
        &self,
        enumerator: &gio::FileEnumerator,
        cancel: Option<&Cancellation>,
    ) -> Result<Vec<Box<dyn Node>>, TransferError> {
        let mut children: Vec<Box<dyn Node>> = Vec::new();
        loop {
            check(cancel)?;
            let Some(info) = enumerator.next_file(raw(cancel))? else {
                break;
            };
            let child = Self::from_file(enumerator.child(&info));
            child.require_utf8_name()?;
            children.push(Box::new(child));
        }
        Ok(children)
    }

    fn require_utf8_name(&self) -> Result<(), TransferError> {
        if self.file.basename().is_some_and(|name| name.to_str().is_none()) {
            // Node names are strings. Never silently replace invalid bytes
            // and publish a copy under a different name.
            return Err(TransferError::failed(
                "This item's name is not valid UTF-8. Rename it before transferring.",
            ));
        }
        Ok(())
    }
}
