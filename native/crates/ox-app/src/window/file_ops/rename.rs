// SPDX-License-Identifier: AGPL-3.0-only
//! Rename (F2): the one selected item gets a new name in its folder
//! (OPS-008, OPS-009).
//!
//! Ports `rename` in `desktop/ui/app.js`. It does nothing unless exactly
//! one item is selected, no operation runs, and the item can be changed:
//! not a share root, a virtual entry or a previous version. The rename
//! never overwrites; a taken name is refused inside the dialog. The
//! renamed item is selected afterwards (the Python app cleared the
//! selection), and Undo renames it back (OPS-029).

use ox_core::ops::{rename_item, OperationContext, RenamedItem};

use super::name_dialog::{ask_for_name, NameRequest, NameSelection};
use super::FileCommand;
use crate::window::BrowserWindow;

impl BrowserWindow {
    /// Asks for the selected item's new name and renames it.
    pub(crate) async fn rename_selection(&self) {
        if !self.allows(FileCommand::Rename) {
            return;
        }
        let items = self.folder_pane().model().selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        let entry = item.entry();
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

    /// Remembers the rename for Undo and selects the item under its new
    /// name.
    fn finish_rename(&self, renamed: RenamedItem) {
        if let Some(record) = renamed.undo_record() {
            self.context().record_operation(record);
        }
        self.reload_selecting(vec![renamed.uri]);
    }
}
