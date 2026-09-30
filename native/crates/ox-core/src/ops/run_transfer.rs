// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move, Trash and permanent delete as the interface starts them.
//!
//! Ports the `operate` branch of `dispatch` in `desktop/winspace.py`: the
//! request is checked before anything changes (whole shares and devices
//! are refused, OPS-035; the destination and every source a mode changes
//! must not be protected, XFER-020; a server listing is no destination,
//! OPS-036), then the transfer engine runs it on a worker with throttled
//! progress. The outcome adds what the Python bridge could not report:
//! where the items are now (SEL-016) and how Undo reverses the run
//! (OPS-029).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::context::{on_worker, OperationContext, WriteProtection};
use super::error::OpsError;
use super::progress::throttled;
use super::undo::{MovedItem, UndoRecord};
use crate::gio_node::GioNode;
use crate::location::{is_smb_server, normalise, require_item_uri};
use crate::transfer::{
    ConflictPolicy, Landed, Node, NodeFactory, Operation, Progress, TransferEngine, TransferMode,
    TransferResult,
};

/// One copy, move, Trash or permanent delete, in the app's protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    /// What to do with the items.
    pub mode: TransferMode,
    /// The selected items.
    pub uris: Vec<String>,
    /// The folder copies and moves go into; Trash and delete ignore it.
    pub destination_folder: Option<String>,
    /// What happens when a name is taken; Trash and delete ignore it.
    pub policy: ConflictPolicy,
}

/// What a finished run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferOutcome {
    /// Every item's outcome, as the transfer engine reports it.
    pub result: TransferResult,
    /// Where finished copies and moves are now, to select them. Items whose
    /// new name is not known exactly are left out.
    pub created: Vec<String>,
    /// How Undo reverses the run, when it can.
    pub undo: Option<UndoRecord>,
}

/// Runs `request`, sending throttled progress to `progress` on the worker
/// thread (forward it to the main loop, for example with
/// `glib::MainContext::invoke`). Cancel through `context`; items already
/// finished stay finished.
///
/// # Errors
///
/// A request refused before anything changed: a share or device root, a
/// protected destination or source, a server listing as destination ("Open
/// a network share before pasting files."), or what the transfer engine
/// refuses (no items, too many, no destination folder). Failures of single
/// items are reported in [`TransferOutcome::result`].
pub async fn run_transfer(
    request: &TransferRequest,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || run_transfer_blocking(&request, &context, progress)).await
}

/// [`run_transfer`] on the calling thread.
fn run_transfer_blocking(
    request: &TransferRequest,
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> Result<TransferOutcome, OpsError> {
    let mut engine = gio_transfer_engine(context, progress);
    run_on_engine(&mut engine, request, context)
}

/// Checks `request` and runs it on `engine`, which a caller running
/// several requests as one operation reuses, so their progress reaches
/// one sink.
pub(super) fn run_on_engine(
    engine: &mut TransferEngine,
    request: &TransferRequest,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    // OPS-035: the Python bridge refuses the whole request, before any
    // item changes, when one of them is a share or device root.
    let items = request
        .uris
        .iter()
        .map(|uri| require_item_uri(uri))
        .collect::<Result<Vec<String>, _>>()?;
    let destination = checked_destination(request, &context.protection)?;
    if changes_sources(request.mode) {
        for item in &items {
            context.protection.check(item)?;
        }
    }
    let operation = Operation::from_request(request.mode, destination.as_deref(), request.policy)?;
    let tracking = RunTracking::before_run(operation);
    let result = engine.run(operation, &items, &context.cancel)?;
    Ok(tracking.finish(result))
}

/// The canonical destination folder of a copy or move, checked against
/// the protection and refused when it is a server listing. An empty
/// folder counts as none, as in the Python bridge.
fn checked_destination(
    request: &TransferRequest,
    protection: &WriteProtection,
) -> Result<Option<String>, OpsError> {
    let named_folder = request
        .destination_folder
        .as_deref()
        .filter(|folder| !folder.is_empty());
    let Some(folder) = named_folder else {
        return Ok(None);
    };
    let folder = normalise(folder)?;
    protection.check(&folder)?;
    // OPS-036: a server listing holds shares, which are not folders to
    // paste into.
    if is_smb_server(&folder) {
        return Err(OpsError::failed("Open a network share before pasting files."));
    }
    Ok(Some(folder))
}

/// True for the modes that change their sources, whose items the write
/// protection must allow (XFER-020).
fn changes_sources(mode: TransferMode) -> bool {
    matches!(
        mode,
        TransferMode::Move | TransferMode::Trash | TransferMode::Delete
    )
}

/// A transfer engine over the production GIO adapter, with the write
/// guard and questions of `context` and `progress` throttled to
/// [`PROGRESS_INTERVAL`].
///
/// [`PROGRESS_INTERVAL`]: super::progress::PROGRESS_INTERVAL
pub(crate) fn gio_transfer_engine(
    context: &OperationContext,
    progress: impl FnMut(Progress) + Send + 'static,
) -> TransferEngine {
    let factory: NodeFactory = Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>));
    let engine = TransferEngine::new(factory).with_progress(throttled(progress));
    context.install(engine)
}

/// What a run must remember before it starts, to tell afterwards where
/// its items are and how to undo it.
enum RunTracking {
    /// A copy or move; the engine reports where its items landed.
    Transfer {
        mode: TransferMode,
        policy: ConflictPolicy,
    },
    /// Move to Trash, started at `since` (seconds since the Unix epoch).
    Trash { since: u64 },
    /// Permanent delete: nothing is left to select or bring back.
    Delete,
}

impl RunTracking {
    /// Prepares to track `operation`.
    fn before_run(operation: Operation<'_>) -> Self {
        match operation {
            Operation::Copy { policy, .. } | Operation::Move { policy, .. } => RunTracking::Transfer {
                mode: operation.mode(),
                policy,
            },
            Operation::Trash => RunTracking::Trash {
                since: unix_seconds_now(),
            },
            Operation::Delete => RunTracking::Delete,
        }
    }

    /// The outcome of the run that ended with `result`.
    fn finish(self, result: TransferResult) -> TransferOutcome {
        match self {
            RunTracking::Transfer { mode, policy } => {
                let landed = result.landed.clone();
                let created = landed.iter().map(|item| item.destination.clone()).collect();
                let undo = transfer_undo(mode, policy, landed);
                TransferOutcome {
                    result,
                    created,
                    undo,
                }
            }
            RunTracking::Trash { since } => {
                let undo = trash_undo(&result.done, since);
                TransferOutcome {
                    result,
                    created: Vec::new(),
                    undo,
                }
            }
            RunTracking::Delete => TransferOutcome {
                result,
                ..TransferOutcome::default()
            },
        }
    }
}

/// How Undo reverses a copy or move whose finished items are `landed`:
/// copies go to the Trash and moved items go back. Replace is not
/// undoable, because the replaced items are gone.
fn transfer_undo(mode: TransferMode, policy: ConflictPolicy, landed: Vec<Landed>) -> Option<UndoRecord> {
    if landed.is_empty() || policy == ConflictPolicy::Replace {
        return None;
    }
    match mode {
        TransferMode::Copy => {
            let copies = landed.into_iter().map(|item| item.destination).collect();
            Some(UndoRecord::Copy { copies })
        }
        TransferMode::Move => {
            let items = landed
                .into_iter()
                .map(|item| MovedItem {
                    original_uri: item.source,
                    moved_uri: item.destination,
                })
                .collect();
            Some(UndoRecord::Move { items })
        }
        TransferMode::Trash | TransferMode::Delete => None,
    }
}

/// How Undo restores the items `trashed` since `since`: by their local
/// paths, which the Recycle Bin records as their original locations.
fn trash_undo(trashed: &[String], since: u64) -> Option<UndoRecord> {
    let original_paths: Vec<_> = trashed
        .iter()
        .filter_map(|uri| GioNode::new(uri).path())
        .collect();
    if original_paths.is_empty() {
        return None;
    }
    Some(UndoRecord::Trash {
        original_paths,
        trashed_since: since,
    })
}

/// The current time in whole seconds since the Unix epoch, the precision
/// of a Recycle Bin item's deletion date.
pub(super) fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
