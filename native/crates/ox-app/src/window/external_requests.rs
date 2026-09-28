// SPDX-License-Identifier: AGPL-3.0-only
//! What a window does for requests from outside it: "Show in folder" from
//! another application (`FileManager1`), `--select`, extra
//! `--new-window` locations and the launcher's "Open windows…".
//!
//! Ports `handleFileManagerRequest` and the `showWindows` event of
//! `desktop/ui/app.js` (INT-014, TAB-044). A request carries locations
//! to show, never anything to run: `ShowFolders` opens each folder in a new
//! tab; `ShowItems` opens each item's folder, reusing a tab that already
//! shows it, lists it again, selects exactly the requested items and
//! scrolls to the first; a file is never opened as a folder.
//! `ShowItemProperties` opens the item's folder with the item selected.

use gtk::subclass::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};
use ox_core::location::{parent_location, same_location};

use super::activation::IncomingTab;
use super::loading::LoadMode;
use super::session::{TabId, TabPlacement};
use super::BrowserWindow;

/// The requested items, grouped by the folder that holds them, in the
/// order their folders first appear (`groups` in
/// `handleFileManagerRequest`).
pub(super) fn items_by_folder(uris: &[String], home: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for uri in uris {
        let folder = parent_location(uri).unwrap_or_else(|| home.to_owned());
        match groups.iter_mut().find(|(known, _)| *known == folder) {
            Some((_, items)) => items.push(uri.clone()),
            None => groups.push((folder, vec![uri.clone()])),
        }
    }
    groups
}

impl BrowserWindow {
    /// Shows a checked `FileManager1` request or `--select`.
    pub(crate) fn show_file_manager_request(&self, request: &FileManagerRequest) {
        let home = self.imp().locations.borrow().home_uri();
        match request.method() {
            FileManagerMethod::ShowFolders => {
                for folder in request.uris() {
                    self.open_folder_tab(folder);
                }
            }
            FileManagerMethod::ShowItems => {
                for (folder, items) in items_by_folder(request.uris(), &home) {
                    self.reveal_items(&folder, items);
                }
            }
            FileManagerMethod::ShowItemProperties => {
                let first = &request.uris()[..1];
                for (folder, items) in items_by_folder(first, &home) {
                    self.select_in_new_tab(&folder, items);
                }
            }
        }
    }

    /// Opens `folder` in a new tab in front, saying in the message line
    /// why an address cannot be opened.
    fn open_folder_tab(&self, folder: &str) {
        if let Err(error) = self.open_tab(folder, TabPlacement::Foreground) {
            self.show_message(&error.to_string());
        }
    }

    /// Shows `folder` with exactly `items` selected and the first in view:
    /// in the tab that shows it already, listed again, or in a new tab.
    fn reveal_items(&self, folder: &str, items: Vec<String>) {
        let existing = self.tab_showing(folder);
        let Some(id) = existing else {
            self.select_in_new_tab(folder, items);
            return;
        };
        self.switch_tab(id);
        self.mark_selection_to_reveal(id, items);
        self.load_tab(id, LoadMode::Reload);
    }

    /// Opens `folder` in a new tab in front with `items` selected once it
    /// is listed.
    fn select_in_new_tab(&self, folder: &str, items: Vec<String>) {
        if let Err(error) = self.open_tab(folder, TabPlacement::Foreground) {
            self.show_message(&error.to_string());
            return;
        }
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active {
            self.mark_selection_to_reveal(id, items);
        }
    }

    /// The tab that shows `folder`, if any.
    fn tab_showing(&self, folder: &str) -> Option<TabId> {
        let session = self.imp().session.borrow();
        let tab = session.tabs().iter().find(|tab| same_location(tab.uri(), folder));
        tab.map(|tab| tab.id)
    }

    /// Makes tab `id` select `items` and scroll to the first when its
    /// listing finishes.
    fn mark_selection_to_reveal(&self, id: TabId, items: Vec<String>) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.selected = items;
            tab.reveals_selection = true;
        }
    }

    /// Opens `uris` as new tabs, folders in tabs and files in their
    /// applications: the locations after the first of `--new-window`.
    pub(crate) fn open_locations_as_tabs(&self, uris: Vec<String>) {
        gtk::glib::spawn_future_local(gtk::glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                for uri in &uris {
                    let result = super::activation::query_entry(uri).await;
                    window.open_incoming(uri, IncomingTab::New, result);
                }
            }
        ));
    }

    /// Opens the list of open windows, as the title bar's button does:
    /// `--windows` and the launcher's "Open windows…" (TAB-044).
    pub(crate) fn show_open_windows(&self) {
        self.imp().open_windows_button.popup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: INT-014
    #[test]
    fn items_are_grouped_by_their_folder_in_request_order() {
        let uris = [
            "file:///home/demo/Downloads/a.pdf".to_owned(),
            "file:///home/demo/Music/b.mp3".to_owned(),
            "file:///home/demo/Downloads/c.zip".to_owned(),
        ];
        let groups = items_by_folder(&uris, "file:///home/demo");
        assert_eq!(
            groups,
            vec![
                (
                    "file:///home/demo/Downloads".to_owned(),
                    vec![uris[0].clone(), uris[2].clone()]
                ),
                ("file:///home/demo/Music".to_owned(), vec![uris[1].clone()]),
            ]
        );
    }
}
