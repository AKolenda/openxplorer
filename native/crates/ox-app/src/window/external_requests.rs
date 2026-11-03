// SPDX-License-Identifier: AGPL-3.0-only
//! What a window does for requests from outside it: "Show in folder" from
//! another application (`FileManager1`), `--select`, extra
//! `--new-window` locations and the launcher's "Open windows…".
//!
//! Ports `handleFileManagerRequest` and the `showWindows` event of
//! `v2.0.0:desktop/ui/app.js` (INT-014, TAB-044). A request carries locations
//! to show, never anything to run: `ShowFolders` opens each folder in a new
//! tab; `ShowItems` opens each item's folder, reusing a tab that already
//! shows it, lists it again, selects exactly the requested items and
//! scrolls to the first; a file is never opened as a folder.
//! `ShowItemProperties` opens the item's folder in a new tab with the
//! item selected, and the item's Properties over it. When Settings opens
//! folders from other apps in a new window, the application gives the
//! request a window of its own, whose tabs are only the request's.
//!
//! Any program on the session bus can send a request, so a share it names
//! is never mounted for it: the tab says the share is not mounted, and
//! connects, signing in with a saved password if there is one, only when
//! the user chooses Try again (NET-004).

use gtk::subclass::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};
use ox_core::location::{parent_location, same_location};

use crate::properties::PropertiesTab;

use super::activation::IncomingTab;
use super::loading::LoadMode;
use super::session::TabId;
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
        self.open_request_tabs(request);
        self.open_request_properties(request);
    }

    /// The tabs of a checked `FileManager1` request: each folder, or each
    /// item's folder with the items selected. A window opened for the
    /// request (INT-014, NAV-042) takes them before it is shown, as its
    /// only tabs; it is left without a tab when none could be opened.
    pub(crate) fn open_request_tabs(&self, request: &FileManagerRequest) {
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

    /// The Properties of a checked `ShowItemProperties` request's item,
    /// over the window once it is shown; nothing for the other methods.
    pub(crate) fn open_request_properties(&self, request: &FileManagerRequest) {
        if request.method() == FileManagerMethod::ShowItemProperties {
            self.open_properties_of(&request.uris()[0], PropertiesTab::General);
        }
    }

    /// Opens `folder` in a new tab in front, saying in the message line
    /// why an address cannot be opened.
    fn open_folder_tab(&self, folder: &str) {
        if let Err(error) = self.add_tab(folder) {
            self.show_message(&error.to_string());
            return;
        }
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active {
            self.wait_to_mount(id);
        }
    }

    /// Makes the listing tab `id` runs, or runs next, leave its share
    /// unmounted until the user asks to connect.
    pub(super) fn wait_to_mount(&self, id: TabId) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.waits_to_mount = true;
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
        self.wait_to_mount(id);
        self.load_tab(id, LoadMode::Reload);
    }

    /// Opens `folder` in a new tab in front with `items` selected once it
    /// is listed.
    fn select_in_new_tab(&self, folder: &str, items: Vec<String>) {
        if let Err(error) = self.add_tab(folder) {
            self.show_message(&error.to_string());
            return;
        }
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active {
            self.wait_to_mount(id);
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
