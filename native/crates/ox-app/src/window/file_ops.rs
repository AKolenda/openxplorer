// SPDX-License-Identifier: AGPL-3.0-only
//! File operations in the window: New, Rename, Duplicate, Delete, the
//! Recycle Bin, Cut, Copy and Paste with their name conflicts, and Undo
//! and Redo.
//!
//! Ports the file commands of `v2.0.0:desktop/ui/app.js` (`newItem`,
//! `newTemplateDialog`, `rename`, `trash`, `copySelection`, `paste`,
//! `transferWithConflicts`, `runOperation`, `updateTransfer`,
//! `updateToolbar` and the trash-support cache) on ox-core's
//! [`ops`](ox_core::ops) service, whose operations run their blocking I/O
//! on GIO's worker threads. The window awaits them on the main loop, so
//! browsing never freezes while one runs, and it runs one at a time
//! (OPS-024), with the transfer panel and Cancel.
//!
//! Gains over the Python app, from Windows 11 and the Dolphin baseline:
//! Rename edits the name in place with the name before its extension
//! selected, Shift+Delete deletes
//! permanently, Duplicate, the Recycle Bin's Restore and Empty, Keep both
//! and an answer per item for name conflicts, the created items are
//! selected afterwards, and Undo and Redo.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `running` | One operation at a time: the transfer panel, Cancel and the report at the end |
//! | `unfinished` | Marks of running copies, and what a crashed run left behind |
//! | `availability` | When each file command is enabled (`updateToolbar`) |
//! | `trash_support` | Whether each folder has a Trash, which labels Delete |
//! | `names` | The name check of the name dialogs (`validateName`) |
//! | `name_dialog` | The dialog that asks for a name (`nameDialog`) |
//! | `new_items` | New folder, and New file from a template |
//! | `template_dialog` | The New file and New from template dialog |
//! | `rename` | Rename: in place, or with the dialog |
//! | `inline_rename` | Renaming in the item's row or tile |
//! | `batch_rename` | Renaming several items to one numbered name |
//! | `hide_confirm` | Asking before a rename hides an item |
//! | `delete` | Move to Trash and permanent delete, with their confirmations |
//! | `recycle_bin` | Restore, Delete permanently and Empty in the Recycle Bin |
//! | `duplicate` | Duplicate |
//! | `journal` | Undo and Redo |
//! | `links` | Create links, from a drop |
//! | `clipboard` | Cut, Copy and the desktop's file clipboard |
//! | `transfer` | Pasting into a folder, with the name-conflict check |
//! | `conflict_dialog` | The name-conflict dialog |
//! | `conflict_compare` | The two items side by side in that dialog |
//! | `conflict_rename` | The new name typed in that dialog |
//! | `unstorable_dialog` | The question about names and links the destination cannot store |
//! | `shortcuts` | The file commands' keys, which text fields keep |
//! | `actions` | The window actions of these commands |

mod actions;
mod availability;
mod batch_rename;
mod clipboard;
mod conflict_compare;
mod conflict_dialog;
mod conflict_rename;
mod delete;
mod duplicate;
mod failure_dialog;
mod hide_confirm;
mod inline_rename;
mod journal;
mod links;
mod move_by_copying_dialog;
mod name_dialog;
mod names;
mod new_items;
mod recycle_bin;
mod rename;
mod running;
mod shortcuts;
mod template_dialog;
mod transfer;
mod trash_support;
mod unfinished;
mod unstorable_dialog;
mod worker_question;

use ox_core::clipboard::ClipboardFiles;
use ox_core::transfer::Cancellation;

pub(super) use availability::FileCommand;
pub(super) use transfer::IncomingItems;
pub(super) use trash_support::TrashSupport;

/// What the window's file operations remember between commands.
#[derive(Debug, Default)]
pub(crate) struct FileOperations {
    /// The cancellation of the operation that runs now; `None` while none
    /// runs (`state.operation` in app.js, OPS-024).
    running: Option<Cancellation>,
    /// Set while a paste checks its destination and asks about name
    /// conflicts (`state.transferPlanning`), so a second paste cannot
    /// start meanwhile.
    planning: bool,
    /// Whether each folder has a Trash, as far as known (CMD-003).
    trash_support: TrashSupport,
    /// The file list on the desktop's clipboard, as last read; `None`
    /// when it holds no files (`state.clipboard`, CLIP-009).
    clipboard: Option<ClipboardFiles>,
    /// How many times the clipboard changed while the window watched, so
    /// a read that an owner change overtook is dropped.
    clipboard_generation: u64,
    /// "Don't ask again" was checked when a rename hid an item (OPS-013).
    hiding_confirmed: bool,
    /// The item to rename in place next, once the rename Tab committed
    /// has finished (OPS-012).
    rename_next: Option<String>,
}

impl FileOperations {
    /// True while an operation runs.
    fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// True while an operation runs or a paste is being planned: no other
    /// file operation may start.
    fn is_busy(&self) -> bool {
        self.is_running() || self.planning
    }

    /// True while no operation runs or is being planned, so a drag or a
    /// drop may start.
    pub(crate) fn is_idle(&self) -> bool {
        !self.is_busy()
    }
}
