// SPDX-License-Identifier: AGPL-3.0-only
//! Removing the source of a move that was finished by copying (XFER-013).
//!
//! The copy records every item it copied together with what the item was
//! before its copy. Once the copy is published, only those items are
//! removed, children before their folders, each with a single-item delete:
//! a file or link is unlinked and a folder is removed only when it is
//! empty. Each item is inspected again right before it is removed, and one
//! that another program changed since the copy read it (saved over by a
//! rename, edited in place, or replaced by another folder) is kept, so its
//! newer content is never lost. Anything that appeared in the source while
//! it was copied was never copied, so it stays where it is too, and the
//! user is told, as KIO does for Dolphin.

use std::collections::HashSet;

use super::error::TransferError;
use super::node::{Node, NodeInfo, NodeKind, WriteGuard};

/// One source item the copy finished, recorded children first.
pub(crate) struct CopiedItem {
    /// The source item.
    pub(crate) node: Box<dyn Node>,
    /// What it was when the copy read it.
    pub(crate) info: NodeInfo,
}

/// The items of a source that were not removed because they were not part
/// of the copy.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct KeptItems {
    /// Items that appeared in the source's folders during the copy.
    pub(crate) appeared: usize,
    /// Copied items that changed after the copy read them.
    pub(crate) changed: usize,
    /// Items the copy left out because they could not be copied
    /// (OPS-047).
    pub(crate) left_out: usize,
}

impl KeptItems {
    /// What the user is told about the kept items of the source at
    /// `location`, or `None` when nothing was kept.
    pub(crate) fn notice(&self, location: &str) -> Option<String> {
        let mut parts = Vec::new();
        if self.left_out > 0 {
            parts.push(
                crate::i18n::ngettext(
                    "1 item could not be moved and was",
                    "{count} items could not be moved and were",
                    self.left_out as u64,
                )
                .replace("{count}", &self.left_out.to_string()),
            );
        }
        if self.changed > 0 {
            parts.push(
                crate::i18n::ngettext(
                    "1 item changed during the move and was",
                    "{count} items changed during the move and were",
                    self.changed as u64,
                )
                .replace("{count}", &self.changed.to_string()),
            );
        }
        if self.appeared > 0 {
            parts.push(
                crate::i18n::ngettext(
                    "1 item appeared in the source during the move and was",
                    "{count} items appeared in the source during the move and were",
                    self.appeared as u64,
                )
                .replace("{count}", &self.appeared.to_string()),
            );
        }
        (!parts.is_empty()).then(|| {
            crate::i18n::format_message(
                "{items} kept at {location}.",
                &[("items", &parts.join("; ")), ("location", location)],
            )
        })
    }
}

/// Removes the source `top`, which was `top_info` before the copy, of a
/// published copy, limited to the `copied` items below it. The items at
/// the `left_out` URIs, which could not be copied, stay with their
/// folders (OPS-047). `guard` is asked about each item before it is
/// removed.
///
/// # Errors
///
/// The guard's refusal or the first item that cannot be inspected or
/// removed; items not yet reached are left in place.
pub(crate) fn remove_copied_source(
    top: &dyn Node,
    top_info: &NodeInfo,
    copied: &[CopiedItem],
    left_out: &HashSet<String>,
    guard: Option<&WriteGuard>,
) -> Result<KeptItems, TransferError> {
    let mut kept_items = KeptItems {
        left_out: left_out.len(),
        ..KeptItems::default()
    };
    // The items kept so far, so the entries of a kept folder that are
    // already counted are not counted again as having appeared.
    let mut kept: HashSet<String> = left_out.clone();
    let items = copied
        .iter()
        .map(|item| (item.node.as_ref(), &item.info))
        .chain([(top, top_info)]);
    for (node, before) in items {
        if let Some(guard) = guard {
            guard(&node.uri())?;
        }
        let now = match node.info(None) {
            Ok(now) => now,
            // Another program removed it already: nothing is left to keep.
            Err(error) if error.is_not_found() => continue,
            Err(error) => return Err(error),
        };
        if !before.still_describes(&now) {
            kept_items.changed += 1;
            kept.insert(node.uri());
            continue;
        }
        if now.kind == NodeKind::Directory {
            let entries = node.children(None)?;
            if !entries.is_empty() {
                let new_entries = entries.iter().filter(|entry| !kept.contains(&entry.uri()));
                kept_items.appeared += new_entries.count();
                kept.insert(node.uri());
                continue;
            }
        }
        node.delete()?;
    }
    Ok(kept_items)
}
