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

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::ops::{
    create_numbered_folder, create_numbered_starter, BuiltinTemplate, CreatedItem, OpsError, TemplateId,
};

use super::FileCommand;
use crate::search::changed_folders;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// The name a new folder starts with, as Windows names it.
const NEW_FOLDER_NAME: &str = crate::i18n::message_id("New folder");

/// What was in front when New ▸ Text document started, which renaming
/// the new file in place must not disturb.
#[derive(Debug, PartialEq, Eq)]
struct FrontView {
    /// The tab in front.
    tab: TabId,
    /// The folder it showed.
    folder: String,
    /// The search box's text.
    search: String,
    /// The selected items.
    selected: Vec<String>,
}

/// What the panel of a slow New ▸ Text document says.
const CREATING_TEXT_DOCUMENT: &str = crate::i18n::message_id("Creating a text document…");

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
    /// cannot be made says why in the toast. While the file is made, on a
    /// slow share or phone, it is the window's running operation, with no
    /// panel; a cancelled one says nothing. When it ends after the user
    /// went to another tab or folder, the file stays and what is shown
    /// is left alone, as in Windows.
    pub(crate) async fn create_text_document(&self) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(front) = self.front_view() else {
            return;
        };
        let (origin, folder) = (front.tab, front.folder.clone());
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let label = ox_core::i18n::gettext_static(CREATING_TEXT_DOCUMENT);
        let Some(write) = self.begin_quick_write(label, &context.cancel) else {
            return;
        };
        #[cfg(test)]
        self.wait_at_text_document_gate().await;
        let name = format!("{}.txt", ox_core::i18n::gettext_static(NEW_TEXT_DOCUMENT_NAME));
        let created = create_numbered_starter(&folder, &name, BuiltinTemplate::Text, &context).await;
        self.end_quick_write(&write);
        match created {
            Err(error) if error.is_cancelled() => {}
            Ok(created) if !self.is_undisturbed_since(&front) => {
                self.context().record_operation(created.undo_record());
                let changed = changed_folders([folder.as_str()], [created.uri.as_str()]);
                self.context().search_cache().folders_written(changed);
                self.mark_changed(origin, &folder);
            }
            created => self.rename_created(created),
        }
    }

    /// What is in front now, for [`Self::is_undisturbed_since`].
    fn front_view(&self) -> Option<FrontView> {
        let tab = self.imp().session.borrow().active_id()?;
        Some(FrontView {
            tab,
            folder: self.current_uri()?,
            search: self.search_box().query(),
            selected: self.folder_pane().model().selected_uris(),
        })
    }

    /// Whether renaming the new item in place would disturb nothing the
    /// user did since `before`: the same tab is in front on the same
    /// folder, with the same search (none) and the same selection, the
    /// keyboard is not in another text field, such as the address bar or
    /// the search box, and no dialog is open over the window.
    fn is_undisturbed_since(&self, before: &FrontView) -> bool {
        let typing_elsewhere = GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::Editable>());
        self.front_view().as_ref() == Some(before) && !typing_elsewhere && !self.has_open_dialog()
    }

    /// Tells tab `id` that `folder` changed, when it still shows it, as a
    /// change notice would: a tab on screen is listed again at once,
    /// keeping its selection and search; one in the background when it
    /// is shown again, so a share that sends no change notices shows the
    /// new item too. Nothing is selected or renamed.
    fn mark_changed(&self, id: TabId, folder: &str) {
        let is_shown = {
            let mut session = self.imp().session.borrow_mut();
            let is_shown = session.is_active(id) || session.beside_active() == Some(id);
            let Some(tab) = session.tab_mut(id).filter(|tab| tab.uri() == folder) else {
                return;
            };
            if !is_shown {
                tab.changed_while_hidden = true;
            }
            is_shown
        };
        if is_shown {
            self.folder_changed(id);
        }
    }

    /// Holds New ▸ Text document before it creates the file until the
    /// gate's sender sends or is dropped, as a slow location would.
    /// Returns the sender that releases it.
    #[cfg(test)]
    pub(crate) fn hold_text_documents(&self) -> async_channel::Sender<()> {
        let (release, gate) = async_channel::bounded(1);
        self.imp().text_document_gate.replace(Some(gate));
        release
    }

    /// Whether New ▸ Text document has reached the gate of
    /// [`Self::hold_text_documents`].
    #[cfg(test)]
    pub(crate) fn reached_text_document_gate(&self) -> bool {
        self.imp().text_document_gate.borrow().is_none()
    }

    /// Waits at the gate of [`Self::hold_text_documents`], if one is set.
    #[cfg(test)]
    async fn wait_at_text_document_gate(&self) {
        let gate = self.imp().text_document_gate.take();
        if let Some(gate) = gate {
            // A dropped sender releases it too.
            let _ = gate.recv().await;
        }
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
