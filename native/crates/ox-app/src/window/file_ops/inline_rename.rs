// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming in place: the name becomes a text field in its row or tile
//! (OPS-010).
//!
//! As in Explorer and Dolphin, the field starts with a file's name before
//! its extension selected, or a folder's whole name. Enter or moving focus
//! away commits; Escape cancels. The name is checked with the Python Rename
//! dialog's rules and messages (`validateName`, then the backend's); a
//! refusal, such as a taken name, shows in the toast and editing goes on,
//! so the typed name is not lost. While the rename runs the field is
//! disabled, so a second commit cannot start it again.

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::entry::Entry;
use ox_core::ops::{rename_item, OperationContext};

use super::names::check_typed_name;
use super::rename::selected_name_length;
use crate::folder_view::cells::FileCell;
use crate::window::BrowserWindow;

/// The CSS class of the text field that edits a name in place.
const NAME_EDITOR_CLASS: &str = "rename-field";

/// The item being renamed in place.
#[derive(Debug, Clone)]
struct RenameTarget {
    /// Its URI.
    uri: String,
    /// Its name before the rename.
    name: String,
}

impl BrowserWindow {
    /// Starts editing `entry`'s name in `cell`.
    pub(super) fn rename_in_place(&self, cell: &FileCell, entry: &Entry) {
        let editor = gtk::Entry::builder()
            .text(&entry.name)
            .hexpand(true)
            .css_classes([NAME_EDITOR_CLASS])
            .build();
        editor.update_property(&[gtk::accessible::Property::Label("New name")]);
        let target = RenameTarget {
            uri: entry.uri.clone(),
            name: entry.name.clone(),
        };
        self.connect_name_editor(cell, &editor, &target);
        cell.show_name_editor(&editor);
        editor.grab_focus();
        let selected = i32::try_from(selected_name_length(entry)).unwrap_or(-1);
        editor.select_region(0, selected);
    }

    /// Enter and leaving the field commit; Escape cancels.
    fn connect_name_editor(&self, cell: &FileCell, editor: &gtk::Entry, target: &RenameTarget) {
        editor.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            #[strong]
            target,
            move |editor| window.commit_name(&cell, editor, &target)
        ));
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            #[weak]
            editor,
            #[strong]
            target,
            move |_| {
                // GTK is still moving focus off the field, which the
                // commit may remove: commit once it is done.
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    cell,
                    #[weak]
                    editor,
                    #[strong]
                    target,
                    move || window.commit_name(&cell, &editor, &target)
                ));
            }
        ));
        editor.add_controller(focus);
        editor.add_controller(self.escape_cancels_rename(cell));
    }

    /// Escape in the field ends the rename and leaves the name as it was.
    fn escape_cancels_rename(&self, cell: &FileCell) -> gtk::EventControllerKey {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key != gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                window.end_rename_in_place(&cell);
                glib::Propagation::Stop
            }
        ));
        keys
    }

    /// Renames the item to the name typed in `editor`: nothing for the
    /// same name, the toast for a refused one, else the rename.
    fn commit_name(&self, cell: &FileCell, editor: &gtk::Entry, target: &RenameTarget) {
        // A disabled field is committing already; a hidden one has ended.
        if !editor.is_sensitive() || editor.parent().is_none() {
            return;
        }
        let typed = editor.text();
        if typed == target.name {
            self.end_rename_in_place(cell);
            return;
        }
        let name = match check_typed_name(&typed) {
            Ok(name) => name.to_owned(),
            Err(invalid) => {
                self.show_message(&invalid.to_string());
                return;
            }
        };
        editor.set_sensitive(false);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            #[weak]
            editor,
            #[strong]
            target,
            async move {
                window.run_rename_in_place(&cell, &editor, &target, &name).await;
            }
        ));
    }

    /// Renames the item to `name`; on a refusal the field comes back for
    /// another try.
    async fn run_rename_in_place(
        &self,
        cell: &FileCell,
        editor: &gtk::Entry,
        target: &RenameTarget,
        name: &str,
    ) {
        let context = OperationContext::new(self.context().write_protection());
        match rename_item(&target.uri, name, &context).await {
            Ok(renamed) => {
                // Before the folder is listed again, which rebinds the cell.
                self.close_name_editor(cell);
                self.finish_rename(renamed);
            }
            Err(error) => {
                editor.set_sensitive(true);
                editor.grab_focus();
                self.show_message(&error.to_string());
            }
        }
    }

    /// Ends the rename from one of the field's own signals, once it has
    /// been handled: GTK may still update the field it came from.
    fn end_rename_in_place(&self, cell: &FileCell) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            move || window.close_name_editor(&cell)
        ));
    }

    /// Gives the view back its focus, then shows the name again. Focus
    /// leaves the field before it goes, so GTK tells the field.
    fn close_name_editor(&self, cell: &FileCell) {
        self.folder_pane().focus_view();
        cell.hide_name_editor();
    }
}
