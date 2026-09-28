// SPDX-License-Identifier: AGPL-3.0-only
//! XFER-020 for a rename: every location in the renamed tree, and its new
//! location, is checked against the write protection before anything
//! changes, so a protected previous version deep inside a folder stops the
//! rename of the whole folder.
//!
//! Ports `TransferEngine._check_write_tree` of `desktop/operations.py`
//! with `source_writable=True`, which `rename_item` in
//! `desktop/gio_backend.py` runs. The transfer engine keeps its own copy of
//! this walk private; this one differs only in reporting [`OpsError`].

use super::context::WriteProtection;
use super::error::OpsError;
use crate::transfer::{nesting_error, Cancellation, Node, NodeKind, MAX_DEPTH};

/// Asks `protection` about `source`, `destination` and every pair below
/// them, without following links and without changing anything. Without a
/// protection this does nothing, like the Python engine without a guard.
///
/// # Errors
///
/// The protection's refusal for the first protected location, the
/// nesting limit, cancellation, or a failure to inspect or list the tree.
pub(crate) fn check_renamed_tree(
    protection: &WriteProtection,
    source: &dyn Node,
    destination: &dyn Node,
    cancel: &Cancellation,
) -> Result<(), OpsError> {
    if !protection.is_restricted() {
        return Ok(());
    }
    let walk = RenamedTree { protection, cancel };
    walk.check(source, destination, 0)
}

/// One preflight walk over a renamed tree.
struct RenamedTree<'a> {
    protection: &'a WriteProtection,
    cancel: &'a Cancellation,
}

impl RenamedTree<'_> {
    /// Checks `source` (at nesting `depth`), its new location
    /// `destination`, and everything below them.
    fn check(&self, source: &dyn Node, destination: &dyn Node, depth: usize) -> Result<(), OpsError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error().into());
        }
        self.protection.check(&source.uri())?;
        self.protection.check(&destination.uri())?;
        // A link to a protected folder is checked as the link itself; its
        // target is never walked.
        if source.info(Some(self.cancel))?.kind != NodeKind::Directory {
            return Ok(());
        }
        for child in source.children(Some(self.cancel))? {
            let child_destination = destination.child(&child.name());
            self.check(child.as_ref(), child_destination.as_ref(), depth + 1)?;
        }
        Ok(())
    }
}
