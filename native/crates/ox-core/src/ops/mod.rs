// SPDX-License-Identifier: AGPL-3.0-only
//! File operations as the interface starts them: New, Rename, paste
//! conflicts, Delete, copy, move, Duplicate, the Recycle Bin, Undo, and
//! moving tabs between windows.
//!
//! Ports the file-operation branches of `dispatch` in
//! `desktop/winspace.py` (`create`, `rename`, `templates`,
//! `createTemplate`, `transferConflicts`, `operate`, `trashSupport`), the
//! template half of `desktop/file_services.py`, `create_item`,
//! `rename_item` and `trash_support` in `desktop/gio_backend.py`,
//! `desktop/tab_transfers.py`, and the delete confirmation, progress and
//! completion text of `desktop/ui/app.js`. Copies, moves, Trash and
//! permanent deletion run on the transfer engine of [`crate::transfer`]
//! with all of its safety rules.
//!
//! Beyond the Python app, from the Dolphin baseline: Duplicate, the
//! Recycle Bin (list, restore, delete, empty), the created items of every
//! operation for selecting them, and an undo journal (see [`UndoRecord`]
//! for what can be undone).
//!
//! Every operation that touches the filesystem is an `async fn`: its
//! blocking GIO calls run on GIO's worker threads while the caller's main
//! loop keeps running, and it stops when the [`OperationContext`]'s
//! cancellation is cancelled. Progress callbacks run on the worker thread.
//! Nothing here depends on GTK. The interface keeps the rule of one file
//! operation at a time (OPS-024) and cancels the running one through the
//! clone of its cancellation it holds, so the Python bridge's job tokens
//! (the `cancel` request) have no counterpart here.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `context` | The cancellation and write protection of an operation, and its worker |
//! | `error` | [`OpsError`] and the bridge's error codes |
//! | `create` | New folder and New file |
//! | `templates` | The built-in and user templates New offers |
//! | `new_from_template` | New from template, staged privately and published without overwriting |
//! | `rename` | Rename, and renaming back for Undo |
//! | `write_check` | The write-protection walk before a rename |
//! | `conflicts` | The name-conflict check before a paste or drop |
//! | `delete_plan` | Trash support and the Delete confirmation |
//! | `run_transfer` | Copy, move, Trash and permanent delete through the engine |
//! | `destinations` | Where a copy's or move's items are now |
//! | `duplicate` | Duplicate in place |
//! | `folder_groups` | Items grouped by folder, for per-folder runs of the engine |
//! | `results` | Adding up per-item results into one result |
//! | `progress` | Progress labels and throttling |
//! | `report` | The toast or result dialog at the end |
//! | `recycle_bin` | Listing, restoring, deleting and emptying `trash:///` |
//! | `undo` | The undo journal |
//! | `undo_apply` | Carrying out an Undo |
//! | `tab_transfer` | Moving a tab to another window |
//! | `random` | Unpredictable stage and capability names |
//!
//! The tests of this service in `desktop/tests` are ported to
//! `tests/ops_*.rs`, each naming the test it comes from;
//! `ops_tab_transfer.rs` names the two that the typed interface makes
//! unnecessary. Not ported here:
//! `properties`, `list_applications`, `prepare_launch` and
//! `SnapshotProvider` of `desktop/file_services.py`, which belong to the
//! Properties and Open with services.

mod conflicts;
mod context;
mod create;
mod delete_plan;
mod destinations;
mod duplicate;
mod error;
mod folder_groups;
mod new_from_template;
mod progress;
mod random;
mod recycle_bin;
mod rename;
mod report;
mod results;
mod run_transfer;
mod tab_transfer;
mod templates;
mod undo;
mod undo_apply;
mod write_check;

pub use conflicts::find_conflicts;
pub use context::{OperationContext, WriteProtection};
pub use create::{create_item, CreatedItem};
pub use delete_plan::{
    delete_command_label, plan_delete, trash_support, DeleteConfirmation, DeleteItem, DeletePlan,
};
pub use duplicate::duplicate_items;
pub use error::OpsError;
pub use new_from_template::{create_from_template, NewFromTemplate};
pub use progress::{starting_label, PROGRESS_INTERVAL};
pub use recycle_bin::{
    delete_from_recycle_bin, empty_recycle_bin, list_recycle_bin, recycle_bin_item_count,
    restore_from_recycle_bin, RecycledItem,
};
pub use rename::{rename_item, RenamedItem};
pub use report::{summarize, summarize_undo, OperationSummary, RESULT_TITLE, STOPPED_TITLE};
pub use run_transfer::{run_transfer, TransferOutcome, TransferRequest};
pub use tab_transfer::{
    Acceptance, Delivery, KeptReason, TabMessage, TabMoveOutcome, TabTransferError, TabTransferToken,
    TabTransfers, WindowId, MAX_PENDING_TAB_TRANSFERS, TAB_TRANSFER_LIFETIME,
};
pub use templates::{
    list_templates, BuiltinTemplate, Template, TemplateId, TemplateList, MAX_TEMPLATE_BYTES,
    MAX_USER_TEMPLATES,
};
pub use undo::{MovedItem, UndoJournal, UndoRecord, UNDO_LIMIT};
pub use undo_apply::undo;
