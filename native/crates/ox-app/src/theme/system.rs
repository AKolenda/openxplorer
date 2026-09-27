// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's light or dark preference.
//!
//! Ports `system_dark` and `on_system_theme_changed` in
//! `desktop/winspace.py`: GNOME's `org.gnome.desktop.interface`
//! `color-scheme` key decides when the schema is installed (`prefer-dark`
//! or `prefer-light`; `default` falls back to a GTK theme name containing
//! "dark"). Without the schema, the XDG desktop portal's
//! `org.freedesktop.appearance color-scheme` setting is read instead.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";

/// Dark or not, from GNOME's keys. `None` when they do not decide.
pub fn dark_from_gnome_keys(color_scheme: Option<&str>, gtk_theme: Option<&str>) -> Option<bool> {
    match color_scheme {
        Some("prefer-dark") => return Some(true),
        Some("prefer-light") => return Some(false),
        _ => {}
    }
    gtk_theme.map(|name| name.to_lowercase().contains("dark"))
}

/// Dark or not, from the portal's value (1 prefers dark, 2 prefers light,
/// 0 has no preference).
pub fn dark_from_portal(value: u32) -> Option<bool> {
    match value {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Watches the desktop colour scheme and reports changes.
///
/// The application creates one and forwards changes to the shared
/// [`Skin`](super::Skin); windows never register here, so closing a
/// window leaves nothing behind.
pub struct SystemScheme {
    settings: Option<gio::Settings>,
    portal_dark: RefCell<Option<bool>>,
    on_change: Box<dyn Fn(bool)>,
    portal_watch: RefCell<Option<gio::SignalSubscription>>,
    portal_signal_seen: Cell<bool>,
}

impl std::fmt::Debug for SystemScheme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemScheme")
            .field("gnome_settings", &self.settings.is_some())
            .field("portal_dark", &self.portal_dark)
            .finish_non_exhaustive()
    }
}

impl SystemScheme {
    /// Starts watching and calls `on_change` with the new value whenever the
    /// scheme changes. With no GNOME schema the portal is queried
    /// asynchronously, and `on_change` hears its answer.
    pub fn new(on_change: impl Fn(bool) + 'static) -> Rc<Self> {
        let settings = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(INTERFACE_SCHEMA, true))
            .map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None));
        let scheme = Rc::new(Self {
            settings,
            portal_dark: RefCell::new(None),
            on_change: Box::new(on_change),
            portal_watch: RefCell::new(None),
            portal_signal_seen: Cell::new(false),
        });
        scheme.watch();
        scheme
    }

    /// True when the desktop prefers dark.
    pub fn is_dark(&self) -> bool {
        if let Some(dark) = self.gnome_dark() {
            return dark;
        }
        if let Some(dark) = *self.portal_dark.borrow() {
            return dark;
        }
        gtk::Settings::default().is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme())
    }

    fn notify(&self) {
        (self.on_change)(self.is_dark());
    }

    fn gnome_dark(&self) -> Option<bool> {
        let settings = self.settings.as_ref()?;
        let schema = settings.settings_schema()?;
        let read = |key: &str| schema.has_key(key).then(|| settings.string(key).to_string());
        let color_scheme = read("color-scheme");
        let gtk_theme = read("gtk-theme");
        dark_from_gnome_keys(color_scheme.as_deref(), gtk_theme.as_deref())
    }

    fn watch(self: &Rc<Self>) {
        if let Some(settings) = &self.settings {
            for key in ["color-scheme", "gtk-theme"] {
                let has_key = settings
                    .settings_schema()
                    .is_some_and(|schema| schema.has_key(key));
                if !has_key {
                    continue;
                }
                let weak = Rc::downgrade(self);
                settings.connect_changed(Some(key), move |_, _| {
                    if let Some(scheme) = weak.upgrade() {
                        scheme.notify();
                    }
                });
            }
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
                return;
            };
            let changed = weak.clone();
            let subscription = connection.subscribe_to_signal(
                Some(PORTAL_NAME),
                Some(PORTAL_SETTINGS),
                Some("SettingChanged"),
                Some(PORTAL_PATH),
                Some(APPEARANCE_NAMESPACE),
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    let value = portal_change(signal.parameters);
                    if let (Some(scheme), Some(value)) = (changed.upgrade(), value) {
                        scheme.portal_signal_seen.set(true);
                        scheme.portal_dark.replace(dark_from_portal(value));
                        scheme.notify();
                    }
                },
            );
            if let Some(scheme) = weak.upgrade() {
                scheme.portal_watch.replace(Some(subscription));
            } else {
                return;
            }
            if let (Some(value), Some(scheme)) = (read_portal(&connection).await, weak.upgrade()) {
                // A newer SettingChanged signal wins over the initial reply.
                if !scheme.portal_signal_seen.get() {
                    scheme.portal_dark.replace(dark_from_portal(value));
                    scheme.notify();
                }
            }
        });
    }
}

/// Reads the portal's colour scheme, or `None` without a portal.
async fn read_portal(connection: &gio::DBusConnection) -> Option<u32> {
    let arguments = (APPEARANCE_NAMESPACE, "color-scheme").to_variant();
    let reply = connection
        .call_future(
            Some(PORTAL_NAME),
            PORTAL_PATH,
            PORTAL_SETTINGS,
            "ReadOne",
            Some(&arguments),
            None,
            gio::DBusCallFlags::NONE,
            2000,
        )
        .await
        .ok()?;
    // The reply is `(v)`; the variant holds a `u`.
    let boxed = reply.child_value(0);
    let inner = boxed.as_variant()?;
    inner.get::<u32>()
}

fn portal_change(parameters: &glib::Variant) -> Option<u32> {
    let (namespace, key, value) = parameters.get::<(String, String, glib::Variant)>()?;
    if namespace != APPEARANCE_NAMESPACE || key != "color-scheme" {
        return None;
    }
    value.get::<u32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_decides_first() {
        assert_eq!(
            dark_from_gnome_keys(Some("prefer-dark"), Some("Adwaita")),
            Some(true)
        );
        assert_eq!(
            dark_from_gnome_keys(Some("prefer-light"), Some("ZorinBlue-Dark")),
            Some(false)
        );
    }

    #[test]
    fn default_scheme_falls_back_to_the_theme_name() {
        assert_eq!(
            dark_from_gnome_keys(Some("default"), Some("ZorinBlue-Dark")),
            Some(true)
        );
        assert_eq!(
            dark_from_gnome_keys(Some("default"), Some("ZorinBlue-Light")),
            Some(false)
        );
        assert_eq!(dark_from_gnome_keys(None, None), None);
    }

    #[test]
    fn portal_values_follow_the_specification() {
        assert_eq!(dark_from_portal(1), Some(true));
        assert_eq!(dark_from_portal(2), Some(false));
        assert_eq!(dark_from_portal(0), None);
    }

    #[test]
    fn portal_changes_accept_only_the_appearance_color_scheme() {
        let change = |namespace: &str, key: &str, value: glib::Variant| (namespace, key, value).to_variant();
        assert_eq!(
            portal_change(&change(APPEARANCE_NAMESPACE, "color-scheme", 1u32.to_variant())),
            Some(1)
        );
        assert_eq!(
            portal_change(&change(APPEARANCE_NAMESPACE, "color-scheme", 0u32.to_variant())),
            Some(0)
        );
        assert_eq!(
            portal_change(&change("another.namespace", "color-scheme", 1u32.to_variant())),
            None
        );
        assert_eq!(
            portal_change(&change(APPEARANCE_NAMESPACE, "accent-color", 1u32.to_variant())),
            None
        );
        assert_eq!(
            portal_change(&change(APPEARANCE_NAMESPACE, "color-scheme", "dark".to_variant())),
            None
        );
    }
}
