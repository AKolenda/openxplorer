// SPDX-License-Identifier: AGPL-3.0-only
//! Adding a place for any location and editing a pin (SIDE-031, SIDE-011).
//!
//! Dolphin's Places panel offers "Add Entry…" on its empty space and
//! "Edit…" on a place: a dialog with the label and the location, which
//! may be any address the address bar takes (a folder path,
//! `smb://server/share/folder`, `sftp://…`), with a folder picker. The
//! location is checked as a pin is (SIDE-007: GIO must find a folder or a
//! share) before anything is saved, and the place joins Quick access
//! without the window going there. Editing keeps the pin where it is.
//! Places carry no icon of their own: Quick access draws each with its
//! folder's art, as Explorer's navigation pane does.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::verify_pin;
use ox_core::location::{normalise_navigation, same_location};
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, SettingsError};
use ox_core::transfer::Cancellation;

use crate::settings_store::Change;

use super::dialog::Dialog;
use super::BrowserWindow;
use super::ButtonStyle;

/// The add dialog's heading and note.
const ADD_TITLE: &str = "Add entry";
const ADD_MESSAGE: &str = "Add a folder or network location to Quick access.";
/// The edit dialog's heading and note.
const EDIT_TITLE: &str = "Edit entry";
const EDIT_MESSAGE: &str = "Change the name or the location of this Quick access entry.";

/// The pin being edited: where it is now and the place after it.
#[derive(Debug, Clone)]
struct EditedPin {
    uri: String,
    /// The pin after it in Quick access, which it stays before.
    next: Option<String>,
}

impl BrowserWindow {
    /// "Add entry…": asks for a label and a location, then pins it.
    pub(super) async fn add_place(&self) {
        self.edit_place(String::new(), String::new(), None).await;
    }

    /// "Edit…" on the pin of `uri`: asks for its new label and location.
    pub(super) async fn edit_pin(&self, uri: String) {
        let shown = self.places().quick_access;
        let Some(index) = shown.iter().position(|place| same_location(&place.uri, &uri)) else {
            return;
        };
        let label = shown[index].label.clone();
        let next = shown.get(index + 1).map(|place| place.uri.clone());
        let edited = EditedPin {
            uri: uri.clone(),
            next,
        };
        self.edit_place(label, uri, Some(edited)).await;
    }

    /// Asks for a label and a location starting from `label` and
    /// `location`, and saves them as a new pin or as `edited`.
    async fn edit_place(&self, label: String, location: String, edited: Option<EditedPin>) {
        let (title, message, answer) = match edited {
            Some(_) => (EDIT_TITLE, EDIT_MESSAGE, "Save"),
            None => (ADD_TITLE, ADD_MESSAGE, "Add"),
        };
        let dialog = Dialog::new(self, title, message);
        let label_field = dialog.add_text_field("Label", &label);
        label_field.set_placeholder_text(Some("The folder's name"));
        let shown_location = self.imp().locations.borrow().display_location(&location);
        let location_field = self.add_location_field(&dialog, &shown_location);
        dialog.add_cancel_button();
        dialog.add_button(answer, ButtonStyle::Accent);
        dialog.open();
        loop {
            if dialog.next_response().await.is_none() {
                return;
            }
            let request = self.place_request(&label_field.text(), &location_field.text());
            let request = match request {
                Ok(request) => request,
                Err(error) => {
                    dialog.show_error(&error);
                    continue;
                }
            };
            // One pin request at a time (SIDE-007), as pinning does.
            if !self.start_pinning() {
                dialog.show_error("Another folder is being pinned. Try again in a moment.");
                continue;
            }
            let running = Cancellation::new();
            dialog.set_busy(Some(&running));
            let outcome = self.save_place(request, edited.clone(), &running).await;
            self.end_pinning();
            dialog.set_busy(None);
            match outcome {
                Ok(()) => {
                    dialog.finish();
                    return;
                }
                // Closing the dialog cancelled it and nothing was saved.
                Err(_) if running.is_cancelled() => return,
                Err(error) => dialog.show_error(&error),
            }
        }
    }

    /// The Location field, with a Browse… button that picks a folder.
    fn add_location_field(&self, dialog: &Dialog, location: &str) -> gtk::Entry {
        let entry = gtk::Entry::builder()
            .text(location)
            .placeholder_text("For example ~/Projects or smb://server/share")
            .activates_default(true)
            .hexpand(true)
            .build();
        let browse = gtk::Button::with_label("Browse…");
        browse.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            entry,
            move |_| window.pick_folder_into(&entry)
        ));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&entry);
        row.append(&browse);
        dialog.add_labelled("Location", &row);
        entry.update_property(&[gtk::accessible::Property::Label("Location")]);
        entry
    }

    /// Lets the user choose a folder and writes its path into `entry`.
    fn pick_folder_into(&self, entry: &gtk::Entry) {
        let picker = gtk::FileDialog::builder()
            .title("Choose a folder")
            .modal(true)
            .build();
        picker.select_folder(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak]
                entry,
                move |chosen| {
                    if let Ok(folder) = chosen {
                        let text = folder.path().map_or_else(
                            || folder.uri().to_string(),
                            |path| path.to_string_lossy().into_owned(),
                        );
                        entry.set_text(&text);
                    }
                }
            ),
        );
    }

    /// The pin the fields describe: the location the address bar would
    /// open for `location`, labelled `label`; an error to show otherwise.
    fn place_request(&self, label: &str, location: &str) -> Result<BookmarkRequest, String> {
        let location = location.trim();
        if location.is_empty() {
            return Err("Enter a folder or network location.".to_owned());
        }
        let locations = self.imp().locations.borrow();
        let base = self.current_uri();
        let uri = normalise_navigation(location, base.as_deref(), &locations.home_path())
            .map_err(|error| error.to_string())?;
        Ok(BookmarkRequest::new(uri, label.trim()))
    }

    /// Checks `request` as a pin, off the main thread, then saves it as a
    /// new pin or in place of `edited`, unless `running` was cancelled
    /// meanwhile: closing the dialog stops the check and saves nothing.
    async fn save_place(
        &self,
        request: BookmarkRequest,
        edited: Option<EditedPin>,
        running: &Cancellation,
    ) -> Result<(), String> {
        let (uri, label) = (request.uri.clone(), request.label.clone());
        let cancellable = running.cancellable().clone();
        let verifying = gio::spawn_blocking(move || {
            let label = (!label.is_empty()).then_some(label.as_str());
            verify_pin(&uri, label, Some(&cancellable))
        });
        let verified = verifying.await;
        if running.is_cancelled() {
            return Err("Cancelled.".to_owned());
        }
        let target = match verified {
            Ok(Ok(target)) => target,
            Ok(Err(error)) => return Err(format!("Could not add: {error}")),
            Err(_panic) => return Err("Could not add: the location could not be checked.".to_owned()),
        };
        let shown: Vec<String> = self
            .places()
            .quick_access
            .iter()
            .map(|place| place.uri.clone())
            .collect();
        let pin = BookmarkRequest::new(target.uri, target.label);
        let change: Change = Box::new(move |settings| {
            let Some(edited) = edited else {
                return settings.pin_many(&[pin], None, Some(&shown)).map(|_| ());
            };
            let moved = !same_location(&edited.uri, &pin.uri);
            // The new pin goes where the old one is; only then does the old
            // one go, so a failed save loses neither.
            let before = if moved {
                Some(edited.uri.as_str())
            } else {
                edited.next.as_deref()
            };
            settings.pin_many(std::slice::from_ref(&pin), before, Some(&shown))?;
            if moved {
                let old = BookmarkRequest::new(edited.uri.clone(), String::new());
                settings.bookmark(BookmarkAction::Remove, BookmarkKind::Pin, &old)?;
            }
            Ok(())
        });
        let (sender, receiver) = async_channel::bounded(1);
        self.context()
            .change_settings(change, move |result: Result<(), SettingsError>| {
                let _ = sender.try_send(result);
            });
        match receiver.recv().await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(format!("Could not save: {error}")),
            Err(_) => Err("Could not save the entry.".to_owned()),
        }
    }
}
