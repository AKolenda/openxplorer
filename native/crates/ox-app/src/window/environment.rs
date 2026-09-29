// SPDX-License-Identifier: AGPL-3.0-only
//! What the window knows about the desktop: mounted volumes, device names,
//! pins and network locations, drawn into the sidebar and landing pages.
//!
//! Ports `refreshEnvironment` in `desktop/ui/app.js` and `environment` in
//! `desktop/winspace.py`. The volume monitor's changes to mounts, volumes
//! and drives (DEV-002) and the application's `places-changed` signal (a
//! pin, a saved share, a visited server, a kernel SMB mount or the
//! settings file changed) redraw the sidebar, the landing page, the icons
//! of network locations in the tabs and the details pane, and every label
//! that names a device. Pinning a folder is in [`super::quick_access`] and
//! mounting a volume in [`super::mounting`].

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::places::{NetworkLocation, Place};

use crate::locations::{self, Page};
use crate::places::{self, PlaceSources, Places};
use crate::volumes;

use super::landing;
use super::sidebar;
use super::BrowserWindow;

/// A volume monitor handler that tells `window` about any mount or volume
/// change; it holds the window weakly, so it never keeps a closed window.
fn redraw_on_change<Changed>(
    window: &glib::WeakRef<BrowserWindow>,
) -> impl Fn(&gio::VolumeMonitor, &Changed) + 'static {
    let window = window.clone();
    move |_, _| {
        if let Some(window) = window.upgrade() {
            window.volumes_changed();
        }
    }
}

impl BrowserWindow {
    /// Draws the sidebar, then redraws it whenever the volumes or the
    /// places change.
    pub(super) fn watch_environment(&self) {
        self.read_volumes();
        self.render_places();
        self.context().refresh_stable_mounts();
        let monitor = self.volume_monitor();
        let window = self.downgrade();
        let handlers = [
            monitor.connect_mount_added(redraw_on_change(&window)),
            monitor.connect_mount_removed(redraw_on_change(&window)),
            monitor.connect_mount_changed(redraw_on_change(&window)),
            monitor.connect_volume_added(redraw_on_change(&window)),
            monitor.connect_volume_removed(redraw_on_change(&window)),
            monitor.connect_volume_changed(redraw_on_change(&window)),
            monitor.connect_drive_connected(redraw_on_change(&window)),
            monitor.connect_drive_disconnected(redraw_on_change(&window)),
            monitor.connect_drive_changed(redraw_on_change(&window)),
        ];
        let places = self.context().connect_places_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.render_places()
        ));
        let mut external = self.imp().handlers.borrow_mut();
        external.volumes.extend(handlers);
        external.places = Some(places);
    }

    /// Reads the volume monitor and rebuilds the device names.
    fn read_volumes(&self) {
        let rows = volumes::from_monitor(self.volume_monitor());
        let mut context = locations::location_context(glib::home_dir(), &rows);
        // Snapshot folders are read-only and mark the tabs inside them.
        context.snapshot_roots = self.context().previous_versions().snapshot_roots();
        self.imp().volumes.replace(rows);
        self.imp().locations.replace(context);
    }

    /// A device was plugged in, renamed or removed: every title, crumb and
    /// place may name it. The settings are read again too, as the Python
    /// app's `environment()` does on every change, so pins the Python app
    /// saved meanwhile appear.
    fn volumes_changed(&self) {
        self.read_volumes();
        self.render_places();
        self.render_location();
        self.context().reload_settings();
        self.context().refresh_stable_mounts();
    }

    /// The sidebar and landing sections for the current settings, volumes
    /// and standard folders. The application reads `user-dirs.dirs` off the
    /// main thread whenever it changes, so nothing is read here.
    pub(super) fn places(&self) -> Places {
        self.places_with(&self.context().known_folders())
    }

    /// The Network list alone, for the icons of network locations in the
    /// tabs and the details pane; only Quick access needs the known
    /// folders.
    pub(super) fn network_locations(&self) -> Vec<NetworkLocation> {
        self.places_with(&[]).network
    }

    /// The sections for the current settings, volumes and visited servers,
    /// with `known_folders` in Quick access.
    fn places_with(&self, known_folders: &[Place]) -> Places {
        let settings = self.context().settings_data();
        let volumes = self.imp().volumes.borrow();
        let stable_mounts = self.context().network().stable_mounts();
        let visited_network = self.context().visited_network();
        places::compose(PlaceSources {
            settings: &settings,
            known_folders,
            volumes: &volumes,
            stable_mounts: &stable_mounts,
            visited_network: &visited_network,
        })
    }

    /// Redraws everything that shows a place: the sidebar, the landing
    /// page, the tabs and the details pane, whose network locations show
    /// the art of their sidebar rows, and the folders Settings offers the
    /// search index.
    pub(super) fn render_places(&self) {
        let places = self.places();
        let entries = sidebar::sidebar_entries(&places, &self.imp().locations.borrow());
        self.sidebar().set_entries(entries);
        if let Some(uri) = self.current_uri() {
            self.sidebar().select(&uri);
        }
        self.render_landing_with(&places);
        self.follow_full_path_preference();
        self.render_tabs();
        self.update_details_pane();
        self.update_index_candidates(&places.quick_access);
    }

    /// Redraws the landing page when the active tab shows one.
    pub(super) fn render_landing(&self) {
        self.render_landing_with(&self.places());
    }

    fn render_landing_with(&self, places: &Places) {
        let Some(page) = self.current_uri().as_deref().and_then(Page::from_uri) else {
            return;
        };
        let body = self.folder_pane().landing();
        let locations = self.imp().locations.borrow();
        let discovery = self.network().discovery().state();
        landing::render(body, page, places, &locations, &discovery);
    }
}
