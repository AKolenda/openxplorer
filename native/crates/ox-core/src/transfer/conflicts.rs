// SPDX-License-Identifier: AGPL-3.0-only
//! Choosing the name an item gets in the destination folder when that name
//! may be taken: Skip, Keep both or Replace, and moves into the folder an
//! item is already in. Ports the conflict branch of `_run_items` in
//! `desktop/operations.py`.

use std::ffi::OsStr;

use super::error::TransferError;
use super::names::child_node;
use super::node::{Cancellation, Node, NodeKind};
use super::types::{ConflictPolicy, TransferMode};
use crate::location::try_new_copy_name;

/// "Keep both" tries `(copy 2)` up to `(copy 9999)`, like the Python app,
/// then gives up.
const MAX_COPY_NUMBER: u32 = 10_000;

/// Where the items of one copy or move go.
pub(crate) struct Placement<'a> {
    pub(crate) mode: TransferMode,
    pub(crate) policy: ConflictPolicy,
    pub(crate) dest_dir: &'a dyn Node,
    pub(crate) cancel: &'a Cancellation,
}

impl Placement<'_> {
    /// The item `source` (of `kind`) becomes in the destination folder, or
    /// `None` when it is skipped: a move into the folder it is already in,
    /// or a taken name with Skip.
    ///
    /// # Errors
    ///
    /// An invalid name, a failed Keep-both search, or
    /// [`TransferError::Cancelled`].
    pub(crate) fn destination_for(
        &self,
        source: &dyn Node,
        kind: NodeKind,
    ) -> Result<Option<Box<dyn Node>>, TransferError> {
        let source_name = source.name();
        let destination = child_node(self.dest_dir, &source_name)?;
        // Moving an item into its own folder would change nothing; with Keep
        // both it would even rename the user's item.
        if self.mode == TransferMode::Move && destination.uri() == source.uri() {
            return Ok(None);
        }
        if !destination.exists(Some(self.cancel)) {
            return Ok(Some(destination));
        }
        match self.policy {
            // Skip never touches the existing item.
            ConflictPolicy::Skip => Ok(None),
            ConflictPolicy::KeepBoth => self.free_copy_name(&source_name, kind).map(Some),
            ConflictPolicy::Replace => Ok(Some(destination)),
        }
    }

    /// The first free Windows-style duplicate name, starting at `(copy 2)`.
    fn free_copy_name(&self, source_name: &OsStr, kind: NodeKind) -> Result<Box<dyn Node>, TransferError> {
        // Duplicate names are text. A name that is not UTF-8 is refused
        // rather than given a lossily converted "(copy N)" name.
        let Some(source_name) = source_name.to_str() else {
            return Err(TransferError::failed(
                "This item's name is not valid UTF-8, so no duplicate name can be made. \
                 Rename it before choosing Keep both.",
            ));
        };
        let is_folder = kind == NodeKind::Directory;
        for number in 2..MAX_COPY_NUMBER {
            self.cancel.check()?;
            let name = try_new_copy_name(source_name, number, is_folder)?;
            let candidate = child_node(self.dest_dir, &name)?;
            if !candidate.exists(Some(self.cancel)) {
                return Ok(candidate);
            }
        }
        Err(TransferError::failed(
            "Too many duplicate names. Rename the item before copying.",
        ))
    }
}
