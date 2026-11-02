// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming several selected items at once (OPS-014), as Windows
//! Explorer does.
//!
//! F2 or Rename with several items selected edits one name in place: the
//! focused item's, or the first selected item's when focus is on none of
//! them ([`super::inline_rename`]). Enter gives every selected item that
//! name with a number in brackets, from (1) in the order shown; each file
//! keeps its own extension ([`ox_core::ops::BatchRename`]), so no
//! extension question is asked. Escape renames nothing. When the edited
//! item's cell is not on screen, the Rename dialog asks for the name
//! instead, as it does for one item. The renames run as the window's one
//! operation; the renamed items are selected afterwards without a
//! message, an item that could not be renamed is named in the operation's
//! result, and Undo renames the whole batch back (OPS-029).

use ox_core::entry::Entry;
use ox_core::location::parent_location;
use ox_core::ops::{rename_batch, summarize_batch_rename, BatchItem, BatchRename};

use super::name_dialog::{ask_for_name, NameRequest, NameSelection};
use super::running::FinishedOperation;
use crate::window::BrowserWindow;

/// The panel's label while the items are renamed.
const RENAMING: &str = crate::i18n::message_id("Renaming items…");

/// The batch that gives `entries`, in the order shown, the name typed for
/// `edited`.
pub(super) fn batch_for(entries: &[Entry], edited: &Entry, typed: &str) -> BatchRename {
    BatchRename {
        items: entries
            .iter()
            .map(|entry| BatchItem {
                uri: entry.uri.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
            })
            .collect(),
        name: typed.to_owned(),
        edited_name: edited.name.clone(),
    }
}

impl BrowserWindow {
    /// Asks for the name of `entries` in the Rename dialog, starting with
    /// `edited`'s name, then renames them all: the edited item's cell is
    /// not on screen.
    pub(super) async fn rename_several_with_dialog(&self, entries: &[Entry], edited: &Entry) {
        let selection = if edited.is_dir {
            NameSelection::Whole
        } else {
            NameSelection::Stem
        };
        let folder = parent_location(&edited.uri).unwrap_or_default();
        let request = NameRequest {
            title: "Rename",
            initial_name: &edited.name,
            selection,
            folder: &folder,
        };
        let batch = ask_for_name(self, request, |name, _cancel| {
            let batch = batch_for(entries, edited, &name);
            async move {
                batch
                    .base_name()
                    .map(|_| batch)
                    .map_err(|error| error.to_string())
            }
        })
        .await;
        if let Some(batch) = batch {
            self.run_batch_rename(&batch).await;
        }
    }

    /// Renames `batch` as the window's one operation and reports what
    /// failed; the renamed items are selected.
    pub(super) async fn run_batch_rename(&self, batch: &BatchRename) {
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RENAMING)) else {
            return;
        };
        let outcome = rename_batch(batch, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_batch_rename(&outcome.result),
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }
}
