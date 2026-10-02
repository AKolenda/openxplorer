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
//! disabled, so a second commit cannot start it again. A name that would
//! hide the item asks first ([`super::hide_confirm`]). As in Dolphin, Tab
//! and Shift+Tab, and Down and Up in the details view, commit and go on
//! to rename the next or previous item (OPS-012).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::entry::Entry;
use ox_core::ops::{rename_item, OperationContext};

use super::names::check_typed_name;
use super::rename::selected_name_length;
use crate::folder_view::cells::FileCell;
use crate::window::folder_pane::FolderView;
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

/// How many places Tab, Shift+Tab, Down or Up move the rename on from
/// the item being renamed; Down and Up only `in_details`, where they
/// move between rows. `None` for any other key, and with Ctrl or Alt,
/// which keep Ctrl+Tab and Ctrl+Shift+Tab for switching tabs.
fn rename_step(key: gdk::Key, modifiers: gdk::ModifierType, in_details: bool) -> Option<i32> {
    if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
        return None;
    }
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    match key {
        gdk::Key::Tab | gdk::Key::KP_Tab if !shift => Some(1),
        gdk::Key::ISO_Left_Tab | gdk::Key::Tab | gdk::Key::KP_Tab => Some(-1),
        gdk::Key::Down if in_details => Some(1),
        gdk::Key::Up if in_details => Some(-1),
        _ => None,
    }
}

impl BrowserWindow {
    /// Starts editing `entry`'s name in `cell`.
    pub(super) fn rename_in_place(&self, cell: &FileCell, entry: &Entry) {
        let editor = gtk::Entry::builder()
            .text(&entry.name)
            .hexpand(true)
            .css_classes([NAME_EDITOR_CLASS])
            .build();
        editor.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "New name",
        ))]);
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
        editor.add_controller(self.rename_keys(cell, editor, target));
    }

    /// Escape in the field ends the rename and leaves the name as it was;
    /// Tab and Shift+Tab, and Down and Up in the details view, commit and
    /// go on to rename the next or previous item (OPS-012).
    fn rename_keys(
        &self,
        cell: &FileCell,
        editor: &gtk::Entry,
        target: &RenameTarget,
    ) -> gtk::EventControllerKey {
        let keys = gtk::EventControllerKey::new();
        // Before the field's own text keys, which take Up and Down.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            cell,
            #[weak]
            editor,
            #[strong(rename_to = uri)]
            target.uri,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if key == gdk::Key::Escape {
                    window.end_rename_in_place(&cell);
                    return glib::Propagation::Stop;
                }
                let in_details = window.folder_pane().view() == FolderView::Details;
                let Some(step) = rename_step(key, modifiers, in_details) else {
                    return glib::Propagation::Proceed;
                };
                let next = window.neighbour_uri(&uri, step);
                window.imp().file_operations.borrow_mut().rename_next = next;
                editor.emit_activate();
                glib::Propagation::Stop
            }
        ));
        keys
    }

    /// The URI of the item `step` places after the one at `uri` in the
    /// order shown, or `None` at either end.
    fn neighbour_uri(&self, uri: &str, step: i32) -> Option<String> {
        let model = self.folder_pane().model();
        let position = (0..model.n_items())
            .find(|&position| model.item(position).is_some_and(|item| item.entry().uri == uri))?;
        let next = position.checked_add_signed(step)?;
        model.item(next).map(|item| item.entry().uri.clone())
    }

    /// Takes the item to rename next, which Tab chose.
    fn take_rename_next(&self) -> Option<String> {
        self.imp().file_operations.borrow_mut().rename_next.take()
    }

    /// Starts renaming the selected item in place once the view has laid
    /// out its rows: the item Tab moved on to.
    pub(in crate::window) fn continue_renaming(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                glib::spawn_future_local(async move { window.rename_selection().await });
            }
        ));
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
            if let Some(next) = self.take_rename_next() {
                let model = self.folder_pane().model();
                model.select_uris(&[next]);
                // As after a changed name, the item comes into view first,
                // so it is renamed in place too.
                if let Some(position) = model.first_selected() {
                    self.folder_pane().reveal(position);
                }
                self.continue_renaming();
            }
            return;
        }
        let name = match check_typed_name(&typed) {
            Ok(name) => name.to_owned(),
            Err(invalid) => {
                self.take_rename_next();
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
        // An update may have finished installing while the field was open.
        if self.refuses_writes_during_update() {
            self.close_name_editor(cell);
            return;
        }
        if !self.confirm_hiding_rename(&target.name, name).await {
            self.take_rename_next();
            editor.set_sensitive(true);
            editor.grab_focus();
            return;
        }
        let context = OperationContext::new(self.context().write_protection());
        match rename_item(&target.uri, name, &context).await {
            Ok(renamed) => {
                // Before the folder is listed again, which rebinds the cell.
                self.close_name_editor(cell);
                match self.take_rename_next() {
                    Some(next) => self.finish_rename_and_continue(&renamed, next),
                    None => self.finish_rename(renamed),
                }
            }
            Err(error) => {
                self.take_rename_next();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-012
    #[test]
    fn tab_and_the_arrows_in_details_move_the_rename_on() {
        let none = gdk::ModifierType::empty();
        let shift = gdk::ModifierType::SHIFT_MASK;
        assert_eq!(rename_step(gdk::Key::Tab, none, false), Some(1));
        assert_eq!(rename_step(gdk::Key::ISO_Left_Tab, shift, false), Some(-1));
        assert_eq!(rename_step(gdk::Key::Down, none, true), Some(1));
        assert_eq!(rename_step(gdk::Key::Up, none, true), Some(-1));
        assert_eq!(
            rename_step(gdk::Key::Down, none, false),
            None,
            "icons move the text cursor"
        );
        assert_eq!(rename_step(gdk::Key::Return, none, true), None);
        let control = gdk::ModifierType::CONTROL_MASK;
        assert_eq!(
            rename_step(gdk::Key::Tab, control, false),
            None,
            "Ctrl+Tab switches tabs"
        );
    }
}
