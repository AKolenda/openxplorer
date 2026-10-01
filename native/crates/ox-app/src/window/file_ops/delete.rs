// SPDX-License-Identifier: AGPL-3.0-only
//! Delete and Shift+Delete, with their confirmations (OPS-015, OPS-016,
//! OPS-017, OPS-018).
//!
//! Ports `trash` in `desktop/ui/app.js`. Delete decides each item by its
//! own folder: where the folder has a Trash the item goes there, elsewhere
//! it is deleted permanently, and the confirmation says which (a folder
//! GIO cannot be asked about counts as having a Trash, so a failed check
//! never becomes a permanent delete). The Trash items run first, then the
//! others, each as an operation of its own. Shift+Delete, which the Python
//! app did not have, deletes the selection permanently after its own
//! confirmation. Both confirm with a red button and Cancel has focus, so
//! Enter never deletes by accident. In the Recycle Bin, both delete the
//! selected items for good ([`super::recycle_bin`]). Items dropped on the
//! Recycle Bin go the way of Delete (OPS-045).

use gtk::gio;
use gtk::gio::prelude::*;
use ox_core::ops::{
    permanent_delete_confirmation, plan_delete, DeleteConfirmation, DeleteItem, TransferRequest,
};
use ox_core::transfer::{Cancellation, ConflictPolicy, TransferMode};

use super::FileCommand;
use crate::window::dialog::{ButtonStyle, Dialog};
use crate::window::BrowserWindow;

/// A Trash or delete request for `uris`.
fn removal(mode: TransferMode, uris: Vec<String>) -> TransferRequest {
    TransferRequest {
        mode,
        uris,
        destination_folder: None,
        policy: ConflictPolicy::Skip,
    }
}

impl BrowserWindow {
    /// The selected items, as the confirmations name them.
    pub(super) fn items_to_delete(&self) -> Vec<DeleteItem> {
        let items = self.folder_pane().model().selected_items();
        items
            .iter()
            .map(|item| DeleteItem {
                uri: item.entry().uri.clone(),
                name: item.entry().name.clone(),
            })
            .collect()
    }

    /// Delete: asks, then moves each selected item to its folder's Trash,
    /// or deletes it where the folder has none.
    pub(crate) async fn delete_selection(&self) {
        if !self.allows(FileCommand::Delete) {
            return;
        }
        if self.shows_recycle_bin() {
            self.delete_from_recycle_bin().await;
            return;
        }
        self.trash_items(&self.items_to_delete()).await;
    }

    /// Items dropped on the Recycle Bin: moved to the Trash with Delete's
    /// confirmation (OPS-045).
    pub(crate) async fn trash_dropped(&self, uris: Vec<String>) {
        let items: Vec<DeleteItem> = uris
            .into_iter()
            .map(|uri| {
                let name = gio::File::for_uri(&uri)
                    .basename()
                    .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
                DeleteItem { uri, name }
            })
            .collect();
        self.trash_items(&items).await;
    }

    /// Asks, then moves each of `items` to its folder's Trash, or deletes
    /// it where the folder has none.
    async fn trash_items(&self, items: &[DeleteItem]) {
        // Only a cancellation fails the plan, and nothing cancels it here.
        let Ok(plan) = plan_delete(items, &Cancellation::new()).await else {
            return;
        };
        let preferences = self.context().settings_data().preferences;
        let asks = (!plan.to_trash.is_empty() && preferences.confirm_trash)
            || (!plan.to_delete.is_empty() && preferences.confirm_delete);
        if asks && !self.confirm_deletion(&plan.confirmation()).await {
            return;
        }
        if !plan.to_trash.is_empty() {
            self.run_and_conclude(&removal(TransferMode::Trash, plan.to_trash))
                .await;
        }
        if !plan.to_delete.is_empty() {
            self.run_and_conclude(&removal(TransferMode::Delete, plan.to_delete))
                .await;
        }
    }

    /// Shift+Delete: asks, then deletes the selection permanently, even
    /// where a Trash exists (OPS-016).
    pub(crate) async fn delete_selection_permanently(&self) {
        if !self.allows(FileCommand::DeletePermanently) {
            return;
        }
        if self.shows_recycle_bin() {
            self.delete_from_recycle_bin().await;
            return;
        }
        let items = self.items_to_delete();
        if !self.confirms_permanent_delete(&items).await {
            return;
        }
        let uris = items.into_iter().map(|item| item.uri).collect();
        self.run_and_conclude(&removal(TransferMode::Delete, uris)).await;
    }

    /// Asks before `items` are deleted permanently, unless the settings
    /// say not to ask (SET-010); true when they may be deleted.
    pub(super) async fn confirms_permanent_delete(&self, items: &[DeleteItem]) -> bool {
        !self.context().settings_data().preferences.confirm_delete
            || self.confirm_deletion(&permanent_delete_confirmation(items)).await
    }

    /// Asks `confirmation`'s question with Cancel and its red button;
    /// true when the user confirmed.
    pub(super) async fn confirm_deletion(&self, confirmation: &DeleteConfirmation) -> bool {
        let dialog = Dialog::new(self, confirmation.title, &confirmation.body);
        dialog.add_cancel_button();
        dialog.add_button(confirmation.confirm_label, ButtonStyle::Danger);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer.is_some()
    }

    /// True while the tab shows the Recycle Bin itself.
    pub(super) fn shows_recycle_bin(&self) -> bool {
        self.command_facts().folder.is_recycle_bin
    }
}
