// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming in place: the name becomes a text field in its row or tile
//! (OPS-010).
//!
//! As in Windows 11, the field starts with a file's name before its last
//! extension selected ("archive.tar" of "archive.tar.gz"), or the whole
//! name of a folder, a shortcut or a name with no extension
//! ([`super::rename::name_selection`]). Enter or moving focus
//! away commits; Escape cancels. The field's own right-click menu and
//! switching to another window keep the rename going. The name is checked with the Python Rename
//! dialog's rules and messages (`validateName`, then the backend's); a
//! refusal, such as a taken name, shows in the toast and editing goes on,
//! so the typed name is not lost. While the rename runs the field is
//! disabled, so a second commit cannot start it again. A name that would
//! hide the item asks first ([`super::hide_confirm`]). No to the question
//! about a changed extension ([`super::extension_confirm`]) ends the
//! rename with the old name, as in Windows 11. As in Windows 11, Tab
//! and Shift+Tab commit and go on to rename the next or previous item
//! (OPS-012), while Down and Up only move the text cursor to the very end
//! and the very start of the name, as End and Home do, as Windows 11's
//! one-line name box does (checked in Windows 11 26200). The row keeps its
//! look while the name is edited: no focus outline is drawn around it. With several items
//! selected, the field edits one of them and Enter gives the typed name to
//! all of them with a number (OPS-050, [`super::numbered_rename`]); Tab is
//! then the field's own key. Ctrl+A selects the whole
//! name in the field and nothing else: GTK 4.14's text field selects its
//! text but lets the key go on to the view, which selected every item too.
//!
//! As in Windows, a character Windows forbids in names is refused as it
//! is typed or pasted, with Windows' balloon ([`super::name_balloon`]),
//! and a file renamed to the name of another file in its folder asks
//! Windows' "Rename File" question, which offers the first free numbered
//! name ([`super::rename_conflict`]).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::entry::Entry;
use ox_core::ops::{rename_item, OpsError, RenamedItem};

use super::name_balloon::refuse_forbidden_characters;
use super::names::check_typed_name;
use super::numbered_rename::numbered_batch_for;
use super::rename::{has_no_extension, selected_name_length};
use super::rename_conflict::{MergeAnswer, TakenNameAnswer};
use crate::folder_view::cells::FileCell;
use crate::folder_view::shown_name::{hidden_extension, shown_name, shows_whole};
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
    /// True for a folder, whose name has no extension.
    is_folder: bool,
    /// True for a folder or a shortcut, whose name has no extension to
    /// ask about changing: a Windows shortcut's own `.lnk` is hidden and
    /// kept, so Explorer never asks.
    has_no_extension: bool,
    /// The extension the field leaves out, with its dot, while names hide
    /// known types' extensions; the rename puts it back, as Windows'
    /// does (VIEW-069).
    hidden_extension: Option<String>,
    /// Every selected item in the order shown, the edited one included,
    /// when several are renamed together; `None` for one item.
    several: Option<Rc<[Entry]>>,
}

/// How many places Tab or Shift+Tab move the rename on from the item
/// being renamed. `None` for any other key, and with Ctrl or Alt, which
/// keep Ctrl+Tab and Ctrl+Shift+Tab for switching tabs.
fn rename_step(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<i32> {
    if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
        return None;
    }
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    match key {
        gdk::Key::Tab | gdk::Key::KP_Tab if !shift => Some(1),
        gdk::Key::ISO_Left_Tab | gdk::Key::Tab | gdk::Key::KP_Tab => Some(-1),
        _ => None,
    }
}

/// Which end of the name Down (1, the end) or Up (-1, the start) moves
/// the text cursor to, as End and Home do in a one-line name box in
/// Windows; `None` for any other key, and with Ctrl or Alt. GTK's own Down
/// and Up leave a one-line field.
fn caret_step(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<i32> {
    if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
        return None;
    }
    match key {
        gdk::Key::Down | gdk::Key::KP_Down => Some(1),
        gdk::Key::Up | gdk::Key::KP_Up => Some(-1),
        _ => None,
    }
}

/// Whether `key` with `modifiers` is Ctrl+A, with Caps Lock or not.
fn is_select_all(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let others = gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK
        | gdk::ModifierType::META_MASK;
    key.to_lower() == gdk::Key::a
        && modifiers.contains(gdk::ModifierType::CONTROL_MASK)
        && !modifiers.intersects(others)
}

impl BrowserWindow {
    /// Starts editing `entry`'s name in `cell`; with `several`, the
    /// selected items in the order shown, the typed name goes to them all.
    pub(super) fn rename_in_place(&self, cell: &FileCell, entry: &Entry, several: Option<Vec<Entry>>) {
        let hidden_extension =
            hidden_extension(&entry.name, shows_whole(entry), self.file_name_extensions()).map(str::to_owned);
        let shown = shown_name(&entry.name, shows_whole(entry), self.file_name_extensions());
        let editor = gtk::Entry::builder()
            .text(shown)
            .hexpand(true)
            .css_classes([NAME_EDITOR_CLASS])
            .build();
        editor.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "New name",
        ))]);
        let target = RenameTarget {
            uri: entry.uri.clone(),
            name: entry.name.clone(),
            is_folder: entry.is_dir,
            has_no_extension: has_no_extension(entry),
            several: several.map(Rc::from),
            hidden_extension: hidden_extension.clone(),
        };
        self.connect_name_editor(cell, &editor, &target);
        // Added last, its key handler runs before the field's other ones.
        refuse_forbidden_characters(&editor);
        cell.show_name_editor(&editor);
        editor.grab_focus();
        // With the extension left out, the whole name shown is selected.
        let selected = if hidden_extension.is_some() {
            -1
        } else {
            i32::try_from(selected_name_length(entry)).unwrap_or(-1)
        };
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
                    move || {
                        // GTK also says focus left when the field's own
                        // right-click menu gives it back, and when the
                        // window goes to the background; the keyboard is
                        // still in the field then, and editing goes on, as
                        // in Explorer.
                        let focus = editor.root().and_then(|root| root.focus());
                        if !focus.is_some_and(|focus| focus.is_ancestor(&editor)) {
                            window.commit_name(&cell, &editor, &target);
                        }
                    }
                ));
            }
        ));
        editor.add_controller(focus);
        editor.add_controller(self.rename_keys(cell, editor, target));
    }

    /// Escape in the field ends the rename and leaves the name as it was;
    /// Ctrl+A selects the whole name; Down and Up move the text cursor to
    /// the end and the start of the name; Tab and Shift+Tab commit and go on to rename the
    /// next or previous item (OPS-012).
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
            #[strong]
            target,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if key == gdk::Key::Escape {
                    window.end_rename_in_place(&cell);
                    return glib::Propagation::Stop;
                }
                // Selected here and stopped: the field's own Ctrl+A goes on
                // to the view in GTK 4.14, which then selects every item.
                if is_select_all(key, modifiers) {
                    editor.select_region(0, -1);
                    return glib::Propagation::Stop;
                }
                if let Some(step) = caret_step(key, modifiers) {
                    if let Some(text) = editor.delegate().and_downcast::<gtk::Text>() {
                        let extend = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                        text.emit_by_name::<()>(
                            "move-cursor",
                            &[&gtk::MovementStep::BufferEnds, &step, &extend],
                        );
                    }
                    return glib::Propagation::Stop;
                }
                // Several items get one name: there is no next one to
                // rename.
                if target.several.is_some() {
                    return glib::Propagation::Proceed;
                }
                let Some(step) = rename_step(key, modifiers) else {
                    return glib::Propagation::Proceed;
                };
                let next = window.neighbour_uri(&target.uri, step);
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
        // A name erased whole (or left as spaces) keeps the old name, as
        // in Windows 11: no rename, no message, nothing to undo. It never
        // becomes the extension alone.
        let typed = match &target.hidden_extension {
            _ if editor.text().trim().is_empty() => target.name.clone(),
            Some(extension) => format!("{}{extension}", editor.text()),
            None => editor.text().to_string(),
        };
        // A name the shell trims back to the old one ("Notes.txt." for
        // "Notes.txt") is no new name either.
        let unchanged =
            typed == target.name || check_typed_name(&typed).is_ok_and(|name| name == target.name);
        if unchanged {
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
        if let Some(several) = &target.several {
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
                #[strong]
                several,
                async move {
                    window
                        .run_several_in_place(&cell, &editor, &target, &several, &name)
                        .await;
                }
            ));
            return;
        }
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
        // An update may have finished installing, or a drive started
        // formatting, while the field was open.
        if self.refuses_writes() {
            self.take_rename_next();
            self.close_name_editor(cell);
            return;
        }
        // A name the backend refuses anyway is refused before anything is
        // asked, with the message that says why (more than 255 bytes).
        if let Err(error) = ox_core::location::validate_name(name) {
            self.take_rename_next();
            editor.set_sensitive(true);
            editor.grab_focus();
            self.show_message(&error.to_string());
            return;
        }
        // No to the extension question ends the rename with the old name,
        // the name box closed, as in Windows 11 File Explorer.
        if !self
            .confirm_extension_change(&target.name, name, target.has_no_extension)
            .await
        {
            self.take_rename_next();
            self.close_name_editor(cell);
            return;
        }
        if !self.confirm_hiding_rename(&target.name, name).await {
            self.take_rename_next();
            editor.set_sensitive(true);
            editor.grab_focus();
            return;
        }
        let Some(renamed) = self.rename_or_offer_number(cell, editor, target, name).await else {
            return;
        };
        match renamed {
            Ok(renamed) => {
                // Before the folder is listed again, which rebinds the cell.
                self.close_name_editor(cell);
                match self.take_rename_next() {
                    Some(next) => self.finish_rename_and_continue(&renamed, next),
                    None => self.finish_rename_in_place(&renamed),
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

    /// Renames the item to `name`; when another file has that name, asks
    /// Windows' "Rename File" question and, on Yes, renames it to the
    /// numbered name it offered. `None` when nothing is left to report:
    /// the write was refused, or No gave the field back for another try.
    async fn rename_or_offer_number(
        &self,
        cell: &FileCell,
        editor: &gtk::Entry,
        target: &RenameTarget,
        name: &str,
    ) -> Option<Result<RenamedItem, OpsError>> {
        // A drive may have started formatting during the questions.
        let refusal = match self.admitted_rename(cell, &target.uri, name).await? {
            Err(OpsError::Exists(refusal)) => refusal,
            renamed => return Some(renamed),
        };
        if target.is_folder {
            return match self.ask_about_merging(&target.uri, name).await {
                MergeAnswer::Merge(into) => {
                    self.take_rename_next();
                    self.close_name_editor(cell);
                    self.merge_folder_into(&target.uri, &into).await;
                    None
                }
                MergeAnswer::KeepEditing => {
                    self.take_rename_next();
                    editor.set_sensitive(true);
                    editor.grab_focus();
                    None
                }
                MergeAnswer::NotAsked => Some(Err(OpsError::Exists(refusal))),
            };
        }
        match self.ask_about_taken_name(&target.uri, name).await {
            TakenNameAnswer::Rename(numbered) => self.admitted_rename(cell, &target.uri, &numbered).await,
            TakenNameAnswer::KeepEditing => {
                self.take_rename_next();
                editor.set_sensitive(true);
                editor.grab_focus();
                None
            }
            TakenNameAnswer::NotAsked => Some(Err(OpsError::Exists(refusal))),
        }
    }

    /// Renames the item at `uri` to `name` once the window admits a write;
    /// `None`, with the field closed and the refusal in the toast, when it
    /// does not, as when a drive started formatting during a question.
    async fn admitted_rename(
        &self,
        cell: &FileCell,
        uri: &str,
        name: &str,
    ) -> Option<Result<RenamedItem, OpsError>> {
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.take_rename_next();
                self.close_name_editor(cell);
                self.show_message(&refusal);
                return None;
            }
        };
        // Its player stops before the file is renamed.
        let _players = crate::media_player::hold_players_within(&[uri.to_owned()]).await;
        Some(rename_item(uri, name, &context).await)
    }

    /// Gives every item of `several` the name typed for the edited one,
    /// numbered; a name refused before anything is renamed keeps the field
    /// for another try.
    async fn run_several_in_place(
        &self,
        cell: &FileCell,
        editor: &gtk::Entry,
        target: &RenameTarget,
        several: &[Entry],
        name: &str,
    ) {
        if self.refuses_writes() {
            self.close_name_editor(cell);
            return;
        }
        let Some(edited) = several.iter().find(|entry| entry.uri == target.uri) else {
            self.close_name_editor(cell);
            return;
        };
        let batch = numbered_batch_for(several, edited, name);
        let base = match batch.base_name() {
            Ok(base) => base,
            Err(error) => {
                editor.set_sensitive(true);
                editor.grab_focus();
                self.show_message(&error.to_string());
                return;
            }
        };
        if !self.confirm_numbered_extension(&batch, &base).await
            || !self.confirm_hiding_rename(&target.name, &base).await
        {
            editor.set_sensitive(true);
            editor.grab_focus();
            return;
        }
        // Before the folder is listed again, which rebinds the cell.
        self.close_name_editor(cell);
        self.run_numbered_rename(&batch).await;
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
    fn tab_moves_the_rename_on_and_the_arrows_move_the_cursor() {
        let none = gdk::ModifierType::empty();
        let shift = gdk::ModifierType::SHIFT_MASK;
        assert_eq!(rename_step(gdk::Key::Tab, none), Some(1));
        assert_eq!(rename_step(gdk::Key::ISO_Left_Tab, shift), Some(-1));
        assert_eq!(rename_step(gdk::Key::Down, none), None, "Down stays in the name");
        assert_eq!(rename_step(gdk::Key::Return, none), None);
        let control = gdk::ModifierType::CONTROL_MASK;
        assert_eq!(
            rename_step(gdk::Key::Tab, control),
            None,
            "Ctrl+Tab switches tabs"
        );
        assert_eq!(caret_step(gdk::Key::Down, none), Some(1));
        assert_eq!(caret_step(gdk::Key::Up, shift), Some(-1));
        assert_eq!(caret_step(gdk::Key::Left, none), None);
        assert_eq!(caret_step(gdk::Key::Up, control), None);
    }
}
