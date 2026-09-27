// SPDX-License-Identifier: AGPL-3.0-only
//! Removing the engine's own staging after a failed or cancelled copy.
//!
//! Ports `_discard_stage`, `_confirmed_absent` and `_clean_staging` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - Only a staging item this engine created (an exclusive `mkdir`, or a
//!   `copy_file` to a free random name) is ever removed. Callers must never
//!   pass a user-selected path.
//! - Removal inspects items without following symbolic links, so a link
//!   inside staging is removed as a link and its target is never touched.
//! - Every cleanup failure is returned, so the caller can report the exact
//!   leftover location for the user to inspect.

use std::time::Duration;

use super::guard::{nesting_error, MAX_DEPTH};
use super::modes::secure_local_staging;
use super::node::{Node, NodeKind, TransferError};

/// Waits before each cleanup attempt on a device. Phones can reject the
/// first request after an aborted transfer, so device staging is retried
/// after half a second and again after one and a half seconds.
const DEVICE_CLEANUP_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(500),
    Duration::from_millis(1500),
];

/// Local and network staging gets exactly one cleanup attempt.
const LOCAL_CLEANUP_DELAYS: [Duration; 1] = [Duration::ZERO];

/// Recursively removes a staging tree the engine exclusively created.
///
/// Folders are made owner-writable first (a restored restrictive mode must
/// not block cleanup), then emptied, then removed. Also used by the ZIP
/// extractor for its own staging folder. Never call this on a user-selected
/// path.
pub fn clean_staging(node: &dyn Node) -> Result<(), TransferError> {
    clean_at_depth(node, 0)
}

fn clean_at_depth(node: &dyn Node, depth: usize) -> Result<(), TransferError> {
    // The private staging folder and payload add two levels to the source
    // tree. An unexpected deeper backend tree must not exhaust the stack.
    if depth > MAX_DEPTH + 2 {
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

/// Removes this engine's own staging item and returns any problem.
///
/// Local and network staging gets one attempt, and every failure is
/// reported. Device staging is retried (see [`DEVICE_CLEANUP_DELAYS`]); a
/// device stage that is definitely missing (an aborted upload the device
/// discarded) needs no cleanup. Any other query error counts as a failed
/// attempt.
pub(crate) fn discard_stage(
    stage: &dyn Node,
    device: bool,
    sleep: &dyn Fn(Duration),
) -> Option<TransferError> {
    let delays: &[Duration] = if device {
        &DEVICE_CLEANUP_DELAYS
    } else {
        &LOCAL_CLEANUP_DELAYS
    };
    let mut problem = None;
    for delay in delays {
        if !delay.is_zero() {
            sleep(*delay);
        }
        if let Err(error) = stage.info(None) {
            if device && error.is_not_found() && confirmed_absent(stage) {
                return None;
            }
            problem = Some(error);
            continue;
        }
        match clean_staging(stage) {
            Ok(()) => return None,
            Err(error) => problem = Some(error),
        }
    }
    problem
}

/// GVfs MTP answers "not found" for an uncached path it failed to look up,
/// so only a successful listing of the parent without the stage proves the
/// stage is gone.
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
