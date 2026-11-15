// SPDX-License-Identifier: AGPL-3.0-only
//! What the window knows about the desktop: mounted volumes, device names,
//! pins and network locations, drawn into the sidebar and landing pages.
//!
//! Ports `refreshEnvironment` in `v2.0.0:desktop/ui/app.js` and `environment` in
//! `v2.0.0:desktop/winspace.py`. The volume monitor's changes to mounts, volumes
//! and drives (DEV-002) and the application's `places-changed` signal (a
//! pin, a saved share, a visited server, a kernel SMB mount or the
//! settings file changed) redraw the sidebar, the landing page, the icons
//! of network locations in the tabs and the details pane, and every label
//! that names a device. Pinning a folder is in [`super::quick_access`] and
//! mounting a volume in [`super::mounting`].

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::places::{NetworkLocation, Place};

use crate::locations::{self, Page};
use crate::places::{self, PlaceSources, Places};
use crate::volumes;

use super::landing;
use super::session::PaneSide;
use super::sidebar;
use super::BrowserWindow;

/// The volume monitor's signals that a drive, volume or mount appeared,
/// went away or changed (`mounts` events in winspace.py). Each redraws
/// every window (DEV-002).
pub(super) const VOLUME_MONITOR_SIGNALS: [&str; 9] = [
    "mount-added",
    "mount-removed",
    "mount-changed",
    "volume-added",
    "volume-removed",
    "volume-changed",
    "drive-connected",
    "drive-disconnected",
    "drive-changed",
];

impl BrowserWindow {
    /// Draws the sidebar, then redraws it whenever the volumes or the
    /// places change.
    pub(super) fn watch_environment(&self) {
        self.read_volumes();
        self.render_places();
        self.context().refresh_stable_mounts();
        let monitor = self.volume_monitor();
        let handlers = VOLUME_MONITOR_SIGNALS.map(|signal| {
            // The handler reads the monitor again rather than the object
            // the signal names, so every signal takes the same handler.
            monitor.connect_local(
                signal,
                false,
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    #[upgrade_or_default]
                    move |_| {
                        window.volumes_changed();
                        None
                    }
                ),
            )
        });
        // A mount that went away also takes the folders the tabs show on
        // it: they say so rather than keep their old rows.
        let removed = monitor.connect_local(
            "mount-removed",
            false,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or_default]
                move |arguments| {
                    let mount = arguments.get(1).and_then(|value| value.get::<gio::Mount>().ok());
                    if let Some(mount) = mount {
                        window.mount_removed(&mount.root().uri());
                    }
                    None
                }
            ),
        );
        let places = self.context().connect_places_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.places_changed()
        ));
        let mut external = self.imp().handlers.borrow_mut();
        external.volumes.extend(handlers);
        external.volumes.push(removed);
        external.places = Some(places);
    }

    /// Reads the volume monitor and rebuilds the device names.
    fn read_volumes(&self) {
        let rows = volumes::from_monitor(self.volume_monitor());
        let mut context = locations::location_context(glib::home_dir(), &rows);
        // Snapshot folders are read-only and mark the tabs inside them.
        context.snapshot_roots = self.context().previous_versions().snapshot_roots();
        context.network_mounts = self.network_mount_points();
        self.imp().volumes.replace(rows);
        self.imp().locations.replace(context);
    }

    /// The places changed: the kernel's SMB mounts may have too, so the
    /// locations under them count as network locations before everything
    /// that shows a place is redrawn.
    fn places_changed(&self) {
        self.imp().locations.borrow_mut().network_mounts = self.network_mount_points();
        self.render_places();
        // The settings may have changed too.
        self.apply_view_options();
        // Settings may have changed how items are shown.
        self.follow_item_preferences();
        if self.current_uri().as_deref() == Some(ox_core::location::RECENT_LOCATIONS_URI) {
            self.refresh();
        }
    }

    /// The mount points of the kernel's CIFS and SMB3 mounts, as last read:
    /// a tab at or below one is a network location (NET-006,
    /// `networkLocation` in app.js).
    fn network_mount_points(&self) -> Vec<PathBuf> {
        let mounts = self.context().network().stable_mounts();
        mounts.into_iter().map(|mount| mount.path).collect()
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
        let searches = self.context().saved_searches();
        let mut entries = sidebar::sidebar_entries(
            &places,
            &searches,
            &self.imp().locations.borrow(),
            self.context().settings_data().preferences.home_quick_access,
        );
        // Windows 11's navigation pane has neither capacity bars nor
        // Recent files, Recent locations and the Recycle Bin; Settings'
        // "Show more items in the navigation pane" adds them (SIDE-034).
        let more_items = self
            .context()
            .settings_data()
            .preferences
            .more_navigation_pane_items;
        if more_items {
            // Recent files hides while the desktop remembers no history
            // (SAFE-022), as in Nautilus.
            let remembers = self.context().recent_policy().remember;
            let fixed = sidebar::recent_and_bin_entries(self.imp().trash_items.get());
            entries.extend(fixed.into_iter().filter(|entry| {
                remembers
                    || !matches!(
                        entry.menu,
                        Some(
                            super::place_menus::PlaceMenu::RecentFiles
                                | super::place_menus::PlaceMenu::RecentLocations
                        )
                    )
            }));
        }
        let (rows, anything_hidden) = self.shown_sidebar_rows(entries);
        self.sidebar().set_capacity_bars(more_items);
        self.sidebar().set_rows(rows, anything_hidden);
        if let Some(uri) = self.current_uri() {
            self.sidebar().select(&uri);
        }
        self.render_landing_with(&places);
        self.follow_gallery_collection();
        self.follow_full_path_preference();
        self.follow_compact_density_preference();
        self.follow_item_check_boxes_preference();
        self.follow_expand_arrows_preference();
        self.render_title();
        self.render_tabs();
        self.update_details_pane();
        self.update_index_candidates(&places.quick_access);
        self.remember_more_menu_places(&places);
        self.update_more_menu();
    }

    /// Redraws the landing page of each pane on screen that shows one.
    pub(super) fn render_landing(&self) {
        self.render_landing_with(&self.places());
    }

    fn render_landing_with(&self, places: &Places) {
        for (side, uri) in self.shown_panes() {
            self.render_landing_in(side, &uri, places);
        }
    }

    /// Draws the landing page at `uri` in the folder pane on `side`, when
    /// `uri` is one.
    pub(super) fn render_landing_in(&self, side: PaneSide, uri: &str, places: &Places) {
        let Some(page) = Page::drawn_at(uri) else {
            return;
        };
        let body = self.pane_on(side).landing();
        if page == Page::Home && !self.context().settings_data().preferences.home_quick_access {
            // "Open Quick access on Home" was turned off while the pane
            // showed Home: it goes to the home folder (NAV-038), which
            // loading a tab on the Home page does while the setting is off.
            let id = {
                let session = self.imp().session.borrow();
                let active = session.active();
                active
                    .into_iter()
                    .chain(active.and_then(super::session::Tab::beside))
                    .find(|tab| tab.side == side)
                    .map(|tab| tab.id)
            };
            if let Some(id) = id {
                glib::idle_add_local_once(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.load_tab(id, super::loading::LoadMode::Navigate)
                ));
            }
            return;
        }
        if page == Page::Home {
            body.remove_css_class("this-pc");
            let preferences = self.context().settings_data().preferences;
            let locations = self.imp().locations.borrow();
            super::home_page::render(body, places, &preferences, &locations);
            return;
        }
        super::home_page::leave(body);
        let locations = self.imp().locations.borrow();
        landing::render(body, page, places, &locations, self.skin().drawn_text_size());
    }
}

#[cfg(test)]
mod tests {
    use ox_core::settings::{BookmarkRequest, Settings};

    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    /// A test cannot plug in a drive, so the handler every volume monitor
    /// signal is connected to is called as the monitor calls it: the window
    /// reads the volumes and the settings again and redraws the sidebar.
    ///
    /// parity: DEV-001, DEV-002
    #[gtk::test]
    fn a_volume_monitor_change_redraws_the_sidebar() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let pin = BookmarkRequest::new(fixture.uri_of("Documents"), "Pinned while plugging in");
        Settings::open(test.settings_directory())
            .pin_many(&[pin], None, None)
            .expect("the settings file takes a pin");

        test.window.volumes_changed();

        wait_until("the sidebar to be redrawn", || {
            let labels = test.window.sidebar().labels();
            labels.contains(&"Pinned while plugging in".to_owned())
        });
        assert!(test.window.sidebar().labels().contains(&"Local Disk".to_owned()));
    }
}
