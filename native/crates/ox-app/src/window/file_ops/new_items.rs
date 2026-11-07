// SPDX-License-Identifier: AGPL-3.0-only
//! New folder (Ctrl+Shift+N), and the New menu's files (OPS-001,
//! OPS-002, OPS-003, CMD-004).
//!
//! New folder makes the folder at once, as Windows Explorer does: "New
//! folder", or "New folder (2)" and so on when that is taken, each made
//! with an exclusive creation so nothing is overwritten (OPS-008). The new
//! folder is then selected and renamed in place, its whole name selected
//! (OPS-010): Enter keeps the typed name, Escape keeps "New folder". Undo
//! moves it to the Trash (OPS-029). This replaces the port of `newItem` in
//! `v2.0.0:desktop/ui/app.js`, which asked for the name in a dialog first.
//! The New menu's documents work the same way, as Windows 11's do: New ▸
//! Text Document makes "New Text Document.txt", or "New Text Document
//! (2).txt" and so on, published without overwriting, then renamed in
//! place with the name before ".txt" selected; Bitmap image, Rich Text
//! Document and Compressed (zipped) Folder make "New Bitmap image.bmp",
//! "New Rich Text Document.rtf" and "New Compressed (zipped) Folder.zip".
//! A template from the user's Templates folder is copied under its own
//! file name, "Letter.odt" or "Letter (2).odt", and renamed in place too.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;
use ox_core::ops::{
    create_numbered_document, create_numbered_folder, create_numbered_from_template, CreatedItem,
    NewDocument, OpsError,
};

use super::template_menu::templates_folder;
use super::FileCommand;
use crate::search::changed_folders;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// The name a new folder starts with, as Windows names it.
const NEW_FOLDER_NAME: &str = crate::i18n::message_id("New folder");

/// The name of the controllers that count the window's input.
const INPUT_WATCH: &str = "input-generation";

/// What a New menu item makes in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NewFile {
    /// One of Windows' documents, such as Text Document.
    Document(NewDocument),
    /// A copy of the user template at this path below Templates
    /// (OPS-003).
    Template(String),
}

impl NewFile {
    /// What the panel of a slow create says.
    fn creating_label(&self) -> &'static str {
        let label = match self {
            NewFile::Document(NewDocument::BitmapImage) => {
                crate::i18n::message_id("Creating a bitmap image…")
            }
            NewFile::Document(NewDocument::RichTextDocument) => {
                crate::i18n::message_id("Creating a rich text document…")
            }
            NewFile::Document(NewDocument::TextDocument) => {
                crate::i18n::message_id("Creating a text document…")
            }
            NewFile::Document(NewDocument::CompressedFolder) => {
                crate::i18n::message_id("Creating a compressed (zipped) folder…")
            }
            NewFile::Template(_) => crate::i18n::message_id("Creating a file…"),
        };
        ox_core::i18n::gettext_static(label)
    }
}

/// The name, before its extension, a new `document` starts with, as
/// Windows 11 names it.
fn document_base_name(document: NewDocument) -> &'static str {
    let name = match document {
        NewDocument::BitmapImage => crate::i18n::message_id("New Bitmap image"),
        NewDocument::RichTextDocument => crate::i18n::message_id("New Rich Text Document"),
        NewDocument::TextDocument => crate::i18n::message_id("New Text Document"),
        NewDocument::CompressedFolder => crate::i18n::message_id("New Compressed (zipped) Folder"),
    };
    ox_core::i18n::gettext_static(name)
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

    /// A New menu document or template: makes `file` (under the next
    /// free numbered name) at once, then selects it and renames it in
    /// place, as Windows Explorer does. Does nothing where New is
    /// disabled, as New folder; a file that cannot be made says why in the
    /// toast. While the file is made, on a slow share or phone, it is the
    /// window's running operation, with no panel; a cancelled one says
    /// nothing. When it ends after the user went to another tab or
    /// folder, the file stays and what is shown is left alone, as in
    /// Windows.
    pub(crate) async fn create_new_file(&self, file: NewFile) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(origin) = self.imp().session.borrow().active_id() else {
            return;
        };
        let Some(folder) = self.current_uri() else {
            return;
        };
        let generation = self.imp().input_generation.get();
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let Some(write) = self.begin_quick_write(file.creating_label(), &context.cancel) else {
            return;
        };
        #[cfg(test)]
        self.wait_at_text_document_gate().await;
        let created = match &file {
            NewFile::Document(document) => {
                let base_name = document_base_name(*document);
                create_numbered_document(&folder, base_name, *document, &context).await
            }
            NewFile::Template(path) => {
                // Without a Templates folder the template is refused as
                // unavailable.
                let templates = templates_folder().await.unwrap_or_default();
                create_numbered_from_template(&folder, path, &templates, &context).await
            }
        };
        self.end_quick_write(write, &context.cancel);
        match created {
            Err(error) if error.is_cancelled() => {}
            Ok(created) if !self.is_undisturbed_since(generation, origin, &folder) => {
                self.context().record_operation(created.undo_record());
                let changed = changed_folders([folder.as_str()], [created.uri.as_str()]);
                self.context().search_cache().folders_written(changed);
                self.mark_changed(&folder);
            }
            created => self.rename_created(created),
        }
    }

    /// Whether renaming the new item in place would disturb nothing the
    /// user did since the create started at input `generation` in tab
    /// `id` on `folder`: no key, mouse button or wheel was used in the
    /// window, it is still the active window, no dialog is in front, and
    /// that tab is still in front on the same folder, without a search.
    fn is_undisturbed_since(&self, generation: u64, id: TabId, folder: &str) -> bool {
        self.imp().input_generation.get() == generation
            && self.is_active()
            && !self.dialog_in_front()
            && !self.is_searching()
            && self.shows_in_front(id, folder)
    }

    /// Whether tab `id` is in front and still shows `folder`.
    fn shows_in_front(&self, id: TabId, folder: &str) -> bool {
        let session = self.imp().session.borrow();
        session.is_active(id) && session.tab(id).is_some_and(|tab| tab.uri() == folder)
    }

    /// Counts every key press, mouse button press and scroll in the
    /// window, before any widget handles it, for
    /// [`Self::is_undisturbed_since`]. Nothing is stopped.
    pub(super) fn watch_input(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, _, _| {
                window.note_input();
                glib::Propagation::Proceed
            }
        ));
        let press = gtk::GestureClick::builder().button(0).build();
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.note_input()
        ));
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        wheel.connect_scroll(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, _| {
                window.note_input();
                glib::Propagation::Proceed
            }
        ));
        for controller in [
            keys.upcast::<gtk::EventController>(),
            press.upcast(),
            wheel.upcast(),
        ] {
            controller.set_name(Some(INPUT_WATCH));
            controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            self.add_controller(controller);
        }
    }

    /// One more input for [`Self::watch_input`]. A rename in place that
    /// waits for its folder to be listed again is dropped: the user moved
    /// on before its editor showed.
    fn note_input(&self) {
        let generation = &self.imp().input_generation;
        generation.set(generation.get().wrapping_add(1));
        if let Ok(mut session) = self.imp().session.try_borrow_mut() {
            session.change_panes(|tab| tab.renames_selection = false);
        }
    }

    /// Tells every pane still showing `folder` that it changed, as a
    /// change notice would: one on screen is listed again at once,
    /// keeping its selection and search; one in the background is listed
    /// again too, or, on a share, when it is shown again, so a share that
    /// sends no change notices shows the new item too. Nothing is
    /// selected or renamed.
    pub(super) fn mark_changed(&self, folder: &str) {
        let showing: Vec<TabId> = {
            let session = self.imp().session.borrow();
            let panes = session.panes().filter(|tab| same_location(tab.uri(), folder));
            panes.map(|tab| tab.id).collect()
        };
        for id in showing {
            self.folder_changed(id);
        }
    }

    /// Holds the next New menu file before it is created until the
    /// gate's sender sends or is dropped, as a slow location would.
    /// Returns the sender that releases it.
    #[cfg(test)]
    pub(crate) fn hold_text_documents(&self) -> async_channel::Sender<()> {
        let (release, gate) = async_channel::bounded(1);
        self.imp().text_document_gate.replace(Some(gate));
        release
    }

    /// Whether a New menu file has reached the gate of
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

    /// Remembers the item New folder or a New menu file made for
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
}
