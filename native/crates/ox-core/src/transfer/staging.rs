// SPDX-License-Identifier: AGPL-3.0-only
//! Removing the engine's own staging after a failed or cancelled copy.
//!
//! Ports `_discard_stage`, `_confirmed_absent` and `_clean_staging` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - XFER-002: only a staging item this engine created (an exclusive
//!   `create_directory`, or a `copy_file` to a free random name) is ever
//!   removed. Callers must never pass a user-selected path. Removal
//!   inspects items without following symbolic links, so a link inside
//!   staging is removed as a link and its target is never touched. A local
//!   staging folder is removed only while its name still leads to the
//!   folder the engine created (see [`Node::delete_staging`]).
//! - XFER-003: every cleanup failure is returned, so the caller can report
//!   the exact leftover location for the user to inspect.
//! - XFER-022: device staging is retried, and a device stage counts as gone
//!   only when a listing of its folder confirms it.

use std::time::Duration;

use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::modes::secure_local_staging;
use super::node::{ItemIdentity, Node, NodeKind};

/// The levels staging adds above a copied tree: the private staging folder
/// and the `payload` inside it.
pub(crate) const STAGING_LEVELS: usize = 2;

/// XFER-022: waits before each cleanup attempt on a device. Phones can
/// reject the first request after an aborted transfer, so device staging
/// is retried after half a second and again after one and a half seconds.
const DEVICE_CLEANUP_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(500),
    Duration::from_millis(1500),
];

/// Local and network staging gets exactly one cleanup attempt.
const LOCAL_CLEANUP_DELAYS: [Duration; 1] = [Duration::ZERO];

/// Where a stage lives, which decides how its cleanup is retried and how a
/// leftover is reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StagingPlace {
    /// A local or network folder: one cleanup attempt; a leftover is a
    /// staging "folder".
    #[default]
    LocalOrNetwork,
    /// A device (MTP): cleanup is retried; a leftover is a staging "item".
    Device,
}

impl StagingPlace {
    /// The place of staging created in `destination_folder`.
    pub(crate) fn of(destination_folder: &dyn Node) -> Self {
        if destination_folder.has_sibling_staging() {
            StagingPlace::Device
        } else {
            StagingPlace::LocalOrNetwork
        }
    }

    /// The waits before each cleanup attempt.
    fn cleanup_delays(self) -> &'static [Duration] {
        match self {
            StagingPlace::LocalOrNetwork => &LOCAL_CLEANUP_DELAYS,
            StagingPlace::Device => &DEVICE_CLEANUP_DELAYS,
        }
    }
}

/// XFER-003: the message for staging that could not be removed, with its
/// exact location, as `desktop/operations.py` reports it.
pub(crate) fn leftover_report(stage: &dyn Node, place: StagingPlace, problem: &TransferError) -> String {
    let what = match place {
        StagingPlace::LocalOrNetwork => "folder",
        StagingPlace::Device => "item",
    };
    format!(
        "Incomplete staging {what} left at {}. Inspect it before removing it. {problem}",
        stage.uri()
    )
}

/// Recursively removes a staging tree the engine exclusively created, by
/// path. This is the default of [`Node::delete_staging`].
///
/// Folders are made owner-writable first (a restored restrictive mode must
/// not block cleanup), then emptied, then removed. The Python ZIP extractor
/// (`desktop/zip_extraction.py`) cleans its own staging folder the same way;
/// its port will use this too. Never call this on a user-selected path.
///
/// # Errors
///
/// The first item that cannot be inspected, listed or removed; the rest of
/// the tree stays for the caller to report.
pub(crate) fn clean_staging(node: &(impl Node + ?Sized)) -> Result<(), TransferError> {
    clean_at_depth(node, 0)
}

/// Removes `node`, at nesting `depth` below the staging root, and
/// everything inside it.
fn clean_at_depth(node: &(impl Node + ?Sized), depth: usize) -> Result<(), TransferError> {
    // An unexpected deeper backend tree must not exhaust the stack.
    if depth > MAX_DEPTH + STAGING_LEVELS {
        return Err(nesting_error());
    }
    if node.info(None)?.kind == NodeKind::Directory {
        secure_local_staging(node)?;
        for child in node.children(None)? {
            clean_at_depth(child.as_ref(), depth + 1)?;
        }
    }
    node.delete()
}

/// Removes this engine's own staging item. `created` is its identity from
/// when the engine made it, if the backend has one.
///
/// Local and network staging gets one attempt, and every failure is
/// reported. Device staging is retried (see [`DEVICE_CLEANUP_DELAYS`]); a
/// device stage that is definitely missing (an aborted upload the device
/// discarded) needs no cleanup. Any other query error counts as a failed
/// attempt.
///
/// # Errors
///
/// The problem of the last attempt when the stage could not be removed.
pub(crate) fn discard_stage(
    stage: &dyn Node,
    created: Option<ItemIdentity>,
    place: StagingPlace,
    sleep: &dyn Fn(Duration),
) -> Result<(), TransferError> {
    let mut problem = TransferError::failed("The staging was not removed.");
    for delay in place.cleanup_delays() {
        if !delay.is_zero() {
            sleep(*delay);
        }
        match remove_stage(stage, created, place) {
            Ok(()) => return Ok(()),
            Err(error) => problem = error,
        }
    }
    Err(problem)
}

/// One cleanup attempt. A device stage that is definitely gone counts as
/// removed; any other query error is a failed attempt.
fn remove_stage(
    stage: &dyn Node,
    created: Option<ItemIdentity>,
    place: StagingPlace,
) -> Result<(), TransferError> {
    match stage.info(None) {
        Ok(_) => stage.delete_staging(created),
        Err(error) if is_discarded_device_stage(stage, place, &error) => Ok(()),
        Err(error) => Err(error),
    }
}

/// XFER-022: true for a device stage the device itself discarded (an aborted
/// upload): the query answered "not found" and a listing of its folder
/// confirms it.
fn is_discarded_device_stage(stage: &dyn Node, place: StagingPlace, query_error: &TransferError) -> bool {
    place == StagingPlace::Device && query_error.is_not_found() && confirmed_absent(stage)
}

/// XFER-022: `GVfs` MTP answers "not found" for an uncached path it failed
/// to look up, so only a successful listing of the parent without the
/// stage proves the stage is gone.
fn confirmed_absent(stage: &dyn Node) -> bool {
    let Some(parent) = stage.parent() else {
        return false;
    };
    let stage_name = stage.name();
    match parent.children(None) {
        Ok(children) => children.iter().all(|child| child.name() != stage_name),
        Err(_) => false,
    }
}
