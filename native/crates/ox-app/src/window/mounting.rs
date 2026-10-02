// SPDX-License-Identifier: AGPL-3.0-only
//! Connecting a drive or device from the sidebar or This PC, and taking
//! one away: Disconnect, Eject and Safely remove.
//!
//! Ports `mountVolume` and `unmount` in `v2.0.0:desktop/ui/app.js` and the
//! `mountVolume` and `unmount` operations of `v2.0.0:desktop/winspace.py`, and
//! adds Dolphin's Eject and Safely remove ([`Removal`]). Each runs through
//! GTK's mount operation without blocking the window: the desktop's own
//! dialogs ask for an encrypted disk's password, show the programs that
//! keep a drive busy and say when an ejected medium's writes are flushed.
//!
//! Before a drive is taken away, every tab showing a folder on it moves
//! to Home, as Dolphin does, so the window's own listings and folder
//! monitors never keep the drive busy (DEV-009).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::file_uri;
use ox_core::network::{mount_volume, NetworkError, WriteActivity};

use crate::devices::Removal;
use crate::locations::Page;
use crate::window::{ButtonStyle, Dialog};

use super::BrowserWindow;

/// The note of "Disconnect this mount?", under the location.
const DISCONNECT_NOTE: &str = crate::i18n::message_id(
    "Close files using this mount first. This disconnects the session mount for other applications too.",
);

/// True when `uri` is the mount root `root` or a location inside it, as
/// GIO compares files.
fn is_inside(uri: &str, root: &str) -> bool {
    let file = gio::File::for_uri(uri);
    let root = gio::File::for_uri(root);
    file.equal(&root) || file.has_prefix(&root)
}

impl BrowserWindow {
    /// Mounts the volume `id` (asking for a password through the
    /// desktop's dialog if needed), then opens it; a failure shows "Could
    /// not mount device" (`mountVolume` in app.js).
    pub(super) fn mount_volume(&self, id: &str) {
        let volumes = self.volume_monitor().volumes();
        let operation = gtk::MountOperation::new(Some(self));
        let id = id.to_owned();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let mounted = mount_volume(&volumes, &id, Some(operation.upcast_ref())).await;
            let Some(window) = window.upgrade() else {
                return;
            };
            match mounted {
                Ok(root) => window.navigate_or_report(&root),
                Err(error) if error.is_cancelled() => {}
                Err(error) => window.show_failure(
                    ox_core::i18n::gettext_static("Could not mount device"),
                    &error.to_string(),
                ),
            }
        });
    }

    /// Mounts the volume `id` for a drop onto its sidebar row (DEV-010)
    /// and returns its root; a failure shows "Could not mount device".
    pub(super) async fn mount_for_drop(&self, id: &str) -> Option<String> {
        #[cfg(test)]
        if let Some((_, root)) = self
            .imp()
            .test_volume
            .borrow()
            .as_ref()
            .filter(|(test_id, _)| test_id == id)
        {
            return Some(root.clone());
        }
        let volumes = self.volume_monitor().volumes();
        let operation = gtk::MountOperation::new(Some(self));
        match mount_volume(&volumes, id, Some(operation.upcast_ref())).await {
            Ok(root) => Some(root),
            Err(error) if error.is_cancelled() => None,
            Err(error) => {
                self.show_failure(
                    ox_core::i18n::gettext_static("Could not mount device"),
                    &error.to_string(),
                );
                None
            }
        }
    }

    /// Takes away the drive or device that holds `uri` as `removal` says.
    /// Disconnect asks first, as This PC's card does; nothing starts while
    /// this window writes.
    pub(super) fn remove_drive(&self, uri: &str, removal: Removal) {
        if self.write_activity() == WriteActivity::Writing {
            self.show_message(&ox_core::i18n::gettext(
                "Finish the current operation before disconnecting.",
            ));
            return;
        }
        match removal {
            Removal::Disconnect => self.confirm_disconnect(uri),
            Removal::Eject | Removal::SafelyRemove => self.run_removal(uri, removal),
        }
    }

    /// "Disconnect this mount?" with the location, then Disconnect.
    fn confirm_disconnect(&self, uri: &str) {
        let address = self.imp().locations.borrow().display_location(uri);
        let message = format!("{address}\n\n{DISCONNECT_NOTE}");
        let dialog = Dialog::new(self, &ox_core::i18n::gettext("Disconnect this mount?"), &message);
        dialog.add_cancel_button();
        dialog.add_button(&ox_core::i18n::gettext("Disconnect"), ButtonStyle::Accent);
        let uri = uri.to_owned();
        dialog.connect_confirmed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog| {
                dialog.finish();
                window.run_removal(&uri, Removal::Disconnect);
            }
        ));
        dialog.open();
    }

    /// Stops reading the tabs on the drive, then removes it.
    fn run_removal(&self, uri: &str, removal: Removal) {
        let label = self.drive_label(uri);
        self.move_tabs_home_from(uri);
        let mounts = self.volume_monitor().mounts();
        let operation = gtk::MountOperation::new(Some(self));
        let activity = self.write_activity();
        let uri = uri.to_owned();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let removed = removal
                .perform(&mounts, &uri, operation.upcast_ref(), activity)
                .await;
            if let Some(window) = window.upgrade() {
                window.finish_removal(&uri, &label, removal, removed);
            }
        });
    }

    /// Shows what the removal did. After Disconnect the window shows
    /// Network, as app.js does; after Eject or Safely remove a tab that
    /// went back onto the drive meanwhile shows This PC.
    fn finish_removal(&self, uri: &str, label: &str, removal: Removal, removed: Result<(), NetworkError>) {
        if let Err(error) = removed {
            if !Removal::is_reported_by_desktop(&error) {
                self.show_failure(removal.failure_title(), &error.to_string());
            }
            return;
        }
        let was_inside = self.current_uri().is_some_and(|current| is_inside(&current, uri));
        self.mark_stale_inside(uri);
        match removal {
            Removal::Disconnect => self.navigate_or_report(Page::Network.uri()),
            Removal::Eject | Removal::SafelyRemove if was_inside => {
                self.navigate_or_report(Page::ThisPc.uri());
            }
            Removal::Eject | Removal::SafelyRemove => {}
        }
        if let Some(message) = removal.done_message(label) {
            self.show_message(&message);
        }
    }

    /// The name of the drive that holds `uri`, for the message after it
    /// is removed.
    fn drive_label(&self, uri: &str) -> String {
        let volumes = self.imp().volumes.borrow();
        let holding = volumes
            .iter()
            .find(|row| row.uri().is_some_and(|root| is_inside(uri, root)));
        holding.map_or_else(|| uri.to_owned(), |row| row.label.clone())
    }

    /// Moves every tab inside `root` to Home: the tab in front navigates
    /// there, and a tab behind it stops reading the drive and lists Home
    /// when it is next shown.
    fn move_tabs_home_from(&self, root: &str) {
        let home = file_uri(&glib::home_dir());
        let (stale, is_active_inside): (Vec<gio::ListStore>, bool) = {
            let mut session = self.imp().session.borrow_mut();
            let active = session.active_id();
            let is_active_inside = session.active().is_some_and(|tab| is_inside(tab.uri(), root));
            let mut stale = Vec::new();
            session.change_panes(|tab| {
                if Some(tab.id) != active && is_inside(tab.uri(), root) {
                    tab.history.push(&home);
                    tab.forget_location_state();
                    stale.push(tab.mark_stale());
                }
            });
            (stale, is_active_inside)
        };
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
        if is_active_inside {
            self.navigate_or_report(&home);
        }
        self.render_tabs();
    }

    /// Makes every tab inside `root` list again when it is next shown.
    fn mark_stale_inside(&self, root: &str) {
        self.mark_tabs_stale(|uri| is_inside(uri, root));
    }

    /// Makes every tab whose location passes `is_stale` list again when it
    /// is next shown, and drops what those tabs listed. The items are
    /// dropped after the session is released, because dropping the active
    /// tab's items runs the view's handlers, which read the session.
    pub(super) fn mark_tabs_stale(&self, is_stale: impl Fn(&str) -> bool) {
        let mut stale: Vec<gio::ListStore> = Vec::new();
        self.imp().session.borrow_mut().change_panes(|tab| {
            if is_stale(tab.uri()) {
                stale.push(tab.mark_stale());
            }
        });
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_is_inside_its_mount_root_and_nowhere_else() {
        assert!(is_inside("file:///media/demo/USB", "file:///media/demo/USB"));
        assert!(is_inside(
            "file:///media/demo/USB/Photos",
            "file:///media/demo/USB"
        ));
        assert!(!is_inside("file:///media/demo/USB2", "file:///media/demo/USB"));
        assert!(!is_inside("file:///home/demo", "file:///media/demo/USB"));
    }
}
