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
    /// Starts at the default folders of `locations`, then reads their
    /// `user-dirs.dirs` and watches it for changes.
    pub(super) fn watch_known_folders(&self, locations: FolderLocations) {
        let defaults = locations.default_paths().quick_access_places();
        self.imp().known_folders.replace(defaults);
        self.monitor_user_dirs(&locations);
        self.read_known_folders(locations);
    }

    /// Reads the standard folders again whenever `user-dirs.dirs` is
    /// written, replaced or removed. A local file monitor never blocks.
    fn monitor_user_dirs(&self, locations: &FolderLocations) {
        let watched = locations.clone();
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
                    context.read_known_folders(watched.clone());
                }
            }
        ));
        self.imp().user_dirs_monitor.replace(Some(monitor));
    }

    /// Reads `locations` on a worker thread, then keeps the result and tells
    /// every window. When the file changes again while it is read, only the
    /// newest reading is kept, whichever finishes last.
    fn read_known_folders(&self, locations: FolderLocations) {
        let reading_number = self.imp().latest_folder_reading.get() + 1;
        self.imp().latest_folder_reading.set(reading_number);
        let context = self.downgrade();
        glib::spawn_future_local(async move {
            let reading = gio::spawn_blocking(move || locations.read_paths().quick_access_places());
            let Ok(places) = reading.await else {
                return;
            };
            let Some(context) = context.upgrade() else {
                return;
            };
            if context.imp().latest_folder_reading.get() != reading_number {
                return;
            }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::path::Path;
    use std::rc::Rc;

    use ox_core::location::file_uri;
    use ox_core::places::KnownFolder;
    use ox_core::settings::Settings;

    use super::*;
    use crate::test_support::harness::{skin, wait_until};

    /// Where `context` shows the Downloads folder.
    fn downloads_uri(context: &AppContext) -> Option<String> {
        let places = context.known_folders();
        let downloads = places
            .into_iter()
            .find(|place| place.known_folder == Some(KnownFolder::Downloads));
        downloads.map(|place| place.uri)
    }

    /// The URI of `name` in `home`.
    fn uri_in(home: &Path, name: &str) -> String {
        file_uri(&home.join(name))
    }

    /// The Python app re-read `user-dirs.dirs` for every sidebar; the
    /// native app watches it, so a folder moved with
    /// `xdg-user-dirs-update` still shows without a restart, and every
    /// window hears `places-changed`.
    ///
    /// parity: SIDE-006
    #[gtk::test]
    fn a_standard_folder_moved_in_user_dirs_shows_without_a_restart() {
        let folder = tempfile::tempdir().expect("the test home has room for a temporary folder");
        let home = folder.path().join("home");
        let config = folder.path().join("config");
        fs::create_dir(&config).expect("the config folder is created");
        let settings = Settings::open(&folder.path().join("settings"));
        let locations = FolderLocations::new(home.clone(), &config);
        let context = AppContext::with_folder_locations(skin(), settings, locations);
        let announcements = Rc::new(Cell::new(0));
        let counter = Rc::clone(&announcements);
        context.connect_places_changed(move || counter.set(counter.get() + 1));
        assert_eq!(downloads_uri(&context), Some(uri_in(&home, "Downloads")));

        fs::write(
            config.join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME/Incoming\"\n",
        )
        .expect("user-dirs.dirs is written");

        wait_until("the moved Downloads folder", || {
            downloads_uri(&context) == Some(uri_in(&home, "Incoming"))
        });
        assert!(announcements.get() >= 1, "the windows were told");
    }
}
