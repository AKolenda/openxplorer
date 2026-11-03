// SPDX-License-Identifier: AGPL-3.0-only
//! Paste (Ctrl+V), and copies and moves into a folder with the
//! name-conflict check (CLIP-003, OPS-026, OPS-027, OPS-028, OPS-036).
//!
//! Ports `paste` and `transferWithConflicts` of `v2.0.0:desktop/ui/app.js`.
//! Paste reads the desktop's clipboard again first. The copy or move then
//! takes its action in the progress window, and before anything is
//! copied, the destination is checked for every incoming name, inside the
//! folders it merges into too ([`scan_name_conflicts`]); without a
//! conflict the copy starts at once with the Skip policy, so a name that
//! appears after the check is still never overwritten. With conflicts the
//! action asks "Replace or Skip Files" in place of its progress, as
//! Windows does, and "Compare info for both files" or "Let me decide for
//! each file" opens the File Conflict window
//! ([`crate::window::conflict_window`]); items without a conflict keep
//! the Skip policy whatever the answers. A copy of an item into its own
//! folder is kept as "name - Copy" without asking, as Windows Explorer
//! does: the only name it meets there is its own. An item moved (cut and
//! pasted) into its own folder stays where it is, without asking, as in
//! Windows Explorer; the other items move. The question holds the job's
//! place among the four that may run at once, and never the browsing
//! window. After a move, the moved items leave a cut clipboard.

use gtk::subclass::prelude::*;
use ox_core::clipboard::{ClipboardFiles, ClipboardMode};
use ox_core::location::{parent_location, same_location};
use ox_core::ops::{
    run_resolved_transfer, scan_name_conflicts, starting_label, ConflictScan, OpsError, Resolution,
    TransferOutcome,
};
use ox_core::transfer::{Cancellation, TransferMode};

use super::running::FinishedOperation;
use super::unfinished::mark_unfinished;
use crate::dialog;
use crate::search::changed_folders;
use crate::window::transfer_panel::{Action, Verb};
use crate::window::BrowserWindow;

/// The title of the dialog shown when the destination cannot be checked.
const CHECK_FAILED_TITLE: &str = crate::i18n::message_id("Could not check destination");

/// A copy or move of some items into one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncomingItems {
    /// Copy or move.
    pub(crate) mode: TransferMode,
    /// The items, in the order they were selected.
    pub(crate) uris: Vec<String>,
    /// The folder they go into.
    pub(crate) destination_folder: String,
}

impl IncomingItems {
    /// The items that a move would really move: an item moved into the
    /// folder it is already in stays where it is, without asking, as in
    /// Windows Explorer. `None` when nothing is left to move.
    fn without_items_already_there(mut self) -> Option<Self> {
        if self.mode == TransferMode::Move {
            let destination = self.destination_folder.clone();
            self.uris.retain(|uri| {
                !parent_location(uri).is_some_and(|parent| same_location(&parent, &destination))
            });
        }
        (!self.uris.is_empty()).then_some(self)
    }
}

impl BrowserWindow {
    /// Paste: reads the clipboard, then copies or moves its items into
    /// `into`, a selected folder (Dolphin's "Paste into folder", CMD-019),
    /// or without one into the folder shown (`paste`).
    pub(crate) async fn paste(&self, into: Option<String>) {
        let clipboard = self.refresh_file_clipboard().await;
        if into.is_none() && self.is_searching() {
            self.show_message(ox_core::i18n::gettext_static(
                "Open the destination folder before pasting.",
            ));
            return;
        }
        // Ctrl+V pastes without the Paste button, which a folder that
        // could not be read disables: it takes nothing either.
        if into.is_none() && self.shown_folder_is_unreadable() {
            self.show_message(ox_core::i18n::gettext_static("This location is unavailable"));
            return;
        }
        let Some(destination_folder) = into.or_else(|| self.current_uri()) else {
            return;
        };
        self.paste_files(clipboard, destination_folder).await;
    }

    /// Paste into the folder at `uri`, from the folder tree's menu
    /// (SIDE-028), as Dolphin's "Paste" on a folder pastes into it.
    pub(crate) async fn paste_into(&self, uri: &str) {
        let clipboard = self.refresh_file_clipboard().await;
        self.paste_files(clipboard, uri.to_owned()).await;
    }

    /// Copies or moves the clipboard's `files` into `destination_folder`
    /// where it is writable.
    async fn paste_files(&self, clipboard: Option<ClipboardFiles>, destination_folder: String) {
        let Some(files) = clipboard else {
            return;
        };
        if !self
            .imp()
            .locations
            .borrow()
            .is_writable_location(&destination_folder)
        {
            self.show_message(ox_core::i18n::gettext_static("This folder is read-only."));
            return;
        }
        let mode = match files.mode() {
            ClipboardMode::Copy => TransferMode::Copy,
            ClipboardMode::Cut => TransferMode::Move,
        };
        let incoming = IncomingItems {
            mode,
            uris: files.uris().to_vec(),
            destination_folder,
        };
        let Some(outcome) = self.transfer_with_conflicts(incoming).await else {
            return;
        };
        if mode == TransferMode::Move && !outcome.result.done.is_empty() {
            self.consume_cut(&files, &outcome.result.done).await;
        }
    }

    /// Checks `incoming` for name conflicts, asks about them, then runs the
    /// copy or move and reports it (`transferWithConflicts`). Returns the
    /// outcome of a run that finished; `None` when another operation runs,
    /// the check failed or the user cancelled.
    pub(crate) async fn transfer_with_conflicts(&self, incoming: IncomingItems) -> Option<TransferOutcome> {
        self.transfer_checked(incoming, true).await
    }

    /// [`Self::transfer_with_conflicts`] for a move that is one step of a
    /// larger operation, which Undo cannot reverse on its own: the items
    /// of an extraction into an existing folder come from a private folder
    /// that is removed afterwards, so the journal does not record it.
    pub(crate) async fn transfer_without_undo(&self, incoming: IncomingItems) -> Option<TransferOutcome> {
        self.transfer_checked(incoming, false).await
    }

    /// Checks, asks, runs and reports `incoming`; the journal records it
    /// when `undoable`.
    ///
    /// The job claims its action in the progress window first, as Windows
    /// shows "Copying 1 item from Downloads to Desktop" before it asks, so
    /// the four-job limit and the overlap check count it while it checks
    /// and asks, and the browsing windows stay free meanwhile.
    async fn transfer_checked(&self, incoming: IncomingItems, undoable: bool) -> Option<TransferOutcome> {
        let incoming = incoming.without_items_already_there()?;
        let origin = self.current_uri();
        let mode = incoming.mode;
        let destination = incoming.destination_folder.clone();
        let context = self.begin_transfer(starting_label(mode), &incoming.uris, Some(&destination))?;
        let verb = Verb::of_mode(mode);
        self.describe_operation(
            &context.cancel,
            Action::of(verb, &incoming.uris, Some(&destination)),
        );
        let Some((scan, answers)) = self.check_and_ask(&incoming, &context.cancel).await else {
            self.end_transfer(&context.cancel);
            return None;
        };
        let progress = self.progress_reporter(&context.cancel);
        let mark = mark_unfinished(Some(&destination));
        let outcome = run_resolved_transfer(&scan, &answers, &context, progress).await;
        drop(mark);
        self.end_transfer(&context.cancel);
        let written = scan
            .items
            .iter()
            .map(|item| item.destination_folder.as_str())
            .chain([destination.as_str()]);
        let changed = changed_folders(written, incoming.uris.iter().map(String::as_str));
        self.context().search_cache().folders_written(changed);
        let finished = outcome.clone().map(|outcome| {
            let finished = FinishedOperation::of_transfer(mode, outcome);
            if undoable {
                finished
            } else {
                FinishedOperation {
                    undo: None,
                    ..finished
                }
            }
        });
        self.conclude_operation_in(finished, origin.as_deref()).await;
        outcome.ok()
    }

    /// Finds the name conflicts of `incoming`, inside merged folders too,
    /// and asks about them in the action of the job `cancel` stops: the
    /// plan and an answer per conflict, or `None` when nothing may run. A
    /// failed check opens "Could not check destination".
    async fn check_and_ask(
        &self,
        incoming: &IncomingItems,
        cancel: &Cancellation,
    ) -> Option<(ConflictScan, Vec<Resolution>)> {
        let checked = scan_name_conflicts(
            incoming.mode,
            &incoming.uris,
            &incoming.destination_folder,
            cancel,
        )
        .await;
        let scan = match checked {
            Ok(scan) => scan,
            Err(OpsError::Cancelled) => return None,
            Err(error) => {
                dialog::show_message(
                    self,
                    ox_core::i18n::gettext_static(CHECK_FAILED_TITLE),
                    &error.to_string(),
                )
                .await;
                return None;
            }
        };
        if !scan.has_conflicts() {
            return Some((scan, Vec::new()));
        }
        let answers = self
            .panel_for_operation(cancel)
            .ask_about_conflicts(&scan)
            .await?;
        Some((scan, answers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A move leaves out the items already in the destination folder; a
    /// copy keeps them, to make "name - Copy".
    ///
    /// parity: OPS-026
    #[test]
    fn a_move_leaves_out_the_items_already_in_the_folder() {
        let items = |mode| IncomingItems {
            mode,
            uris: vec![
                "file:///tmp/target/a.txt".to_owned(),
                "file:///tmp/other/b.txt".to_owned(),
            ],
            destination_folder: "file:///tmp/target/".to_owned(),
        };
        let moved = items(TransferMode::Move).without_items_already_there();
        assert_eq!(
            moved.map(|moved| moved.uris),
            Some(vec!["file:///tmp/other/b.txt".to_owned()])
        );
        assert_eq!(
            items(TransferMode::Copy).without_items_already_there(),
            Some(items(TransferMode::Copy))
        );
        let only_there = IncomingItems {
            uris: vec!["file:///tmp/target/a.txt".to_owned()],
            ..items(TransferMode::Move)
        };
        assert_eq!(only_there.without_items_already_there(), None);
    }
}
