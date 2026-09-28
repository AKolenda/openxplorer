// SPDX-License-Identifier: AGPL-3.0-only
//! Connecting a drive or device from the sidebar or This PC, and taking
//! one away: Disconnect, Eject and Safely remove.
//!
//! Ports `mountVolume` and `unmount` in `desktop/ui/app.js` and the
//! `mountVolume` and `unmount` operations of `desktop/winspace.py`, and
//! adds Dolphin's Eject and Safely remove ([`Removal`]). Each runs through
//! GTK's mount operation without blocking the window: the desktop's own
//! dialogs ask for an encrypted disk's password, show the programs that
//! keep a drive busy and say when an ejected medium's writes are flushed.
//!
//! Before a drive is taken away, the window stops listing and watching
//! the tabs on it, so its own folder monitors never keep the drive busy
//! (DEV-009); after, those tabs list again when next shown.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::network::{mount_volume, NetworkError, WriteActivity};

use crate::devices::Removal;
use crate::dialogs::NetworkFormDialog;
use crate::locations::Page;

use super::session::Tab;
use super::BrowserWindow;

/// The note of "Disconnect this mount?", under the location.
const DISCONNECT_NOTE: &str =
    "Close files using this mount first. This disconnects the session mount for other applications too.";

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
                Err(error) => window.show_failure("Could not mount device", &error.to_string()),
            }
        });
    }

    /// Takes away the drive or device that holds `uri` as `removal` says.
    /// Disconnect asks first, as This PC's card does; nothing starts while
    /// this window writes.
    pub(super) fn remove_drive(&self, uri: &str, removal: Removal) {
        if self.write_activity() == WriteActivity::Writing {
            self.show_message("Finish the current operation before disconnecting.");
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
        let dialog = NetworkFormDialog::new(self, "Disconnect this mount?", &message, "Disconnect");
        let uri = uri.to_owned();
        dialog.connect_confirmed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog| {
                dialog.finish();
                window.run_removal(&uri, Removal::Disconnect);
            }
        ));
        dialog.present();
    }

    /// Stops reading the tabs on the drive, then removes it.
    fn run_removal(&self, uri: &str, removal: Removal) {
        let label = self.drive_label(uri);
        self.stop_reading_inside(uri);
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
    /// Network, as app.js does; after Eject or Safely remove a tab on the
    /// drive shows This PC.
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

    /// Stops the listings and folder watches of the tabs inside `root`.
    fn stop_reading_inside(&self, root: &str) {
        let mut session = self.imp().session.borrow_mut();
        let inside = session
            .tabs_mut()
            .iter_mut()
            .filter(|tab| is_inside(tab.uri(), root));
        for tab in inside {
            tab.stop_reading();
        }
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
        let stale: Vec<gio::ListStore> = {
            let mut session = self.imp().session.borrow_mut();
            let tabs = session.tabs_mut().iter_mut();
            tabs.filter(|tab| is_stale(tab.uri()))
                .map(Tab::mark_stale)
                .collect()
        };
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
