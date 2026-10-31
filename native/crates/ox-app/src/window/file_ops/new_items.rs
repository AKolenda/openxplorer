// SPDX-License-Identifier: AGPL-3.0-only
//! New folder (Ctrl+Shift+N), and the New menu's files (OPS-001,
//! OPS-002, CMD-004).
//!
//! New folder makes the folder at once, as Windows Explorer does: "New
//! folder", or "New folder (2)" and so on when that is taken, each made
//! with an exclusive creation so nothing is overwritten (OPS-008). The new
//! folder is then selected and renamed in place, its whole name selected
//! (OPS-010): Enter keeps the typed name, Escape keeps "New folder". Undo
//! moves it to the Trash (OPS-029). This replaces the port of `newItem` in
//! `v2.0.0:desktop/ui/app.js`, which asked for the name in a dialog first.
//! New ▸ Text document works the same way with the Text document starter:
//! "New Text Document.txt", or "New Text Document (2).txt" and so on,
//! published without overwriting, then renamed in place with the name
//! before ".txt" selected. Every other file of the New menu opens the
//! template dialog ([`super::template_dialog`]) with its template chosen.

use gtk::subclass::prelude::*;
use ox_core::ops::{
    create_numbered_folder, create_numbered_starter, BuiltinTemplate, CreatedItem, OpsError, TemplateId,
};

use super::FileCommand;
use crate::window::BrowserWindow;

/// The name a new folder starts with, as Windows names it.
const NEW_FOLDER_NAME: &str = crate::i18n::message_id("New folder");

/// The name, before ".txt", a new text document starts with, as Windows
/// names it.
const NEW_TEXT_DOCUMENT_NAME: &str = crate::i18n::message_id("New Text Document");

/// The template a New menu item starts its dialog with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NewFileKind {
    /// "File…": an empty file with any name (`newTemplateDialog('empty')`).
    Empty,
    /// One of the built-in starters, such as "Markdown document".
    Starter(BuiltinTemplate),
    /// "From template…": the first template, to choose another
    /// (`newTemplateDialog(null)`).
    AnyTemplate,
    /// A user template the New menu lists (OPS-003).
    Template(TemplateId),
}

impl BrowserWindow {
    /// New ▸ Folder and Ctrl+Shift+N: makes "New folder" (or the next
    /// free numbered name) at once, then selects it and renames it in
    /// place, as Windows Explorer does. Does nothing where New is
    /// disabled: during a search, while an operation runs, or in a folder
    /// that is not writable. A folder that cannot be made says why in the
    /// toast.
    pub(crate) async fn create_folder(&self) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(folder) = self.current_uri() else {
            return;
        };
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let name = ox_core::i18n::gettext_static(NEW_FOLDER_NAME);
        self.rename_created(create_numbered_folder(&folder, name, &context).await);
    }

    /// New ▸ Text document: makes "New Text Document.txt" (or the next
    /// free numbered name) from the Text document starter at once, then
    /// selects it and renames it in place, as Windows Explorer does.
    /// Does nothing where New is disabled, as New folder; a file that
    /// cannot be made says why in the toast.
    pub(crate) async fn create_text_document(&self) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(folder) = self.current_uri() else {
            return;
        };
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let name = format!("{}.txt", ox_core::i18n::gettext_static(NEW_TEXT_DOCUMENT_NAME));
        let created = create_numbered_starter(&folder, &name, BuiltinTemplate::Text, &context).await;
        self.rename_created(created);
    }

    /// Remembers the item New folder or New ▸ Text document made for
    /// Undo, then lists the folder with it selected and renamed in place;
    /// a failure says why in the toast.
    fn rename_created(&self, created: Result<CreatedItem, OpsError>) {
        match created {
            Ok(created) => {
                self.context().record_operation(created.undo_record());
                // Listed again, the new item is selected, scrolled to and
                // renamed in place, as Tab moves a rename on (OPS-012).
                if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
                    tab.renames_selection = true;
                }
                self.reload_selecting(vec![created.uri]);
            }
            Err(error) => self.show_message(&error.to_string()),
        }
    }

    /// Remembers a created item for Undo, then lists the folder with the
    /// item selected.
    pub(super) fn finish_creation(&self, created: CreatedItem) {
        self.context().record_operation(created.undo_record());
        self.reload_selecting(vec![created.uri]);
    }
}
