// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming several selected items at once with F2 (OPS-050), as Windows
//! Explorer does. The "Rename items" dialog with its `#` template, under
//! the classic menu's "Rename items…", is [`super::batch_rename`]
//! (OPS-014).
//!
//! F2 or Rename with several items selected edits one name in place: the
//! focused item's, or the first selected item's when focus is on none of
//! them ([`super::inline_rename`]). Enter gives every selected item that
//! name with a number in brackets, from (1) in the order shown; each file
//! keeps its own extension ([`ox_core::ops::NumberedRename`]); only a
//! file without one can gain one from a dotted name ("README" becomes
//! "Plan.v1 (2)"), and then the extension question asks first, as for one
//! item (OPS-049). Escape renames nothing. When the edited
//! item's cell is not on screen, the Rename dialog asks for the name
//! instead, as it does for one item. The renames run as the window's one
//! operation; the renamed items are selected afterwards without a
//! message, an item that could not be renamed is named in the operation's
//! result, and Undo renames the whole batch back (OPS-029).

use gtk::glib::prelude::*;
use ox_core::entry::Entry;
use ox_core::location::parent_location;
use ox_core::ops::{rename_numbered_items, BatchItem, NumberedRename};

use super::extension_confirm::changes_extension;
use super::name_dialog::{ask_for_name, NameRequest};
use super::rename::{has_no_extension, name_selection, NOT_RENAMED};
use super::running::FinishedOperation;
use crate::window::BrowserWindow;

/// The panel's label while the items are renamed.
const RENAMING: &str = crate::i18n::message_id("Renaming items…");

/// The batch that gives `entries`, in the order shown, the name typed for
/// `edited`.
pub(super) fn numbered_batch_for(entries: &[Entry], edited: &Entry, typed: &str) -> NumberedRename {
    NumberedRename {
        items: entries
            .iter()
            .map(|entry| BatchItem {
                uri: entry.uri.clone(),
                name: entry.name.clone(),
                // A shortcut's name, like a folder's, has no extension.
                is_dir: has_no_extension(entry),
            })
            .collect(),
        name: typed.to_owned(),
        edited_name: edited.name.clone(),
        edited_is_dir: has_no_extension(edited),
    }
}

impl BrowserWindow {
    /// Asks for the name of `entries` in the Rename dialog, starting with
    /// `edited`'s name, then renames them all: the edited item's cell is
    /// not on screen.
    pub(super) async fn rename_numbered_with_dialog(&self, entries: &[Entry], edited: &Entry) {
        let selection = name_selection(edited);
        let folder = parent_location(&edited.uri).unwrap_or_default();
        let request = NameRequest {
            title: "Rename",
            initial_name: &edited.name,
            selection,
            folder: &folder,
        };
        let batch = ask_for_name(self, request, |name, _cancel| {
            // The unchanged name renames nothing, as Enter on it in place.
            let batch = (name != edited.name).then(|| numbered_batch_for(entries, edited, &name));
            let edited_name = edited.name.clone();
            let window = self.downgrade();
            async move {
                let Some(batch) = batch else {
                    return Ok(None);
                };
                let base = batch.base_name().map_err(|error| error.to_string())?;
                let not_renamed = || ox_core::i18n::gettext_static(NOT_RENAMED).to_owned();
                let window = window.upgrade().ok_or_else(not_renamed)?;
                // A name that changes an item's extension or hides the
                // items asks first, as in place and in the Rename dialog
                // of one item (OPS-049, OPS-013); Cancel keeps the dialog
                // with the typed name.
                if !window.confirm_numbered_extension(&batch, &base).await
                    || !window.confirm_hiding_rename(&edited_name, &base).await
                {
                    return Err(not_renamed());
                }
                Ok(Some(batch))
            }
        })
        .await;
        if let Some(Some(batch)) = batch {
            self.run_numbered_rename(&batch).await;
        }
    }

    /// True when the numbered rename to `base` may go ahead: no file's
    /// extension changes, or the user confirmed it (OPS-049).
    pub(super) async fn confirm_numbered_extension(&self, batch: &NumberedRename, base: &str) -> bool {
        let changed = batch
            .numbered_names(base)
            .find(|(item, name)| changes_extension(&item.name, name, item.is_dir));
        match changed {
            Some((item, name)) => {
                self.confirm_extension_change(&item.name, &name, item.is_dir)
                    .await
            }
            None => true,
        }
    }

    /// Renames `batch` as the window's one operation and reports what
    /// failed; the renamed items are selected.
    pub(super) async fn run_numbered_rename(&self, batch: &NumberedRename) {
        let Some(context) = self.begin_operation(ox_core::i18n::gettext_static(RENAMING)) else {
            return;
        };
        let outcome = rename_numbered_items(batch, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }
}
