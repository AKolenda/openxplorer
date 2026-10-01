// SPDX-License-Identifier: AGPL-3.0-only
//! Pinning folders to Quick access, and unpinning them.
//!
//! Ports `pinEntry`, `pinEntries`, `pinCurrent` and the pin half of
//! `removeBookmark` in `v2.0.0:desktop/ui/app.js` and the `pin` request of
//! `v2.0.0:desktop/winspace.py`: the one selected folder, or the folder the tab
//! shows, goes at the end of Quick access; folders dropped on Quick access
//! go where they were dropped (DND-014); and "Unpin from Quick access"
//! removes only the pin (a standard folder's pin is hidden), with the
//! Python app's messages. A dropped item that is not pinned yet must be a
//! folder or a share, which GIO confirms off the main thread. The pins are
//! saved off the main thread through the Python app's own settings file
//! and lock; no file is moved or deleted. One pin request runs at a time.

use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{pin_target, verify_pin, EntryError};
use ox_core::location::{normalise, same_location};
use ox_core::places::Place;
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, SettingsError};

use crate::locations::Page;
use crate::settings_store::Change;

use super::file_drag::is_draggable_location;
use super::file_drop::DropRefusal;
use super::BrowserWindow;

/// The most folders one drop pins.
const MAX_DROPPED_PINS: usize = 200;

/// The toast when a file is among the items to pin (`pinEntries`).
const FOLDERS_ONLY: &str = "Only folders and network shares can be pinned. Select folders only.";

/// The folders a drop on Quick access pins: local and SMB locations,
/// canonical, in order and without duplicates. A share may be pinned
/// whole; a server may not.
///
/// # Errors
///
/// [`DropRefusal::ItemCount`] for no items or more than 200, and
/// [`DropRefusal::NotPinnable`] for anything else (`receiveFileDrop`).
fn pinnable_uris(uris: &[String]) -> Result<Vec<String>, DropRefusal> {
    if !(1..=MAX_DROPPED_PINS).contains(&uris.len()) {
        return Err(DropRefusal::ItemCount);
    }
    let mut pins: Vec<String> = Vec::with_capacity(uris.len());
    for uri in uris {
        let pin = normalise(uri).map_err(|_| DropRefusal::NotPinnable)?;
        if !is_draggable_location(&pin) {
            return Err(DropRefusal::NotPinnable);
        }
        if !pins.contains(&pin) {
            pins.push(pin);
        }
    }
    Ok(pins)
}

/// The pin to save for the dropped `uri`: a place Quick access shows
/// already keeps its label, and may be reordered while its share is away
/// (`verify_batch`); anything else is pinned only once GIO confirms a
/// folder or share. Runs GIO synchronously; call it on a worker.
fn verified_pin(uri: &str, shown: &[Place]) -> Result<BookmarkRequest, EntryError> {
    if let Some(place) = shown.iter().find(|place| place.uri == uri) {
        return Ok(BookmarkRequest::new(place.uri.clone(), place.label.clone()));
    }
    let target = verify_pin(uri, None, None::<&gio::Cancellable>)?;
    Ok(BookmarkRequest::new(target.uri, target.label))
}

/// The toast after `count` folders were pinned.
fn pinned_message(count: usize) -> String {
    if count == 1 {
        "Pinned to Quick access. No files were moved.".to_owned()
    } else {
        format!("{count} folders pinned. No files were moved.")
    }
}

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
            Err(EntryError::NotPinnable) => self.show_message(FOLDERS_ONLY),
            Err(error) => self.show_message(&error.to_string()),
        }
    }

    /// Starts a pin request; false while another one runs.
    pub(super) fn start_pinning(&self) -> bool {
        !self.imp().pinning.replace(true)
    }

    /// Ends the pin request.
    pub(super) fn end_pinning(&self) {
        self.imp().pinning.set(false);
    }

    /// Ends the pin request and shows `message`.
    fn finish_pinning(&self, message: &str) {
        self.end_pinning();
        self.show_message(message);
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
        if !self.start_pinning() {
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
                    window.finish_pinning(&message);
                }
            ),
        );
    }

    /// Pins the folders at `uris`, dropped on Quick access, before the pin
    /// at `before`, or at the end (`pinEntries`).
    ///
    /// # Errors
    ///
    /// As [`pinnable_uris`]; what GIO or the settings file refuse later
    /// shows in the toast.
    pub(super) fn pin_dropped(&self, uris: &[String], before: Option<String>) -> Result<(), DropRefusal> {
        let uris = pinnable_uris(uris)?;
        if !self.start_pinning() {
            return Ok(());
        }
        let shown = self.places().quick_access;
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.save_dropped_pins(uris, before, shown).await;
            }
        ));
        Ok(())
    }

    /// Verifies the dropped `uris` off the main thread, then saves them as
    /// pins before `before` in the Quick access order `shown`.
    async fn save_dropped_pins(&self, uris: Vec<String>, before: Option<String>, shown: Vec<Place>) {
        let shown_order: Vec<String> = shown.iter().map(|place| place.uri.clone()).collect();
        let verifying = gio::spawn_blocking(move || {
            uris.iter()
                .map(|uri| verified_pin(uri, &shown))
                .collect::<Result<Vec<_>, _>>()
        });
        let requests = match verifying.await {
            Ok(Ok(requests)) => requests,
            Ok(Err(error)) => {
                self.finish_pinning(&format!("Could not pin: {error}"));
                return;
            }
            Err(_panic) => {
                self.finish_pinning("Could not pin: the folders could not be checked.");
                return;
            }
        };
        let count = requests.len();
        let change: Change = Box::new(move |settings| {
            settings
                .pin_many(&requests, before.as_deref(), Some(&shown_order))
                .map(|_pins| ())
        });
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    let message = match result {
                        Ok(()) => pinned_message(count),
                        Err(error) => format!("Could not pin: {error}"),
                    };
                    window.finish_pinning(&message);
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
