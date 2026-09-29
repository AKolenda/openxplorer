// SPDX-License-Identifier: AGPL-3.0-only
//! Rename (F2): the one selected item gets a new name in its folder
//! (OPS-008, OPS-009, OPS-010).
//!
//! Ports `rename` in `desktop/ui/app.js`. It does nothing unless an item
//! is selected, no operation runs, and the item can be changed: not a
//! share root, a virtual entry or a previous version. With several items
//! selected, the batch rename asks instead ([`super::batch_rename`]),
//! where the Python app did nothing. As in
//! Explorer and Dolphin, the name is edited in place in its row or tile
//! ([`super::inline_rename`]); when the item's cell is not on screen, the
//! Python app's Rename dialog asks instead. Either way the name is checked
//! with the Python messages and the rename never overwrites. The renamed
//! item is selected afterwards (the Python app cleared the selection), a
//! toast with Undo says it is renamed (OPS-032), and Undo renames it back
//! (OPS-029).

use ox_core::entry::Entry;
use ox_core::ops::{rename_item, OperationContext, RenamedItem};

use super::name_dialog::{ask_for_name, stem_length, NameRequest, NameSelection};
use super::FileCommand;
use crate::window::BrowserWindow;

/// How much of `entry`'s name a rename selects: a file's name before its
/// extension, a folder's whole name.
pub(super) fn selected_name_length(entry: &Entry) -> usize {
    if entry.is_dir {
        entry.name.chars().count()
    } else {
        stem_length(&entry.name)
    }
}

impl BrowserWindow {
    /// F2: renames the selected item in place, or with the dialog when its
    /// cell is not on screen; several selected items are renamed together
    /// (OPS-014).
    pub(crate) async fn rename_selection(&self) {
        if !self.allows(FileCommand::Rename) {
            return;
        }
        let model = self.folder_pane().model();
        let selected = model.selected_items();
        if selected.len() > 1 {
            let entries: Vec<Entry> = selected.iter().map(|item| item.entry().clone()).collect();
            self.rename_several(&entries).await;
            return;
        }
        let Some(position) = model.first_selected() else {
            return;
        };
        let Some(item) = model.item(position) else {
            return;
        };
        let view = self.folder_pane().view_widget();
        match self.folder_pane().owners().file_cell_at(position, &view) {
            Some(cell) => self.rename_in_place(&cell, item.entry()),
            None => self.rename_with_dialog(item.entry()).await,
        }
    }

    /// Asks for `entry`'s new name in the Rename dialog and renames it.
    async fn rename_with_dialog(&self, entry: &Entry) {
        let selection = if entry.is_dir {
            NameSelection::Whole
        } else {
            NameSelection::Stem
        };
        let request = NameRequest {
            title: "Rename",
            initial_name: &entry.name,
            selection,
        };
        let protection = self.context().write_protection();
        let renamed = ask_for_name(self, request, |name| {
            let uri = entry.uri.clone();
            let context = OperationContext::new(protection.clone());
            async move {
                rename_item(&uri, &name, &context)
                    .await
                    .map_err(|error| error.to_string())
            }
        })
        .await;
        if let Some(renamed) = renamed {
            self.finish_rename(renamed);
        }
    }

    /// Remembers the rename for Undo, selects the item under its new
    /// name, and says so in a toast with Undo (OPS-032).
    pub(super) fn finish_rename(&self, renamed: RenamedItem) {
        if let Some(record) = renamed.undo_record() {
            self.context().record_operation(record);
            self.show_message_with_undo(&format!("Renamed to “{}”.", renamed.name));
        }
        self.reload_selecting(vec![renamed.uri]);
    }
}
