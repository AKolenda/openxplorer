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
//! browsing stays responsive. `begin_operation` reserves the window for
//! an exclusive write; `begin_transfer` admits bounded concurrent jobs
//! when their source and destination paths do not overlap. Each job has
//! its own transfer panel and cancellation token (OPS-024, OPS-025).
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
//! | `running` | Exclusive operations, transfer progress, cancellation and completion reports |
//! | `progress` | Bounded, coalesced progress updates from worker threads |
//! | `jobs` | Bounded concurrent transfers, independent panels and overlapping-path refusal |
//! | `unfinished` | Marks of running copies, and what a crashed run left behind |
//! | `availability` | When each file command is enabled (`updateToolbar`) |
//! | `delete_label` | The name of Delete: "Delete", or "Delete permanently" in the Recycle Bin |
//! | `names` | The name check of the name dialogs (`validateName`) |
//! | `name_dialog` | The dialog that asks for a name (`nameDialog`) |
//! | `new_items` | New folder, and the New menu's documents and templates, made in place |
//! | `template_menu` | The user's templates in the New menu, subfolders as submenus |
//! | `rename` | Rename: in place, or with the dialog |
//! | `inline_rename` | Renaming in the item's row or tile |
//! | `name_balloon` | The characters Windows forbids, refused as they are typed in that field |
//! | `rename_conflict` | The "Rename File" question when a file takes a taken name |
//! | `batch_rename` | Rename items…: the dialog's `#` template for several items |
//! | `numbered_rename` | F2 on several items: one name with (1), (2), as in Windows Explorer |
//! | `hide_confirm` | Asking before a rename hides an item |
//! | `delete` | Move to Trash and permanent delete, with their confirmations |
//! | `recycle_bin` | Restore, Delete permanently and Empty in the Recycle Bin |
//! | `duplicate` | Duplicate |
//! | `journal` | Undo and Redo |
//! | `links` | Create links, from a drop |
//! | `clipboard` | Cut, Copy and the desktop's clipboard: files and pictures |
//! | `pasted_content` | A picture pasted as a new file (CLIP-015) |
//! | `transfer` | Pasting into a folder, with the name-conflict check |
//! | `unstorable_dialog` | The question about names and links the destination cannot store |
//! | `shortcuts` | The file commands' keys, which text fields keep |
//! | `actions` | The window actions of these commands |

mod actions;
mod availability;
mod batch_rename;
mod clipboard;
mod delete;
mod delete_label;
mod duplicate;
mod extension_confirm;
mod failure_dialog;
mod hide_confirm;
mod inline_rename;
mod jobs;
mod journal;
mod links;
mod move_by_copying_dialog;
mod name_balloon;
mod name_dialog;
mod names;
mod new_items;
mod numbered_rename;
mod pasted_content;
mod progress;
mod recycle_bin;
mod rename;
mod rename_conflict;
mod running;
mod shortcuts;
mod template_menu;
mod transfer;
mod unfinished;
mod unstorable_dialog;
mod worker_question;

use ox_core::ops::TemplateList;
use ox_core::transfer::Cancellation;

pub(super) use availability::FileCommand;
pub(super) use clipboard::{ClipboardContent, ClipboardKind};
pub(super) use jobs::MAX_JOBS;
pub(super) use shortcuts::file_key_bindings;
pub(super) use transfer::IncomingItems;

/// What the window's file operations remember between commands.
#[derive(Debug, Default)]
pub(crate) struct FileOperations {
    /// The cancellation of the operation that runs now; `None` while none
    /// runs (`state.operation` in app.js, OPS-024).
    running: Option<Cancellation>,
    /// Independent copy, move and delete jobs, with their own panels.
    jobs: Vec<jobs::Job>,
    /// What the desktop's clipboard offers Paste, as last read
    /// (`state.clipboard`, CLIP-009): a file list, a picture, or nothing
    /// at all (CLIP-015, CLIP-016). Only the kind is kept, so a large
    /// picture the clipboard holds is not held here as well.
    clipboard: ClipboardKind,
    /// How many times the clipboard changed while the window watched, so
    /// a read that an owner change overtook is dropped.
    clipboard_generation: u64,
    /// "Don't ask again" was checked when a rename hid an item (OPS-013).
    hiding_confirmed: bool,
    /// The item to rename in place next, once the rename Tab committed
    /// has finished (OPS-012).
    rename_next: Option<String>,
    /// The templates the New menu lists, as last read (OPS-003).
    templates: Option<TemplateList>,
}

impl FileOperations {
    /// True while an exclusive operation runs.
    fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// How many transfers run now, each with its own panel. A paste that
    /// checks or asks about name conflicts is one.
    pub(crate) fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// True while an exclusive operation runs: no new file operation may
    /// start.
    pub(crate) fn is_busy(&self) -> bool {
        self.is_running()
    }

    /// True while no operation runs, so a drag or a drop may start.
    pub(crate) fn is_idle(&self) -> bool {
        !self.is_busy() && self.jobs.is_empty()
    }
}
