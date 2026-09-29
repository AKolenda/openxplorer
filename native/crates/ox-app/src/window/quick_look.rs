// SPDX-License-Identifier: AGPL-3.0-only
//! Quick Look with Space through the GNOME previewer (PROP-012), as Files
//! does it: Space on one selected file asks GNOME Sushi
//! (`org.gnome.NautilusPreviewer2.ShowFile`) to show it, moving the
//! selection with the arrow keys shows the new file, and Space or Escape
//! closes it. Without Sushi installed, Space keeps toggling the selection;
//! a typed prefix keeps its space (see `input`).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::BrowserWindow;

/// Sushi's bus name, and the object and interface of its previewer.
pub(super) const PREVIEWER_NAME: &str = "org.gnome.NautilusPreviewer";
pub(super) const PREVIEWER_PATH: &str = "/org/gnome/NautilusPreviewer";
pub(super) const PREVIEWER_INTERFACE: &str = "org.gnome.NautilusPreviewer2";
/// How long a call to the previewer may take.
const CALL_TIMEOUT_MS: i32 = 5000;

impl BrowserWindow {
    /// Space without a typed prefix: closes an open preview, or previews
    /// the one selected file. False when there is nothing to do here, so
    /// Space goes on to the view.
    pub(super) fn toggle_quick_look(&self) -> bool {
        if self.imp().quick_look_open.get() {
            self.close_quick_look();
            return true;
        }
        let Some(uri) = self.quick_look_target() else {
            return false;
        };
        let Some(bus) = previewer_bus() else {
            return false;
        };
        self.imp().quick_look_open.set(true);
        show_file(&bus, &uri, false);
        true
    }

    /// Escape: closes an open preview; false when none is open.
    pub(super) fn close_quick_look(&self) -> bool {
        if !self.imp().quick_look_open.replace(false) {
            return false;
        }
        if let Some(bus) = previewer_bus() {
            call(&bus, "Close", None);
        }
        true
    }

    /// Shows the newly selected file in an open preview, as the arrow
    /// keys move the selection.
    pub(super) fn follow_quick_look(&self) {
        if !self.imp().quick_look_open.get() {
            return;
        }
        if let (Some(uri), Some(bus)) = (self.quick_look_target(), previewer_bus()) {
            show_file(&bus, &uri, false);
        }
    }

    /// The one selected file, if exactly one file is selected.
    fn quick_look_target(&self) -> Option<String> {
        let selected = self.folder_pane().model().selected_items();
        let [item] = selected.as_slice() else {
            return None;
        };
        let entry = item.entry();
        (!entry.is_dir && !entry.is_virtual).then(|| entry.uri.clone())
    }
}

/// The session bus, when Sushi runs or can be started on it.
fn previewer_bus() -> Option<gio::DBusConnection> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let ask = |method: &str, arguments: Option<&glib::Variant>| {
        bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            arguments,
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            gio::Cancellable::NONE,
        )
        .ok()
    };
    let has_owner = ask("NameHasOwner", Some(&(PREVIEWER_NAME,).to_variant()))
        .and_then(|reply| reply.get::<(bool,)>())
        .is_some_and(|(owned,)| owned);
    let activatable = || {
        ask("ListActivatableNames", None)
            .and_then(|reply| reply.get::<(Vec<String>,)>())
            .is_some_and(|(names,)| names.iter().any(|name| name == PREVIEWER_NAME))
    };
    (has_owner || activatable()).then_some(bus)
}

/// `ShowFile(uri, parent handle, close if shown)`. The window's handle
/// needs the X11 or Wayland GDK bindings, which the app does not link, so
/// the preview opens as its own window.
fn show_file(bus: &gio::DBusConnection, uri: &str, close_if_shown: bool) {
    call(bus, "ShowFile", Some(&(uri, "", close_if_shown).to_variant()));
}

/// Calls `method` on the previewer without waiting for it.
fn call(bus: &gio::DBusConnection, method: &str, arguments: Option<&glib::Variant>) {
    bus.call(
        Some(PREVIEWER_NAME),
        PREVIEWER_PATH,
        PREVIEWER_INTERFACE,
        method,
        arguments,
        None,
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
        gio::Cancellable::NONE,
        |_| {},
    );
}
