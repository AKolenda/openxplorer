// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files and folders out of the folder views (DND-001, DND-002,
//! DND-003, DND-005, DND-006, DND-008).
//!
//! Ports `fileDragEntry` and `makeFileDraggable` of `desktop/ui/app.js`
//! and `prepare_files` of `desktop/native_file_drag.py` on GTK's own drag
//! source. Dragging a selected item carries the whole selection;
//! dragging another item selects only it first. Only real local and SMB
//! files, folders and links can leave: a share, a server, a special file
//! or anything else refuses the whole drag with the Python app's message.
//! The drag offers a file list, which GTK also publishes as
//! `text/uri-list`, with exact GIO URIs, so unusual names survive.
//!
//! Safety rule (DND-008): a drag out offers Copy only, so a receiver can
//! never ask the source to delete what it received; nothing is mounted or
//! downloaded during the gesture. No drag starts from blank space or
//! while a file operation runs or is being planned.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{Entry, EntryKind};
use ox_core::location::is_smb_server;

use crate::folder_view::item::FileItem;

use super::BrowserWindow;

/// The most items one drag carries (`MAX_ITEMS`).
const MAX_DRAGGED_ITEMS: usize = 200;

/// Why a selection cannot be dragged out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Select up to 200 files or folders. Extract ZIP contents before dragging them.")]
pub(super) struct NotDraggable;

/// Whether `entry` may leave the app by a drag (`fileDragEntry`): a real
/// local or SMB file, folder or link, not a share, server or shortcut.
fn is_draggable(entry: &Entry) -> bool {
    let is_file_location = entry.uri.starts_with("file:") || entry.uri.starts_with("smb:");
    let is_plain_item = matches!(
        entry.kind,
        EntryKind::File | EntryKind::Directory | EntryKind::Symlink
    );
    is_file_location && is_plain_item && !entry.is_virtual && !is_smb_server(&entry.uri)
}

/// The URIs a drag of `entries` carries: every one, in order, without
/// duplicates.
///
/// # Errors
///
/// [`NotDraggable`] for no entries, more than [`MAX_DRAGGED_ITEMS`], or
/// any entry that may not leave the app: the whole drag is refused.
pub(super) fn dragged_uris(entries: &[&Entry]) -> Result<Vec<String>, NotDraggable> {
    let within_limit = (1..=MAX_DRAGGED_ITEMS).contains(&entries.len());
    if !within_limit || !entries.iter().all(|entry| is_draggable(entry)) {
        return Err(NotDraggable);
    }
    let mut uris: Vec<String> = Vec::with_capacity(entries.len());
    for entry in entries {
        if !uris.contains(&entry.uri) {
            uris.push(entry.uri.clone());
        }
    }
    Ok(uris)
}

/// What a drag of `uris` offers: a file list, which GTK also serves as
/// `text/uri-list`.
fn file_list_content(uris: &[String]) -> gdk::ContentProvider {
    let files: Vec<gio::File> = uris.iter().map(|uri| gio::File::for_uri(uri)).collect();
    let list = gdk::FileList::from_array(&files);
    gdk::ContentProvider::for_value(&list.to_value())
}

impl BrowserWindow {
    /// Lets items of `view` be dragged out to other windows and apps.
    pub(super) fn attach_file_drag(&self, view: &gtk::Widget) {
        let source = gtk::DragSource::new();
        // DND-008: Copy only, so a receiver never deletes the source.
        source.set_actions(gdk::DragAction::COPY);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            #[upgrade_or]
            None,
            move |_, x, y| {
                let position = window.folder_pane().owners().position_at(&view, x, y)?;
                window.drag_content_for(position)
            }
        ));
        view.add_controller(source);
    }

    /// What dragging the item at `position` offers, selecting only it
    /// first when it is not selected; `None` while an operation runs or
    /// when the selection may not leave the app, which the toast explains.
    pub(super) fn drag_content_for(&self, position: u32) -> Option<gdk::ContentProvider> {
        if !self.imp().file_operations.borrow().is_idle() {
            return None;
        }
        let model = self.folder_pane().model();
        if !model.selection().is_selected(position) {
            model.select_only(position);
        }
        let items = model.selected_items();
        let entries: Vec<&Entry> = items.iter().map(FileItem::entry).collect();
        match dragged_uris(&entries) {
            Ok(uris) => Some(file_list_content(&uris)),
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// parity: DND-002, DND-003, DND-005
    #[test]
    fn a_drag_carries_every_selected_item_once_in_order() {
        let folder = folder_entry("Projects (2024) & more");
        let file = file_entry("Budget +20%.xlsx");

        let uris = dragged_uris(&[&folder, &file, &folder]);

        assert_eq!(uris, Ok(vec![folder.uri.clone(), file.uri.clone()]));
    }

    /// parity: DND-003
    #[test]
    fn one_item_that_may_not_leave_refuses_the_whole_drag() {
        let file = file_entry("notes.txt");
        let mut share = folder_entry("share");
        share.is_virtual = true;
        let mut socket = file_entry("socket");
        socket.kind = EntryKind::Special;
        let many: Vec<Entry> = (0..=MAX_DRAGGED_ITEMS)
            .map(|number| file_entry(&format!("file {number}")))
            .collect();
        let many_refs: Vec<&Entry> = many.iter().collect();

        assert_eq!(dragged_uris(&[&file, &share]), Err(NotDraggable));
        assert_eq!(dragged_uris(&[&socket]), Err(NotDraggable));
        assert_eq!(dragged_uris(&[]), Err(NotDraggable));
        assert_eq!(dragged_uris(&many_refs), Err(NotDraggable));
        assert_eq!(
            NotDraggable.to_string(),
            "Select up to 200 files or folders. Extract ZIP contents before dragging them."
        );
    }
}
