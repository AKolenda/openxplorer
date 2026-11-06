// SPDX-License-Identifier: AGPL-3.0-only
//! Rename (F2): the one selected item gets a new name in its folder
//! (OPS-008, OPS-009, OPS-010).
//!
//! Ports `rename` in `v2.0.0:desktop/ui/app.js`. It does nothing unless an item
//! is selected, no operation runs, and the item can be changed: not a
//! share root, a virtual entry or a previous version. With several items
//! selected, the name typed for one of them goes to all of them with a
//! number, as in Windows Explorer ([`super::numbered_rename`]), where the
//! Python app did nothing. As in
//! Explorer and Dolphin, the name is edited in place in its row or tile
//! ([`super::inline_rename`]); when the item's cell is not on screen, the
//! Python app's Rename dialog asks instead. Either way the name is checked
//! with the Python messages and the rename never overwrites. The renamed
//! item is selected afterwards (the Python app cleared the selection) and
//! Undo renames it back (OPS-029); as in Windows Explorer, no message says
//! it was renamed.

use gtk::gio::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{entry_from_info, Entry, ATTRIBUTES};
use ox_core::location::parent_location;
use ox_core::ops::{rename_item, OperationContext, RenamedItem};

use super::name_dialog::{ask_for_name, stem_length, NameRequest, NameSelection};
use super::FileCommand;
use crate::window::BrowserWindow;

/// The Rename dialog's line when the user keeps a name that would hide
/// the item.
pub(super) const NOT_RENAMED: &str = crate::i18n::message_id("The name was not changed.");

/// How much of `entry`'s name a rename selects, as in Windows 11: a
/// file's name before its last extension ([`stem_length`]), the whole name
/// of a folder or a shortcut. A shortcut here is a symbolic link named
/// exactly as Explorer shows one, "Report.txt - Shortcut", with no hidden
/// `.lnk`: Windows selects all of a shortcut's shown name, since its own
/// extension is hidden, and so does this.
pub(super) fn name_selection(entry: &Entry) -> NameSelection {
    if entry.is_dir || entry.is_symlink {
        NameSelection::Whole
    } else {
        NameSelection::Stem
    }
}

/// Whether `entry`'s name has no extension for a rename to keep or ask
/// about: a folder's or a shortcut's ([`name_selection`]).
pub(super) fn has_no_extension(entry: &Entry) -> bool {
    name_selection(entry) == NameSelection::Whole
}

/// How many characters of `entry`'s name a rename selects
/// ([`name_selection`]).
pub(super) fn selected_name_length(entry: &Entry) -> usize {
    match name_selection(entry) {
        NameSelection::Whole => entry.name.chars().count(),
        NameSelection::Stem => stem_length(&entry.name),
    }
}

/// Where `shown` is after the folder `old` was renamed to `new`: `None`
/// unless `shown` is `old` or inside it.
fn moved_location(shown: &str, old: &str, new: &str) -> Option<String> {
    let shown_file = gio::File::for_uri(shown);
    let old_file = gio::File::for_uri(old);
    if shown_file.equal(&old_file) {
        return Some(new.to_owned());
    }
    let inside = old_file.relative_path(&shown_file)?;
    Some(
        gio::File::for_uri(new)
            .resolve_relative_path(inside)
            .uri()
            .to_string(),
    )
}

impl BrowserWindow {
    /// F2: renames the selected item in place, or with the dialog when its
    /// cell is not on screen; several selected items get one numbered name
    /// (OPS-050).
    pub(crate) async fn rename_selection(&self) {
        if !self.allows(FileCommand::Rename) || self.refuses_writes() {
            return;
        }
        let model = self.folder_pane().model();
        let selected = model.selected_positions();
        if selected.len() > 1 {
            self.rename_several_selected(&selected).await;
            return;
        }
        let Some(position) = model.first_selected() else {
            return;
        };
        let Some(item) = model.item(position) else {
            return;
        };
        let view = self.folder_pane().view_widget();
        match self.folder_pane().owners().file_cell_at(position, &view) {
            Some(cell) => self.rename_in_place(&cell, item.entry(), None),
            None => self.rename_with_dialog(item.entry()).await,
        }
    }

    /// Rename items…: renames the several selected items with the "Rename
    /// items" dialog and its `#` template (OPS-014); nothing with one item
    /// or none selected.
    pub(crate) async fn rename_items_selection(&self) {
        if !self.allows(FileCommand::Rename) || self.refuses_writes() {
            return;
        }
        let selected = self.folder_pane().model().selected_items();
        if selected.len() < 2 {
            return;
        }
        let entries: Vec<Entry> = selected.iter().map(|item| item.entry().clone()).collect();
        self.rename_several(&entries).await;
    }

    /// Several items selected at `positions`, ascending: edits the name of
    /// the focused one, or of the first when focus is on none of them, and
    /// gives the typed name to all of them (OPS-050).
    async fn rename_several_selected(&self, positions: &[u32]) {
        let pane = self.folder_pane();
        let model = pane.model();
        let entries: Vec<Entry> = positions
            .iter()
            .filter_map(|&position| model.item(position).map(|item| item.entry().clone()))
            .collect();
        let edited = pane
            .focused_position()
            .filter(|position| positions.contains(position))
            .or_else(|| positions.first().copied());
        let Some((position, item)) = edited.and_then(|position| Some((position, model.item(position)?)))
        else {
            return;
        };
        let view = pane.view_widget();
        match pane.owners().file_cell_at(position, &view) {
            Some(cell) => self.rename_in_place(&cell, item.entry(), Some(entries)),
            None => self.rename_numbered_with_dialog(&entries, item.entry()).await,
        }
    }

    /// Rename… of the folder tree's folder at `uri` (SIDE-028): asks for
    /// its new name in the Rename dialog.
    pub(crate) async fn rename_folder_at(&self, uri: &str) {
        if self.refuses_writes() {
            return;
        }
        let file = gio::File::for_uri(uri);
        let info = file
            .query_info_future(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
            .await;
        let info = match info {
            Ok(info) => info,
            Err(error) => {
                self.show_message(error.message());
                return;
            }
        };
        let Some(renamed) = self.ask_and_rename(&entry_from_info(&file, &info)).await else {
            return;
        };
        // The folder shown, or one it is in, moves with its new name, as
        // Dolphin's view follows a renamed folder.
        let shown = self.current_uri().filter(|_| !renamed.is_unchanged());
        let moved = shown.and_then(|shown| moved_location(&shown, &renamed.original_uri, &renamed.uri));
        match moved {
            Some(moved) => {
                self.remember_rename(&renamed);
                self.navigate_or_report(&moved);
            }
            None => self.finish_rename(renamed),
        }
    }

    /// Asks for `entry`'s new name in the Rename dialog and renames it.
    async fn rename_with_dialog(&self, entry: &Entry) {
        if let Some(renamed) = self.ask_and_rename(entry).await {
            self.finish_rename(renamed);
        }
    }

    /// Asks for `entry`'s new name in the Rename dialog and renames it;
    /// the renamed item, or `None` when the user cancelled.
    async fn ask_and_rename(&self, entry: &Entry) -> Option<RenamedItem> {
        let selection = name_selection(entry);
        let folder = parent_location(&entry.uri).unwrap_or_default();
        let request = NameRequest {
            title: "Rename",
            initial_name: &entry.name,
            selection,
            folder: &folder,
        };
        ask_for_name(self, request, |name, cancel| {
            let uri = entry.uri.clone();
            let old_name = entry.name.clone();
            // A shortcut's name, like a folder's, has no extension to ask
            // about.
            let has_no_extension = selection == NameSelection::Whole;
            let window = self.downgrade();
            async move {
                let window = window
                    .upgrade()
                    .ok_or_else(|| ox_core::i18n::gettext_static(NOT_RENAMED).to_owned())?;
                // A name the backend refuses anyway is refused before
                // anything is asked, with the message that says why.
                ox_core::location::validate_name(&name).map_err(|error| error.to_string())?;
                let confirmed = window
                    .confirm_extension_change(&old_name, &name, has_no_extension)
                    .await
                    && window.confirm_hiding_rename(&old_name, &name).await;
                if !confirmed {
                    return Err(ox_core::i18n::gettext_static(NOT_RENAMED).to_owned());
                }
                let (admitted, _writing) = window.admit_write()?;
                let context = OperationContext { cancel, ..admitted };
                rename_item(&uri, &name, &context)
                    .await
                    .map_err(|error| error.to_string())
            }
        })
        .await
    }

    /// Renames the item at `uri`, called `old_name`, to `name`, as the
    /// name field of Properties asks (PROP-005): the same checks, question
    /// about a changed extension, asked over `parent`, the Properties
    /// window, write protection and Undo as Rename. The error is the
    /// message to show.
    pub(crate) async fn rename_item_at(
        &self,
        parent: &gtk::Window,
        uri: &str,
        old_name: &str,
        name: &str,
        is_folder: bool,
    ) -> Result<(), String> {
        // An invalid name is refused before anything is asked, as renaming
        // in place and the Rename dialog refuse it.
        ox_core::location::validate_name(name).map_err(|error| error.to_string())?;
        if !self
            .confirm_extension_change_over(parent, old_name, name, is_folder)
            .await
        {
            return Err(ox_core::i18n::gettext_static(NOT_RENAMED).to_owned());
        }
        let (context, _writing) = self.admit_write()?;
        let renamed = rename_item(uri, name, &context)
            .await
            .map_err(|error| error.to_string())?;
        self.finish_rename(renamed);
        Ok(())
    }

    /// Remembers the rename for Undo and selects the item under its new
    /// name.
    pub(super) fn finish_rename(&self, renamed: RenamedItem) {
        self.remember_rename(&renamed);
        self.reload_selecting(vec![renamed.uri]);
    }

    /// Like [`Self::finish_rename`], but selects `next` and starts
    /// renaming it once the folder is listed again: Tab moved on (OPS-012).
    pub(super) fn finish_rename_and_continue(&self, renamed: &RenamedItem, next: String) {
        self.remember_rename(renamed);
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.renames_selection = true;
        }
        self.reload_selecting(vec![next]);
    }

    /// Records the rename for Undo.
    fn remember_rename(&self, renamed: &RenamedItem) {
        if let Some(record) = renamed.undo_record() {
            self.context().record_operation(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As in Windows 11, Rename selects a file's name before its last
    /// extension and the whole name of a folder or a shortcut, the
    /// symbolic link named as Explorer names one ("Report.txt -
    /// Shortcut"), whose hidden `.lnk` Windows keeps out of the field.
    ///
    /// parity: OPS-010
    #[test]
    fn rename_selects_a_files_name_before_its_extension_and_all_of_a_shortcut() {
        let file = crate::test_support::file_entry("my.notes.txt");
        let archive = crate::test_support::file_entry("archive.tar.gz");
        let folder = crate::test_support::folder_entry("v1.2 release");
        let mut shortcut = crate::test_support::file_entry("Report.txt - Shortcut");
        shortcut.is_symlink = true;
        let dotfile = crate::test_support::file_entry(".hidden");

        assert_eq!(name_selection(&file), NameSelection::Stem);
        assert_eq!(selected_name_length(&file), 8, "my.notes");
        assert_eq!(selected_name_length(&archive), 11, "archive.tar");
        assert_eq!(name_selection(&folder), NameSelection::Whole);
        assert_eq!(selected_name_length(&folder), 12);
        assert_eq!(name_selection(&shortcut), NameSelection::Whole);
        assert_eq!(selected_name_length(&shortcut), 21);
        assert_eq!(selected_name_length(&dotfile), 7);
    }
}
