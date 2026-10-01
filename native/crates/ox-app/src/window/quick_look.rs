// SPDX-License-Identifier: AGPL-3.0-only
//! Quick Look with Space through the GNOME previewer (PROP-012), as Files
//! does it: Space on one selected file asks GNOME Sushi
//! (`org.gnome.NautilusPreviewer2.ShowFile`) to show it, moving the
//! selection shows the new file, the arrow keys pressed in Sushi's window
//! move the selection (`SelectionEvent`), and Space or Escape closes it.
//! When Sushi closes itself, its `Visible` property says so and the window
//! stops following the selection. Without Sushi installed, Space keeps
//! toggling the selection; a typed prefix keeps its space (see `input`).
//!
//! The window follows Sushi through a [`gio::DBusProxy`] made when it is
//! built, so a key press never waits for the bus.

use std::cell::{Cell, RefCell};

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
/// `GtkDirectionType` values of `SelectionEvent` that go back.
const DIRECTION_UP: u32 = 2;
const DIRECTION_LEFT: u32 = 4;

/// A window's link to the previewer.
#[derive(Debug, Default)]
pub(super) struct QuickLook {
    /// The previewer, once the proxy is made.
    proxy: RefCell<Option<gio::DBusProxy>>,
    /// Sushi runs, or the bus can start it.
    is_available: Cell<bool>,
    /// Sushi shows a file this window asked it to.
    is_open: Cell<bool>,
    /// Sushi said its window is shown since this window asked it to show
    /// a file; only then does a hidden window mean it was closed, not
    /// that it is still starting.
    was_visible: Cell<bool>,
}

impl BrowserWindow {
    /// Makes the proxy of the previewer and follows it: whether it can be
    /// used, whether its window is still shown, and the arrow keys
    /// pressed in it. Called once, as the window is built.
    pub(super) fn watch_quick_look(&self) {
        let window = self.downgrade();
        gio::DBusProxy::for_bus(
            gio::BusType::Session,
            gio::DBusProxyFlags::DO_NOT_AUTO_START_AT_CONSTRUCTION,
            None,
            PREVIEWER_NAME,
            PREVIEWER_PATH,
            PREVIEWER_INTERFACE,
            gio::Cancellable::NONE,
            move |proxy| {
                if let (Some(window), Ok(proxy)) = (window.upgrade(), proxy) {
                    window.follow_previewer(&proxy);
                }
            },
        );
    }

    fn follow_previewer(&self, proxy: &gio::DBusProxy) {
        let quick_look = &self.imp().quick_look;
        quick_look.is_available.set(proxy.name_owner().is_some());
        let window = self.downgrade();
        proxy.connect_notify_local(Some("g-name-owner"), move |proxy, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let quick_look = &window.imp().quick_look;
            if proxy.name_owner().is_some() {
                quick_look.is_available.set(true);
            } else {
                // Sushi quit; it may still be started again on demand.
                quick_look.is_open.set(false);
            }
        });
        let window = self.downgrade();
        proxy.connect_local("g-properties-changed", false, move |_| {
            let window = window.upgrade()?;
            window.previewer_visibility_changed();
            None
        });
        let window = self.downgrade();
        proxy.connect_local("g-signal", false, move |values| {
            let window = window.upgrade()?;
            let signal = values.get(2)?.get::<String>().ok()?;
            let parameters = values.get(3)?.get::<glib::Variant>().ok()?;
            if signal == "SelectionEvent" {
                if let Some((direction,)) = parameters.get::<(u32,)>() {
                    window.move_quick_look_selection(direction);
                }
            }
            None
        });
        quick_look.proxy.replace(Some(proxy.clone()));
        if !quick_look.is_available.get() {
            self.find_activatable_previewer(proxy);
        }
    }

    /// Asks the bus whether it can start Sushi when it is not running.
    fn find_activatable_previewer(&self, proxy: &gio::DBusProxy) {
        let window = self.downgrade();
        proxy.connection().call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListActivatableNames",
            None,
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            gio::Cancellable::NONE,
            move |reply| {
                let listed = reply
                    .ok()
                    .and_then(|reply| reply.get::<(Vec<String>,)>())
                    .is_some_and(|(names,)| names.iter().any(|name| name == PREVIEWER_NAME));
                if let Some(window) = window.upgrade().filter(|_| listed) {
                    window.imp().quick_look.is_available.set(true);
                }
            },
        );
    }

    /// Stops following the selection once Sushi's window is gone.
    fn previewer_visibility_changed(&self) {
        let quick_look = &self.imp().quick_look;
        let proxy = quick_look.proxy.borrow();
        let visible = proxy
            .as_ref()
            .and_then(|proxy| proxy.cached_property("Visible"))
            .and_then(|visible| visible.get::<bool>());
        match visible {
            Some(true) => quick_look.was_visible.set(true),
            Some(false) if quick_look.was_visible.replace(false) => quick_look.is_open.set(false),
            _ => {}
        }
    }

    /// Moves the selection one item back or on, as an arrow key pressed
    /// in Sushi's window asks.
    fn move_quick_look_selection(&self, direction: u32) {
        if !self.imp().quick_look.is_open.get() {
            return;
        }
        let model = self.folder_pane().model();
        let Some(&current) = model.selected_positions().first() else {
            return;
        };
        let target = if matches!(direction, DIRECTION_UP | DIRECTION_LEFT) {
            current.checked_sub(1)
        } else {
            Some(current + 1).filter(|next| *next < model.n_items())
        };
        if let Some(target) = target {
            model.select_only(target);
            self.folder_pane().reveal(target);
        }
    }

    /// Space without a typed prefix: closes an open preview, or previews
    /// the one selected file. False when there is nothing to do here, so
    /// Space goes on to the view.
    pub(super) fn toggle_quick_look(&self) -> bool {
        let quick_look = &self.imp().quick_look;
        if quick_look.is_open.get() {
            self.close_quick_look();
            return true;
        }
        let Some(uri) = self.quick_look_target() else {
            return false;
        };
        if !quick_look.is_available.get() {
            return false;
        }
        let Some(proxy) = quick_look.proxy.borrow().clone() else {
            return false;
        };
        quick_look.is_open.set(true);
        quick_look.was_visible.set(false);
        show_file(&proxy, &uri);
        true
    }

    /// Escape: closes an open preview; false when none is open.
    pub(super) fn close_quick_look(&self) -> bool {
        let quick_look = &self.imp().quick_look;
        if !quick_look.is_open.replace(false) {
            return false;
        }
        if let Some(proxy) = quick_look.proxy.borrow().as_ref() {
            call(proxy, "Close", None);
        }
        true
    }

    /// Shows the newly selected file in an open preview, as the selection
    /// moves.
    pub(super) fn follow_quick_look(&self) {
        let quick_look = &self.imp().quick_look;
        if !quick_look.is_open.get() {
            return;
        }
        let proxy = quick_look.proxy.borrow().clone();
        if let (Some(uri), Some(proxy)) = (self.quick_look_target(), proxy) {
            show_file(&proxy, &uri);
        }
    }

    /// Whether Space can preview here, for tests.
    #[cfg(test)]
    pub(super) fn quick_look_is_available(&self) -> bool {
        self.imp().quick_look.is_available.get()
    }

    /// Whether a preview this window asked for is shown, for tests.
    #[cfg(test)]
    pub(super) fn quick_look_is_open(&self) -> bool {
        self.imp().quick_look.is_open.get()
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

/// `ShowFile(uri, parent handle, close if shown)`. The window's handle
/// needs the X11 or Wayland GDK bindings, which the app does not link, so
/// the preview opens as its own window.
fn show_file(proxy: &gio::DBusProxy, uri: &str) {
    call(proxy, "ShowFile", Some(&(uri, "", false).to_variant()));
}

/// Calls `method` on the previewer without waiting for it.
fn call(proxy: &gio::DBusProxy, method: &str, arguments: Option<&glib::Variant>) {
    proxy.call(
        method,
        arguments,
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
        gio::Cancellable::NONE,
        |_| {},
    );
}
