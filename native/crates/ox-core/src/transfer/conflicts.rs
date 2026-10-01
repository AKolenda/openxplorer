// SPDX-License-Identifier: AGPL-3.0-only
//! Choosing the name an item gets in the destination folder when that name
//! may be taken: Skip, Keep both, Replace or a name the user typed, and
//! moves into the folder an item is already in. Ports the conflict branch
//! of `_run_items` in `v2.0.0:desktop/operations.py`.

use std::ffi::OsStr;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::names::child_node;
use super::node::{Node, NodeKind};
use super::types::{ConflictPolicy, TransferMode};
use crate::location::{new_copy_name, ItemKind};

/// XFER-008: "Keep both" tries `(copy 2)` up to `(copy 9999)`, like the
/// Python app, then gives up.
const MAX_COPY_NUMBER: u32 = 10_000;

/// Where the items of one copy or move go.
pub(crate) struct Placement<'a> {
    /// Copy or move; a move into the item's own folder is skipped.
    pub(crate) mode: TransferMode,
    /// What happens when the item's name is taken.
    pub(crate) policy: ConflictPolicy,
    /// The folder the items go into.
    pub(crate) destination_folder: &'a dyn Node,
    /// The name the item gets instead of its own, when the user answered
    /// a conflict with Rename; `None` keeps each item's name.
    pub(crate) name: Option<&'a OsStr>,
}

impl Placement<'_> {
    /// The destination `source` (of `kind`) gets in the destination folder
    /// when it arrives as `name` (or the name the user typed), or `None` when it is skipped: a move into
    /// the folder it is already in, or a taken name with Skip.
    ///
    /// The existence checks and the Keep-both search stop when the user
    /// cancels through `cancel`, the run's [`Batch`](super::batch::Batch)
    /// token.
    ///
    /// # Errors
    ///
    /// An invalid name, a failed Keep-both search, or
    /// [`TransferError::Cancelled`].
    pub(crate) fn destination_for(
        &self,
        source: &dyn Node,
        name: &OsStr,
        kind: NodeKind,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn Node>>, TransferError> {
        let destination = child_node(self.destination_folder, self.name.unwrap_or(name))?;
        // XFER-012: moving an item into its own folder would change nothing,
        // and with Keep both it would even rename the user's item, so it is
        // skipped.
        if self.mode == TransferMode::Move && destination.uri() == source.uri() {
            return Ok(None);
        }
        if !destination.exists(Some(cancel)) {
            return Ok(Some(destination));
        }
        // The user chose this name because the item's own was taken; it
        // was free then, so a clash now is reported, never resolved.
        if self.name.is_some() {
            return Err(TransferError::failed(
                "The new name is taken too. Choose another name.",
            ));
        }
        // OPS-028: Dolphin refuses to overwrite an item with itself.
        if self.policy == ConflictPolicy::Replace && destination.uri() == source.uri() {
            return Err(TransferError::failed("An item cannot replace itself."));
        }
        match self.policy {
            // XFER-006: Skip never touches the existing item.
            ConflictPolicy::Skip => Ok(None),
            ConflictPolicy::KeepBoth => self.free_copy_name(name, kind, cancel).map(Some),
            ConflictPolicy::Replace => Ok(Some(destination)),
        }
    }

    /// XFER-008: the first free Windows-style duplicate name, starting at
    /// `(copy 2)`.
    fn free_copy_name(
        &self,
        source_name: &OsStr,
        kind: NodeKind,
        cancel: &Cancellation,
    ) -> Result<Box<dyn Node>, TransferError> {
        // Duplicate names are text. A name that is not UTF-8 is refused
        // rather than given a lossily converted "(copy N)" name.
        let Some(source_name) = source_name.to_str() else {
            return Err(TransferError::failed(
                "This item's name is not valid UTF-8, so no duplicate name can be made. \
                 Rename it before choosing Keep both.",
            ));
        };
        let item_kind = if kind == NodeKind::Directory {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        for number in 2..MAX_COPY_NUMBER {
            cancel.check()?;
            let name = new_copy_name(source_name, number, item_kind)?;
            let candidate = child_node(self.destination_folder, &name)?;
            if !candidate.exists(Some(cancel)) {
                return Ok(candidate);
            }
        }
        Err(TransferError::failed(
            "Too many duplicate names. Rename the item before copying.",
        ))
    }
}
