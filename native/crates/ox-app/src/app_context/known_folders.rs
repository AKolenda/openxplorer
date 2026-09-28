// SPDX-License-Identifier: AGPL-3.0-only
//! The standard folders Quick access shows (Desktop, Downloads, ...), kept
//! current without reading a file on the main thread.
//!
//! Ports the known-folder part of `environment()` in `desktop/winspace.py`,
//! which read `user-dirs.dirs` on every call so that a folder moved with
//! `xdg-user-dirs-update` showed at once. Here the file is read off the
//! main thread when the application starts and again whenever it changes;
//! windows draw Quick access from the last reading and hear
//! `places-changed` after each one.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::places::{FolderLocations, Place};
use ox_core::LOG_DOMAIN;

use super::AppContext;

/// Whether `event` may leave `user-dirs.dirs` with other contents: a write
/// that finished, or the file created, removed or renamed into place.
fn may_change_the_file(event: gio::FileMonitorEvent) -> bool {
    matches!(
        event,
        gio::FileMonitorEvent::ChangesDoneHint
            | gio::FileMonitorEvent::Created
            | gio::FileMonitorEvent::Deleted
            | gio::FileMonitorEvent::MovedIn
            | gio::FileMonitorEvent::MovedOut
            | gio::FileMonitorEvent::Renamed
    )
}

impl AppContext {
    /// Starts at the default folders, then reads `user-dirs.dirs` and
    /// watches it for changes.
    pub(super) fn watch_known_folders(&self) {
        let locations = FolderLocations::from_environment();
        let defaults = locations.default_paths().quick_access_places();
        self.imp().known_folders.replace(defaults);
        self.monitor_user_dirs(&locations);
        self.read_known_folders(locations);
    }

    /// Reads the standard folders again whenever `user-dirs.dirs` is
    /// written, replaced or removed. A local file monitor never blocks.
    fn monitor_user_dirs(&self, locations: &FolderLocations) {
        let file = gio::File::for_path(locations.user_dirs_file());
        let monitor = match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(monitor) => monitor,
            Err(error) => {
                let path = locations.user_dirs_file().display();
                glib::g_warning!(LOG_DOMAIN, "Could not watch {path} for changes: {error}");
                return;
            }
        };
        monitor.connect_changed(glib::clone!(
            #[weak(rename_to = context)]
            self,
            move |_, _, _, event| {
                if may_change_the_file(event) {
                    context.read_known_folders(FolderLocations::from_environment());
                }
            }
        ));
        self.imp().user_dirs_monitor.replace(Some(monitor));
    }

    /// Reads `locations` on a worker thread, then keeps the result and tells
    /// every window.
    fn read_known_folders(&self, locations: FolderLocations) {
        let context = self.downgrade();
        glib::spawn_future_local(async move {
            let reading = gio::spawn_blocking(move || locations.read_paths().quick_access_places());
            let Ok(places) = reading.await else {
                return;
            };
            let Some(context) = context.upgrade() else {
                return;
            };
            let changed = *context.imp().known_folders.borrow() != places;
            context.imp().known_folders.replace(places);
            if changed {
                context.notify_places_changed();
            }
        });
    }

    /// The Quick access rows of the standard folders, as last read.
    pub(crate) fn known_folders(&self) -> Vec<Place> {
        self.imp().known_folders.borrow().clone()
    }
}
