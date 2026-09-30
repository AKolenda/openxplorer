// SPDX-License-Identifier: AGPL-3.0-only
//! Removing the source of a move that was finished by copying (XFER-013).
//!
//! The copy records every item it copied. Once the copy is published, only
//! those items are removed, children before their folders, each with a
//! single-item delete: a file or link is unlinked and a folder is removed
//! only when it is empty. Anything another program put into the source
//! while it was being copied was never copied, so it stays where it is and
//! the user is told, as KIO does for Dolphin.

use super::error::TransferError;
use super::node::{Node, NodeKind, WriteGuard};

/// One source item the copy finished, recorded children first.
pub(crate) struct CopiedItem {
    /// The source item.
    pub(crate) node: Box<dyn Node>,
    /// What it is, as the copy found it.
    pub(crate) kind: NodeKind,
}

/// The items of a source that were not removed because they were not part
/// of the copy.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct KeptItems {
    /// Items that appeared in the source's folders during the copy.
    pub(crate) appeared: usize,
}

impl KeptItems {
    /// What the user is told about the kept items of the source at
    /// `location`, or `None` when nothing was kept.
    pub(crate) fn notice(&self, location: &str) -> Option<String> {
        match self.appeared {
            0 => None,
            1 => Some(format!(
                "1 item appeared in the source during the move and was kept at {location}."
            )),
            count => Some(format!(
                "{count} items appeared in the source during the move and were kept at {location}."
            )),
        }
    }
}

/// Removes the source `top` (of kind `top_kind`) of a published copy,
/// limited to the `copied` items below it. `guard` is asked about each
/// item before it is removed.
///
/// # Errors
///
/// The guard's refusal or the first item that cannot be inspected or
/// removed; items not yet reached are left in place.
pub(crate) fn remove_copied_source(
    top: &dyn Node,
    top_kind: NodeKind,
    copied: &[CopiedItem],
    guard: Option<&WriteGuard>,
) -> Result<KeptItems, TransferError> {
    let mut kept_folders: usize = 0;
    let mut remaining_entries = 0;
    let items = copied
        .iter()
        .map(|item| (item.node.as_ref(), item.kind))
        .chain([(top, top_kind)]);
    for (node, kind) in items {
        if let Some(guard) = guard {
            guard(&node.uri())?;
        }
        if kind == NodeKind::Directory {
            let entries = node.children(None)?.len();
            if entries > 0 {
                kept_folders += 1;
                remaining_entries += entries;
                continue;
            }
        }
        node.delete()?;
    }
    // Every kept folder below the top is itself an entry of a kept folder.
    let appeared = remaining_entries - kept_folders.saturating_sub(1);
    Ok(KeptItems { appeared })
}
