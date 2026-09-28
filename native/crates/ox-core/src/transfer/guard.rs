// SPDX-License-Identifier: AGPL-3.0-only
//! The checks that walk a whole tree before it changes: protected
//! (read-only) locations anywhere in an affected tree (XFER-020), and the
//! nesting depth limit every walk of the engine and its backends obeys.
//!
//! Ports `TransferEngine._check_write_tree` and the depth limit of
//! `desktop/operations.py`.

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::names::child_node;
use super::node::{Node, NodeKind, WriteGuard};

/// The deepest folder nesting the engine walks. Deeper trees are refused
/// before anything is changed (preflight) or while copying, so a runaway
/// tree (or a backend that reports a loop) cannot recurse without bound.
pub const MAX_DEPTH: usize = 128;

/// The error for a tree deeper than [`MAX_DEPTH`].
pub(crate) fn nesting_error() -> TransferError {
    TransferError::failed(format!(
        "Folder nesting exceeds this build’s safety limit ({MAX_DEPTH})."
    ))
}

/// What an operation does to its source tree, which decides whether the
/// write guard is asked about the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceChange {
    /// The source stays as it is (copy).
    Kept,
    /// The source is moved, renamed, trashed or deleted.
    Changed,
}

/// XFER-020 preflight for one top-level item: asks `guard` about every
/// affected URI before anything changes, so a protected descendant (for
/// example a `.snapshot` folder deep inside a selected folder) stops the
/// whole item.
///
/// The source tree is checked when `source_change` is
/// [`SourceChange::Changed`]; `destination` is checked in parallel with the
/// source tree (copy, move). Nothing is followed through symbolic links and
/// nothing is modified. Without a guard this does nothing, exactly like the
/// Python engine. The Python app also runs it before a rename (`rename_item`
/// in `desktop/gio_backend.py`).
///
/// # Errors
///
/// The guard's refusal for the first protected URI, the nesting limit,
/// [`TransferError::Cancelled`], or a failure to inspect or list the tree.
pub(crate) fn check_write_tree(
    guard: Option<&WriteGuard>,
    source: &dyn Node,
    destination: Option<&dyn Node>,
    cancel: &Cancellation,
    source_change: SourceChange,
) -> Result<(), TransferError> {
    let Some(guard) = guard else {
        return Ok(());
    };
    let preflight = WritePreflight {
        guard,
        cancel,
        source_change,
    };
    preflight.check_tree(source, destination, 0)
}

/// One preflight walk.
struct WritePreflight<'a> {
    guard: &'a WriteGuard,
    cancel: &'a Cancellation,
    source_change: SourceChange,
}

impl WritePreflight<'_> {
    /// Checks `source` (at nesting `depth`), its counterpart in the
    /// destination, and everything below them.
    fn check_tree(
        &self,
        source: &dyn Node,
        destination: Option<&dyn Node>,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        if self.source_change == SourceChange::Changed {
            (self.guard)(&source.uri())?;
        }
        if let Some(destination) = destination {
            (self.guard)(&destination.uri())?;
        }
        // Inspected without following links: a link to a protected folder is
        // checked as the link itself, and its target is never walked.
        if source.info(Some(self.cancel))?.kind != NodeKind::Directory {
            return Ok(());
        }
        for child in source.children(Some(self.cancel))? {
            let child_destination = match destination {
                Some(folder) => Some(child_node(folder, child.name())?),
                None => None,
            };
            self.check_tree(child.as_ref(), child_destination.as_deref(), depth + 1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: XFER-015
    #[test]
    fn nesting_error_names_the_limit() {
        assert_eq!(
            nesting_error(),
            TransferError::failed("Folder nesting exceeds this build’s safety limit (128).")
        );
    }
}
