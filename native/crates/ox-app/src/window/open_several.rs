// SPDX-License-Identifier: AGPL-3.0-only
//! Opening several selected items at once (Enter or Open), as Dolphin's
//! `itemsActivated` does: each folder in a background tab of its own, each
//! file in its default application, and a question first when more than
//! five would open (OPEN-003). One selected item is opened as `openEntry`
//! opens it ([`BrowserWindow::activate_item`]).

use gtk::glib;
use ox_core::entry::Entry;

use super::activation::{activation_for, Activation};
use super::dialog::{ButtonStyle, Dialog};
use super::session::TabPlacement;
use super::BrowserWindow;

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
        let entries: Vec<Entry> = positions
            .into_iter()
            .filter_map(|position| model.item(position))
            .map(|item| item.entry().clone())
            .collect();
        if entries.len() <= MANY_ITEMS {
            self.open_each(&entries);
            return;
        }
        let question = format!("Are you sure you want to open {} items?", entries.len());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(&window, "Open", &question);
                dialog.add_cancel_button();
                let open = dialog.add_button("Open all", ButtonStyle::Primary);
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                if answer == Some(open) {
                    window.open_each(&entries);
                }
            }
        ));
    }

    /// Opens each of `entries`: folders in background tabs, in the order
    /// of the view, and files and archives in their default applications.
    /// An item that cannot be opened from here is left out.
    fn open_each(&self, entries: &[Entry]) {
        for entry in entries {
            match activation_for(entry) {
                Activation::Folder(uri) => self.open_tab_or_report(&uri, TabPlacement::Background),
                Activation::File | Activation::Archive => self.open_file(entry),
                Activation::Refused(_) => {}
            }
        }
    }
}
