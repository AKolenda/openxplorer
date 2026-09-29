// SPDX-License-Identifier: AGPL-3.0-only
//! Opening items and typed addresses: folders open in the tab, files in
//! their default application.
//!
//! Ports `openEntry`, `submitAddress` and `openIncoming` in
//! `desktop/ui/app.js` and `activation_kind` in `desktop/activation.py`.
//! An address or command-line argument is looked up first, so a file is
//! opened without moving the tab or adding a history entry, and a typed
//! page title ("Network") names a folder of that name when one exists.
//! Only an explicit request (Enter, double-click, Open, a typed address or
//! a command-line argument) ever launches an application.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry, EntryError, EntryKind};
use ox_core::integration;

use crate::locations::{self, Page};

use super::BrowserWindow;

/// Why an item cannot be opened (`activation_kind` in activation.py).
const NOT_OPENABLE: &str = "This item is not a regular file or a readable folder.";

/// What activating an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Activation {
    /// Open this folder in the tab.
    Folder(String),
    /// Open the file in its default application.
    File,
    /// Browse the ZIP archive in the archive browser (ARC-002).
    Archive,
    /// Refuse, with this message.
    Refused(&'static str),
}

/// Where a folder from the command line or another app opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IncomingTab {
    /// The active tab moves to it: the first location, as `openIncoming`.
    Active,
    /// A new tab in front: every later location.
    New,
}

/// What activating `entry` does, from freshly queried metadata.
pub(super) fn activation_for(entry: &Entry) -> Activation {
    let not_a_file = !matches!(
        entry.kind,
        EntryKind::File | EntryKind::Special | EntryKind::Symlink
    );
    if entry.kind == EntryKind::Directory || (not_a_file && entry.is_dir) {
        return Activation::Folder(entry.navigation_uri().to_owned());
    }
    if matches!(
        entry.kind,
        EntryKind::Special | EntryKind::Unknown | EntryKind::Symlink
    ) {
        return Activation::Refused(NOT_OPENABLE);
    }
    if integration::Activation::for_entry(entry) == Ok(integration::Activation::BrowseArchive) {
        return Activation::Archive;
    }
    Activation::File
}

/// Queries `uri` without blocking the interface.
pub(super) async fn query_entry(uri: &str) -> Result<Entry, EntryError> {
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    Ok(entry::entry_from_info(&file, &info))
}

impl BrowserWindow {
    /// Opens the item at a display position (Enter, double-click, Open).
    pub(super) fn activate_item(&self, position: u32) {
        if let Some(item) = self.folder_pane().model().item(position) {
            self.activate_entry(item.entry());
        }
    }

    fn activate_entry(&self, entry: &Entry) {
        match activation_for(entry) {
            Activation::Folder(uri) => self.navigate_or_report(&uri),
            Activation::File => {
                let file = entry.clone();
                self.after_mounting(&entry.uri, move |window| window.open_file(&file));
            }
            Activation::Archive => {
                let archive = entry.clone();
                self.after_mounting(&entry.uri, move |window| window.open_archive(&archive));
            }
            Activation::Refused(message) => self.show_message(message),
        }
    }

    /// Opens a file in its default application and records it among the
    /// recent files.
    fn open_file(&self, entry: &Entry) {
        let on_error = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |error: glib::Error| window.show_message(&error.to_string())
        );
        self.context().open_file(entry, self.upcast_ref(), on_error);
    }

    /// Opens the file at `uri`, which the tab tried to list as a folder.
    pub(super) fn open_file_location(&self, uri: &str) {
        let uri = uri.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match query_entry(&uri).await {
                    Ok(entry) if activation_for(&entry) == Activation::File => window.open_file(&entry),
                    Ok(entry) if activation_for(&entry) == Activation::Archive => window.open_archive(&entry),
                    Ok(_) => {}
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
    }

    /// Opens what was typed into the address bar and pressed Enter on.
    pub(super) fn submit_address(&self, text: &str) {
        let typed = text.trim();
        let current = self.current_uri();
        let place = self.place_titled(typed);
        let unchanged_page = place.is_some() && place == current;
        let is_page_uri = locations::is_home_alias(typed) || Page::from_uri(typed).is_some();
        if unchanged_page || is_page_uri {
            self.finish_address();
            self.navigate_or_report(typed);
            return;
        }
        let folder = match self.resolve_relative(text) {
            Ok(folder) => folder,
            Err(error) => {
                match place {
                    Some(place) => self.navigate_or_report(&place),
                    None => self.show_message(&error.to_string()),
                }
                return;
            }
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = query_entry(&folder).await;
                window.open_typed_location(&folder, place.as_deref(), result);
            }
        ));
    }

    fn open_typed_location(&self, uri: &str, place: Option<&str>, result: Result<Entry, EntryError>) {
        let entry = match (result, place) {
            (Ok(entry), _) => entry,
            (Err(EntryError::NotFound(_)), Some(place)) => {
                self.finish_address();
                self.navigate_or_report(place);
                return;
            }
            // An unmounted share opens in the tab, which says why it is
            // unavailable and offers Try again.
            (Err(EntryError::NotMounted(_)), _) => {
                self.finish_address();
                self.navigate_or_report(uri);
                return;
            }
            (Err(error), _) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        if !matches!(activation_for(&entry), Activation::Refused(_)) {
            self.finish_address();
        }
        self.activate_entry(&entry);
    }

    /// Opens command-line or desktop locations, as `openIncoming`: the
    /// first folder in the active tab, the others in new tabs, and files
    /// in their applications.
    pub(crate) fn open_locations(&self, uris: Vec<String>) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                for (index, uri) in uris.iter().enumerate() {
                    let tab = if index == 0 {
                        IncomingTab::Active
                    } else {
                        IncomingTab::New
                    };
                    let result = query_entry(uri).await;
                    window.open_incoming(uri, tab, result);
                }
            }
        ));
    }

    /// Opens one incoming location, whose metadata query gave `result`.
    pub(super) fn open_incoming(&self, uri: &str, tab: IncomingTab, result: Result<Entry, EntryError>) {
        let Ok(entry) = result else {
            // A missing or unreadable location opens as a tab that says so.
            self.open_incoming_folder(uri, tab);
            return;
        };
        match activation_for(&entry) {
            Activation::Folder(folder) => self.open_incoming_folder(&folder, tab),
            Activation::File => self.open_file(&entry),
            Activation::Archive => self.open_archive(&entry),
            Activation::Refused(message) => self.show_message(message),
        }
    }

    /// Opens the folder `uri` where `tab` says, showing an address the app
    /// cannot open in the message line.
    fn open_incoming_folder(&self, uri: &str, tab: IncomingTab) {
        let opened = match tab {
            IncomingTab::Active => self.navigate(uri),
            IncomingTab::New => self.add_tab(uri),
        };
        if let Err(error) = opened {
            self.show_message(&error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// parity: ARC-002
    #[test]
    fn folders_open_in_the_tab_files_in_an_application_and_zips_in_the_browser() {
        let folder = folder_entry("Projects");
        assert_eq!(activation_for(&folder), Activation::Folder(folder.uri.clone()));
        assert_eq!(activation_for(&file_entry("notes.txt")), Activation::File);
        assert_eq!(activation_for(&file_entry("photos.zip")), Activation::Archive);
        assert_eq!(activation_for(&file_entry("PHOTOS.ZIP")), Activation::Archive);
        assert_eq!(
            activation_for(&folder_entry("Archive.zip")),
            Activation::Folder(folder_entry("Archive.zip").uri)
        );
    }

    #[test]
    fn special_items_and_dangling_links_are_refused() {
        for kind in [EntryKind::Special, EntryKind::Symlink, EntryKind::Unknown] {
            let mut entry = file_entry("pipe");
            entry.kind = kind;
            assert_eq!(
                activation_for(&entry),
                Activation::Refused(NOT_OPENABLE),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn shares_and_shortcuts_open_their_target() {
        let mut share = folder_entry("media");
        share.kind = EntryKind::Mountable;
        share.target_uri = Some("smb://nas/media".into());
        assert_eq!(
            activation_for(&share),
            Activation::Folder("smb://nas/media".into())
        );
    }
}
