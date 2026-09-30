// SPDX-License-Identifier: AGPL-3.0-only
//! New folder (Ctrl+Shift+N), and the New menu's files (OPS-001,
//! OPS-002, CMD-004).
//!
//! Ports `newItem` and `openNewMenu` of `desktop/ui/app.js`. New folder
//! asks for the name first, "New folder" selected, and creates the folder
//! with an exclusive creation, so an existing name is refused inside the
//! dialog and never overwritten (OPS-008). The new folder is selected
//! afterwards, as Windows and Dolphin do (the Python app left the
//! selection as it was), and Undo moves it to the Trash (OPS-029).
//! Every file of the New menu opens the template dialog
//! ([`super::template_dialog`]) with its template chosen.

use ox_core::location::ItemKind;
use ox_core::ops::{create_item, BuiltinTemplate, CreatedItem, OperationContext};

use super::name_dialog::{ask_for_name, NameRequest, NameSelection};
use super::FileCommand;
use crate::window::BrowserWindow;

/// The name a new folder starts with.
const NEW_FOLDER_NAME: &str = "New folder";

/// The template a New menu item starts its dialog with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewFileKind {
    /// "File…": an empty file with any name (`newTemplateDialog('empty')`).
    Empty,
    /// One of the built-in starters, such as "Markdown document".
    Starter(BuiltinTemplate),
    /// "From template…": the first template, to choose another
    /// (`newTemplateDialog(null)`).
    AnyTemplate,
}

impl BrowserWindow {
    /// New ▸ Folder and Ctrl+Shift+N: asks for the name, creates the
    /// folder, then selects it. Does nothing where New is disabled: during
    /// a search, while an operation runs, or in a folder that is not
    /// writable.
    pub(crate) async fn create_folder(&self) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(folder) = self.current_uri() else {
            return;
        };
        let protection = self.context().write_protection();
        let request = NameRequest {
            title: NEW_FOLDER_NAME,
            initial_name: NEW_FOLDER_NAME,
            selection: NameSelection::Whole,
            folder: &folder,
        };
        let created = ask_for_name(self, request, |name| {
            let folder = folder.clone();
            let context = OperationContext::new(protection.clone());
            async move {
                create_item(&folder, &name, ItemKind::Folder, &context)
                    .await
                    .map_err(|error| error.to_string())
            }
        })
        .await;
        if let Some(created) = created {
            self.finish_creation(created);
        }
    }

    /// Remembers a created item for Undo, then lists the folder with the
    /// item selected.
    pub(super) fn finish_creation(&self, created: CreatedItem) {
        self.context().record_operation(created.undo_record());
        self.reload_selecting(vec![created.uri]);
    }
}
