// SPDX-License-Identifier: AGPL-3.0-only
//! Check location and Apply location, which run off the main thread, and
//! what follows a change: the offer to move the old folder's files, then
//! Brave's dialog when its box is ticked.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::folder_locations::{AppliedLocation, CheckedLocation, Consent, LocationChange, RelocationError};
use ox_core::transfer::TransferResult;

use super::{LocationPanel, Tone};
use crate::integration::BraveDialog;
use crate::window::BrowserWindow;

/// The status after a change, before any file moves.
const UPDATED: &str = crate::i18n::message_id("Location updated. Configuration backed up. No files moved.");
/// The status after every offered item moved.
const FILES_MOVED: &str =
    crate::i18n::message_id("Location updated. Configuration backed up. Files moved to the new location.");

impl LocationPanel {
    /// Check location: validates the field's destination and, if it still
    /// holds the same text afterwards, shows the real path.
    pub(super) fn check_location(&self) {
        let check_button = self.imp().check_button.get().expect("build adds Check location");
        check_button.set_sensitive(false);
        self.forget_check();
        let requested = self.field().text().to_string();
        self.set_status(
            &ox_core::i18n::gettext("Checking the folder and write access…"),
            Tone::Plain,
        );
        let folder = self.folder();
        let relocation = self.relocation();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[weak]
            check_button,
            async move {
                let value = requested.clone();
                let checked = gio::spawn_blocking(move || relocation.check(folder, &value)).await;
                check_button.set_sensitive(true);
                let Ok(checked) = checked else {
                    return;
                };
                panel.show_check(&requested, checked);
            }
        ));
    }

    fn show_check(&self, requested: &str, checked: Result<CheckedLocation, RelocationError>) {
        if self.field().text() != requested {
            self.set_status(
                &ox_core::i18n::gettext("The destination changed. Check it again."),
                Tone::Plain,
            );
            return;
        }
        match checked {
            Ok(checked) => {
                let kind = if checked.is_network {
                    "Mounted network folder"
                } else {
                    "Local folder"
                };
                let path = checked.path.to_string_lossy().into_owned();
                self.set_field_text(&path);
                self.set_status(&format!("{kind} · {path}"), Tone::Valid);
                self.imp().checked.replace(Some(checked));
            }
            Err(error) => self.set_status(&error.to_string(), Tone::Error),
        }
        self.update_apply();
    }

    /// Apply location: moves the folder to the checked destination.
    pub(super) fn apply_location(&self) {
        if !self.consent().is_active() {
            self.set_status(
                &ox_core::i18n::gettext("Confirm the change using the checkbox first."),
                Tone::Plain,
            );
            return;
        }
        if !self.is_field_checked() {
            self.set_status(
                &ox_core::i18n::gettext("Check the destination first."),
                Tone::Plain,
            );
            return;
        }
        self.imp().is_applying.set(true);
        self.update_apply();
        let value = self.field().text().to_string();
        let folder = self.folder();
        let relocation = self.relocation();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            async move {
                let applied =
                    gio::spawn_blocking(move || relocation.apply(folder, &value, Consent::Given)).await;
                panel.imp().is_applying.set(false);
                panel.update_apply();
                match applied {
                    Ok(Ok(applied)) => panel.show_applied(applied).await,
                    Ok(Err(error)) => panel.set_status(&error.to_string(), Tone::Error),
                    Err(_) => {}
                }
            }
        ));
    }

    /// Shows the new location and clears the consent, offers to move the
    /// old folder's files after a change, then opens Brave's dialog when
    /// its box is ticked.
    async fn show_applied(&self, applied: AppliedLocation) {
        let location = applied.location;
        if let Some(known) = self.imp().location.borrow_mut().as_mut() {
            known.previous_path = Some(location.previous.clone());
            known.path.clone_from(&location.path);
        }
        self.consent().set_active(false);
        self.imp().checked.replace(None);
        self.set_status(ox_core::i18n::gettext_static(UPDATED), Tone::Valid);
        let Some(window) = self.root().and_downcast::<BrowserWindow>() else {
            return;
        };
        if matches!(applied.change, LocationChange::Changed { .. }) {
            self.offer_to_move_files(&window, &location).await;
        }
        let sync_brave = self.imp().sync_brave.get().is_some_and(CheckButtonExt::is_active);
        if sync_brave {
            self.open_brave_dialog(&window, &location.path.to_string_lossy());
        }
    }

    /// Lists the items of the old folder that may follow it, off the main
    /// thread, and lets the window ask about moving them and move them.
    async fn offer_to_move_files(&self, window: &BrowserWindow, location: &CheckedLocation) {
        let relocation = self.relocation();
        let applied = location.clone();
        let Ok(items) = gio::spawn_blocking(move || relocation.contents_to_move(&applied)).await else {
            return;
        };
        let moved = window
            .offer_to_move_files(&location.previous, &location.path, items)
            .await;
        if let Some(moved) = moved {
            let status = moved_status(&moved.result, &location.previous.to_string_lossy());
            self.set_status(&status, Tone::Valid);
        }
    }

    /// Brave's download-folder dialog for `path`, which asks its own
    /// consent (PROP-018).
    fn open_brave_dialog(&self, window: &BrowserWindow, path: &str) {
        let brave = self
            .imp()
            .brave
            .get()
            .expect("new sets Brave's integration")
            .clone();
        let report = glib::clone!(
            #[weak]
            window,
            move |message: &str| window.show_message(message)
        );
        BraveDialog::present_for(window, brave, path, report);
    }
}

/// The status after moving the files of `previous`.
fn moved_status(result: &TransferResult, previous: &str) -> String {
    let is_complete = result.skipped.is_empty() && result.errors.is_empty() && !result.cancelled;
    if is_complete {
        ox_core::i18n::gettext_static(FILES_MOVED).to_owned()
    } else {
        ox_core::i18n::format_message(
            "Location updated. Configuration backed up. Some items stayed in {previous}.",
            &[("previous", &(previous).to_string())],
        )
    }
}
