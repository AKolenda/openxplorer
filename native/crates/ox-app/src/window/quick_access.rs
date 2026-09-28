// SPDX-License-Identifier: AGPL-3.0-only
//! Pinning folders to Quick access, and unpinning them.
//!
//! Ports `pinEntry`, `pinCurrent` and the pin half of `removeBookmark` in
//! `desktop/ui/app.js`: the one selected folder, or the folder the tab
//! shows, goes at the end of Quick access, and "Unpin from Quick access"
//! removes only the pin (a standard folder's pin is hidden), with the
//! Python app's messages. The pins are saved off the main thread through
//! the Python app's own settings file and lock; no file is moved or
//! deleted.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::pin_target;
use ox_core::location::same_location;
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, SettingsError};

use crate::locations::Page;
use crate::settings_store::Change;

use super::BrowserWindow;

impl BrowserWindow {
    /// Pins the one selected folder to Quick access (`pinEntry`).
    pub(super) fn pin_selected(&self) {
        let items = self.folder_pane().model().selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        let entry = item.entry();
        // Without a label the pin is named after the folder
        // (`label or entry['name']` in Python).
        match pin_target(entry, None) {
            Ok(target) => self.pin(target.uri, target.label),
            Err(error) => self.show_message(&error.to_string()),
        }
    }

    /// Pins the folder the tab shows (`pinCurrent`).
    pub(super) fn pin_folder(&self) {
        let Some(uri) = self.current_uri().filter(|uri| Page::from_uri(uri).is_none()) else {
            return;
        };
        let label = self.imp().locations.borrow().title_for(&uri);
        self.pin(uri, label);
    }

    /// Adds a Quick access pin, with the Python app's messages.
    fn pin(&self, uri: String, label: String) {
        let quick_access = self.places().quick_access;
        if quick_access.iter().any(|place| same_location(&place.uri, &uri)) {
            self.show_message("Already pinned to Quick access.");
            return;
        }
        let change: Change = Box::new(move |settings| {
            let request = BookmarkRequest::new(uri, label);
            // Not dropped on a row, so the pin goes at the end, and no
            // sidebar order to save with it.
            let drop_target: Option<&str> = None;
            let shown_order: Option<&[String]> = None;
            settings
                .pin_many(&[request], drop_target, shown_order)
                .map(|_pins| ())
        });
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    let message = match result {
                        Ok(()) => "Pinned to Quick access. No files were moved.".to_owned(),
                        Err(error) => format!("Could not pin: {error}"),
                    };
                    window.show_message(&message);
                }
            ),
        );
    }

    /// "Unpin from Quick access": removes the pin of `uri`; the folder
    /// stays (SIDE-009).
    pub(super) fn unpin(&self, uri: &str) {
        let request = BookmarkRequest::new(uri.to_owned(), String::new());
        let change: Change =
            Box::new(move |settings| settings.bookmark(BookmarkAction::Remove, BookmarkKind::Pin, &request));
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    let message = match result {
                        Ok(()) => "Unpinned. The folder was not deleted.".to_owned(),
                        Err(error) => error.to_string(),
                    };
                    window.show_message(&message);
                }
            ),
        );
    }
}
