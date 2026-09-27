// SPDX-License-Identifier: AGPL-3.0-only
//! What the window knows about the desktop: mounted volumes, device names,
//! pins and network locations, drawn into the sidebar and landing pages.
//!
//! Ports `refreshEnvironment` in `desktop/ui/app.js` and `environment` in
//! `desktop/winspace.py`. The volume monitor's changes and the
//! application's `places-changed` signal (a pin, a saved share, a visited
//! server or the settings file changed) redraw the sidebar, the landing
//! page and every label that names a device.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::pin_target;
use ox_core::location::same_location;
use ox_core::settings::{PinRequest, SettingsError};

use crate::locations::{self, Page};
use crate::places::{self, PlaceSources, Places};
use crate::settings_store::Change;
use crate::volumes::{self, VolumeFacts};

use super::landing::{self, Drawing};
use super::sidebar;
use super::BrowserWindow;

impl BrowserWindow {
    /// Draws the sidebar, then redraws it whenever the volumes or the
    /// places change.
    pub(super) fn watch_environment(&self) {
        self.read_volumes();
        self.render_places();
        let monitor = self.volume_monitor().clone();
        let redraw = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.volumes_changed()
        );
        let redraw = std::rc::Rc::new(redraw);
        let handlers = [
            monitor.connect_mount_added(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
            monitor.connect_mount_removed(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
            monitor.connect_mount_changed(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
            monitor.connect_volume_added(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
            monitor.connect_volume_removed(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
            monitor.connect_volume_changed(glib::clone!(
                #[strong]
                redraw,
                move |_, _| redraw()
            )),
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
        let context = locations::location_context(glib::home_dir(), &rows);
        self.imp().volumes.replace(rows);
        self.imp().locations.replace(context);
    }

    /// A device was plugged in, renamed or removed: every title, crumb and
    /// place may name it.
    fn volumes_changed(&self) {
        self.read_volumes();
        self.render_places();
        self.render_location();
        self.update_details_pane();
    }

    /// The sidebar and landing sections for the current settings and volumes.
    fn places(&self) -> Places {
        let settings = self.context().settings_data();
        let known_folders = ox_core::places::known_folders();
        let volumes = self.imp().volumes.borrow();
        let visited_network = self.context().visited_network();
        places::compose(PlaceSources {
            settings: &settings,
            known_folders: &known_folders,
            volumes: &volumes,
            stable_mounts: &[],
            visited_network: &visited_network,
        })
    }

    fn drawing(&self) -> Drawing {
        Drawing {
            appearance: self.skin().appearance(),
            scale: self.scale_factor(),
        }
    }

    /// Redraws the sidebar and the landing page.
    pub(super) fn render_places(&self) {
        let places = self.places();
        let entries = sidebar::sidebar_entries(&places, &self.imp().locations.borrow());
        let drawing = self.drawing();
        self.sidebar().show(entries, drawing.appearance, drawing.scale);
        if let Some(uri) = self.current_uri() {
            self.sidebar().select(&uri);
        }
        self.render_landing_with(&places);
    }

    /// Redraws the landing page when the active tab shows one.
    pub(super) fn render_landing(&self) {
        self.render_landing_with(&self.places());
    }

    fn render_landing_with(&self, places: &Places) {
        let Some(page) = self.current_uri().as_deref().and_then(Page::from_uri) else {
            return;
        };
        let locations = self.imp().locations.borrow();
        landing::render(&self.content().landing, page, places, &locations, self.drawing());
    }

    /// Pins the one selected folder to Quick access (`pinEntry`).
    pub(super) fn pin_selected(&self) {
        let items = self.content().model.selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        let entry = item.entry();
        match pin_target(entry, &entry.name) {
            Ok(target) => self.pin(target.uri, target.label),
            Err(error) => self.chrome().show_message(&error.to_string()),
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
            self.chrome().show_message("Already pinned to Quick access.");
            return;
        }
        let change: Change = Box::new(move |settings| {
            let request = PinRequest::new(uri, label);
            settings.pin_many(&[request], None, None).map(|_| ())
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
                    window.chrome().show_message(&message);
                }
            ),
        );
    }

    /// Mounts the volume `id` (asking for a password through the desktop's
    /// dialog if needed), then opens it (`mountVolume` in app.js).
    pub(super) fn mount_volume(&self, id: &str) {
        let volume = self
            .volume_monitor()
            .volumes()
            .into_iter()
            .find(|volume| VolumeFacts::from_volume(volume).id() == id);
        let Some(volume) = volume else {
            self.chrome().show_message("This device is no longer connected.");
            return;
        };
        let operation = gtk::MountOperation::new(Some(self));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let mounted = volume
                    .mount_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await;
                match (mounted, volume.get_mount()) {
                    (Err(error), _) => window.chrome().show_message(error.message()),
                    (Ok(()), Some(mount)) => window.navigate_or_report(&mount.root().uri()),
                    (Ok(()), None) => {}
                }
            }
        ));
    }
}
