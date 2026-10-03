// SPDX-License-Identifier: AGPL-3.0-only
//! Opening several selected items at once (Enter or Open), as Dolphin's
//! `itemsActivated` does: each folder in a background tab of its own, each
//! file in its default application, and a question first when more than
//! five would open (OPEN-003). One selected item is opened as `openEntry`
//! opens it ([`BrowserWindow::activate_item`]).

use gtk::glib;
use ox_core::entry::Entry;

use super::activation::{activation_for, desktop_link, Activation};
use super::session::TabPlacement;
use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;

/// More items than this at once are asked about first (Dolphin's limit).
const MANY_ITEMS: usize = 5;

impl BrowserWindow {
    /// Opens the selected items: one as a double-click does, several each
    /// in its own way.
    pub(super) fn open_selection(&self) {
        let model = self.folder_pane().model();
        let positions = model.selected_positions();
        if let [position] = positions.as_slice() {
            self.activate_item(*position);
            return;
        }
        // A file dialog takes several files as its choice (INT-032).
        if self.pick_selection() {
            return;
        }
        let entries: Vec<Entry> = positions
            .into_iter()
            .filter_map(|position| model.item(position))
            .map(|item| item.entry().clone())
            .collect();
        if entries.len() <= MANY_ITEMS {
            self.open_each(&entries);
            return;
        }
        let question = ox_core::i18n::format_message(
            "Are you sure you want to open {len} items?",
            &[("len", &entries.len().to_string())],
        );
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(&window, &ox_core::i18n::gettext("Open"), &question);
                dialog.add_cancel_button();
                let open = dialog.add_button(&ox_core::i18n::gettext("Open all"), ButtonStyle::Accent);
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                if answer == Some(open) {
                    window.open_each(&entries);
                }
            }
        ));
    }

    /// Opens each of `entries`: folders, and `.desktop` links to folders,
    /// in background tabs, in the order of the view, and files in their
    /// default applications. An archive opens in its default application
    /// too, since browsing one is a dialog of its own and several would
    /// stack. An item that cannot be opened from here is left out.
    fn open_each(&self, entries: &[Entry]) {
        for entry in entries {
            // Inside a ZIP opened like a folder: folders in background
            // tabs, files as their private copies (ARC-026).
            if let Some(inside) = super::zip_folder::archive_location(&entry.uri) {
                if inside.is_folder() {
                    self.open_tab_or_report(&entry.uri, TabPlacement::Background);
                } else {
                    self.activate_zip_member(entry);
                }
                continue;
            }
            match activation_for(entry) {
                Activation::Folder(uri) => self.open_tab_or_report(&uri, TabPlacement::Background),
                Activation::File => match desktop_link(entry) {
                    Some(target) => self.follow_link_in_background(entry, target),
                    None => self.open_file(entry),
                },
                Activation::Archive => self.open_file(entry),
                Activation::Refused(_) => {}
            }
        }
    }
}
