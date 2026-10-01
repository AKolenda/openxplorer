// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's light or dark preference.
//!
//! Ports `system_dark` and `on_system_theme_changed` in
//! `desktop/winspace.py`: GNOME's `org.gnome.desktop.interface`
//! `color-scheme` key decides when the schema is installed (`prefer-dark`
//! or `prefer-light`; `default` falls back to a GTK theme name containing
//! "dark"). Without the schema, the XDG desktop portal's
//! `org.freedesktop.appearance color-scheme` setting is read instead, and
//! while the portal has no preference, GTK's own dark preference as it was
//! when the app started (`gtk_system_dark`).
//!
//! Inside Flatpak the runtime ships GNOME's schema too, but the sandbox
//! cannot read the host's settings, so the schema holds only its defaults
//! and would always ask for light. There the portal, which answers with the
//! host's colour scheme, decides as on a desktop without the schema. While
//! its `color-scheme` has no preference, the GTK theme name that GNOME-based
//! portals also serve (`org.gnome.desktop.interface gtk-theme`) decides as
//! GNOME's own key does on the host: "dark" in the name asks for dark.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::integration::{Sandbox, DESKTOP_PORTAL_NAME, DESKTOP_PORTAL_PATH};

use super::{Appearance, INTERFACE_SCHEMA};

const COLOR_SCHEME_KEY: &str = "color-scheme";
/// GNOME's GTK theme name, which GNOME-based portals serve too.
pub(super) const GTK_THEME_KEY: &str = "gtk-theme";

/// The interface of the XDG desktop portal that serves desktop settings.
pub(super) const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
/// The portal's namespace for desktop-neutral appearance settings.
pub(super) const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";

/// How long the portal may take to answer the first read, in milliseconds.
const PORTAL_TIMEOUT_MS: i32 = 2000;

/// The appearance GNOME's keys ask for, or `None` when they do not decide.
fn appearance_from_gnome_keys(color_scheme: Option<&str>, gtk_theme: Option<&str>) -> Option<Appearance> {
    match color_scheme {
        Some("prefer-dark") => return Some(Appearance::Dark),
        Some("prefer-light") => return Some(Appearance::Light),
        _ => {}
    }
    gtk_theme.map(appearance_of_theme_name)
}

/// Dark for a GTK theme whose name contains "dark" (`ZorinBlue-Dark`), as
/// `system_dark` in winspace.py decides; light otherwise.
fn appearance_of_theme_name(name: &str) -> Appearance {
    if name.to_lowercase().contains("dark") {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// The appearance the portal's value asks for (1 prefers dark, 2 prefers
/// light), or `None` for 0, which has no preference.
fn appearance_from_portal(value: u32) -> Option<Appearance> {
    match value {
        1 => Some(Appearance::Dark),
        2 => Some(Appearance::Light),
        _ => None,
    }
}

/// A setting of the Settings portal that decides the appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PortalSetting {
    /// `org.freedesktop.appearance color-scheme`: 0 no preference, 1 dark,
    /// 2 light.
    ColorScheme(u32),
    /// GNOME's `gtk-theme`, which GNOME-based portals serve too.
    GtkTheme(String),
}

impl PortalSetting {
    /// The setting `value` holds for `key` in `namespace`, or `None` for
    /// any other setting or a value of the wrong type.
    fn read(namespace: &str, key: &str, value: &glib::Variant) -> Option<Self> {
        match (namespace, key) {
            (APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY) => value.get::<u32>().map(Self::ColorScheme),
            (INTERFACE_SCHEMA, GTK_THEME_KEY) => value.get::<String>().map(Self::GtkTheme),
            _ => None,
        }
    }
}

/// GTK's own dark preference on `display` (`gtk-application-prefer-dark-theme`
/// in the user's GTK settings). Read it before
/// [`Skin::install`](super::Skin::install) forces the flag off: afterwards
/// the flag only says which palette the skin drew last. winspace.py reads
/// `gtk_system_dark` before theming, too.
pub(crate) fn gtk_preference(display: &gdk::Display) -> Appearance {
    let settings = gtk::Settings::for_display(display);
    if settings.is_gtk_application_prefer_dark_theme() {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// GNOME's interface settings, when they are the desktop's: on the host
/// with the schema installed. Inside Flatpak they are the runtime's
/// defaults, not the desktop's, so the portal decides there.
fn gnome_interface_settings(sandbox: Sandbox) -> Option<gio::Settings> {
    if sandbox.is_flatpak() {
        return None;
    }
    super::desktop_settings(INTERFACE_SCHEMA)
}

/// The string value of `key`, when the installed `schema` has it.
fn string_key(settings: &gio::Settings, schema: &gio::SettingsSchema, key: &str) -> Option<String> {
    if !schema.has_key(key) {
        return None;
    }
    Some(settings.string(key).to_string())
}

/// Watches the desktop colour scheme and reports changes.
///
/// The application creates one and forwards changes to the shared
/// [`Skin`](super::Skin); windows never register here, so closing a
/// window leaves nothing behind.
pub(crate) struct SystemScheme {
    /// GNOME's interface settings, when the schema is installed.
    gnome_settings: Option<gio::Settings>,
    /// The portal's last colour scheme, when there is no GNOME schema.
    portal_appearance: Cell<Option<Appearance>>,
    /// The appearance of the GTK theme name the portal reports; decides
    /// while its colour scheme has no preference.
    portal_theme: Cell<Option<Appearance>>,
    /// GTK's dark preference as read at startup, before the skin forced
    /// it; the desktop's appearance when neither GNOME's keys nor the
    /// portal decide.
    gtk_fallback: Appearance,
    /// Hears the desktop's appearance whenever it may have changed.
    on_change: Box<dyn Fn(Appearance)>,
    /// Keeps the portal's `SettingChanged` subscription alive.
    portal_subscription: RefCell<Option<gio::SignalSubscription>>,
    /// A `SettingChanged` signal for the colour scheme arrived; it is
    /// newer than the first read.
    portal_scheme_seen: Cell<bool>,
    /// The same for the GTK theme name.
    portal_theme_seen: Cell<bool>,
}

impl std::fmt::Debug for SystemScheme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemScheme")
            .field("gnome_settings", &self.gnome_settings.is_some())
            .field("portal_appearance", &self.portal_appearance)
            .field("portal_theme", &self.portal_theme)
            .field("gtk_fallback", &self.gtk_fallback)
            .finish_non_exhaustive()
    }
}

impl SystemScheme {
    /// Starts watching and calls `on_change` with the desktop's appearance
    /// whenever the scheme changes. With no GNOME schema the portal is
    /// queried asynchronously, and `on_change` hears its answer.
    ///
    /// `gtk_fallback` is GTK's dark preference from [`gtk_preference`],
    /// read before the skin was installed.
    pub(crate) fn new(gtk_fallback: Appearance, on_change: impl Fn(Appearance) + 'static) -> Rc<Self> {
        let gnome_settings = gnome_interface_settings(Sandbox::detect());
        let scheme = Rc::new(Self::unwatched(gnome_settings, gtk_fallback, Box::new(on_change)));
        match &scheme.gnome_settings {
            Some(settings) => scheme.watch_gnome_keys(settings),
            None => {
                glib::spawn_future_local(follow_session_portal(Rc::downgrade(&scheme)));
            }
        }
        scheme
    }

    /// A scheme that reads `gnome_settings` but watches nothing yet.
    fn unwatched(
        gnome_settings: Option<gio::Settings>,
        gtk_fallback: Appearance,
        on_change: Box<dyn Fn(Appearance)>,
    ) -> Self {
        Self {
            gnome_settings,
            portal_appearance: Cell::new(None),
            portal_theme: Cell::new(None),
            gtk_fallback,
            on_change,
            portal_subscription: RefCell::new(None),
            portal_scheme_seen: Cell::new(false),
            portal_theme_seen: Cell::new(false),
        }
    }

    /// A scheme as it is on a desktop without GNOME's schema before the
    /// portal answers, for tests that set the portal's value themselves.
    #[cfg(test)]
    fn without_gnome_schema(gtk_fallback: Appearance) -> Self {
        Self::unwatched(None, gtk_fallback, Box::new(|_| {}))
    }

    /// The appearance the desktop asks for: GNOME's keys decide, else the
    /// portal's colour scheme, else the portal's GTK theme name, else the
    /// GTK preference read at startup, before the skin forced it
    /// (`gtk_system_dark` in winspace.py).
    pub(crate) fn appearance(&self) -> Appearance {
        self.gnome_appearance()
            .or(self.portal_appearance.get())
            .or(self.portal_theme.get())
            .unwrap_or(self.gtk_fallback)
    }

    fn notify(&self) {
        (self.on_change)(self.appearance());
    }

    /// The appearance GNOME's keys ask for, reading only the keys the
    /// installed schema has.
    fn gnome_appearance(&self) -> Option<Appearance> {
        let settings = self.gnome_settings.as_ref()?;
        let schema = settings.settings_schema()?;
        let color_scheme = string_key(settings, &schema, COLOR_SCHEME_KEY);
        let gtk_theme = string_key(settings, &schema, GTK_THEME_KEY);
        appearance_from_gnome_keys(color_scheme.as_deref(), gtk_theme.as_deref())
    }

    /// Follows GNOME's own keys, when their schema is installed.
    fn watch_gnome_keys(self: &Rc<Self>, settings: &gio::Settings) {
        let Some(schema) = settings.settings_schema() else {
            return;
        };
        let keys = [COLOR_SCHEME_KEY, GTK_THEME_KEY];
        for key in keys.into_iter().filter(|key| schema.has_key(key)) {
            settings.connect_changed(
                Some(key),
                glib::clone!(
                    #[weak(rename_to = scheme)]
                    self,
                    move |_, _| scheme.notify()
                ),
            );
        }
    }

    /// Records the portal's colour-scheme value and reports the scheme.
    #[cfg(test)]
    fn set_portal_value(&self, value: u32) {
        self.record(&PortalSetting::ColorScheme(value));
        self.notify();
    }

    /// Records one portal setting without reporting it.
    fn record(&self, setting: &PortalSetting) {
        match setting {
            PortalSetting::ColorScheme(value) => self.portal_appearance.set(appearance_from_portal(*value)),
            PortalSetting::GtkTheme(name) => self.portal_theme.set(Some(appearance_of_theme_name(name))),
        }
    }

    /// Whether a `SettingChanged` signal for `setting` arrived, which is
    /// newer than the first read.
    fn signal_seen(&self, setting: &PortalSetting) -> &Cell<bool> {
        match setting {
            PortalSetting::ColorScheme(_) => &self.portal_scheme_seen,
            PortalSetting::GtkTheme(_) => &self.portal_theme_seen,
        }
    }
}

/// Follows the XDG desktop portal on the session bus.
async fn follow_session_portal(scheme: Weak<SystemScheme>) {
    let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
        return;
    };
    follow_portal(scheme, &connection, DESKTOP_PORTAL_NAME).await;
}

/// Follows the colour scheme and GTK theme name of the portal that
/// `portal` names on `connection`: every change, and the current values
/// unless a change arrived first.
async fn follow_portal(scheme: Weak<SystemScheme>, connection: &gio::DBusConnection, portal: &str) {
    let subscription = subscribe_to_portal_changes(connection, portal, scheme.clone());
    let Some(watching) = scheme.upgrade() else {
        return;
    };
    watching.portal_subscription.replace(Some(subscription));
    // Hold no strong reference across the read, so the application can
    // drop the scheme while the portal is slow to answer.
    drop(watching);
    let mut settings = Vec::new();
    for (namespace, key) in [
        (APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY),
        (INTERFACE_SCHEMA, GTK_THEME_KEY),
    ] {
        let value = read_portal(connection, portal, namespace, key).await;
        settings.extend(value.and_then(|value| PortalSetting::read(namespace, key, &value)));
    }
    let Some(scheme) = scheme.upgrade() else {
        return;
    };
    // A newer SettingChanged signal wins over the initial reply.
    let fresh: Vec<_> = settings
        .iter()
        .filter(|setting| !scheme.signal_seen(setting).get())
        .collect();
    if fresh.is_empty() {
        return;
    }
    for setting in fresh {
        scheme.record(setting);
    }
    scheme.notify();
}

/// Subscribes `scheme` to the `SettingChanged` signals of `portal`; the
/// colour scheme and the GTK theme name are in different namespaces.
fn subscribe_to_portal_changes(
    connection: &gio::DBusConnection,
    portal: &str,
    scheme: Weak<SystemScheme>,
) -> gio::SignalSubscription {
    connection.subscribe_to_signal(
        Some(portal),
        Some(PORTAL_SETTINGS),
        Some("SettingChanged"),
        Some(DESKTOP_PORTAL_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let Some(scheme) = scheme.upgrade() else {
                return;
            };
            let Some(setting) = portal_change(signal.parameters) else {
                return;
            };
            scheme.signal_seen(&setting).set(true);
            scheme.record(&setting);
            scheme.notify();
        },
    )
}

/// Reads `key` in `namespace` from `portal`, or `None` without a portal
/// or a setting it does not serve.
pub(super) async fn read_portal(
    connection: &gio::DBusConnection,
    portal: &str,
    namespace: &str,
    key: &str,
) -> Option<glib::Variant> {
    let arguments = (namespace, key).to_variant();
    let reply = connection
        .call_future(
            Some(portal),
            DESKTOP_PORTAL_PATH,
            PORTAL_SETTINGS,
            "ReadOne",
            Some(&arguments),
            None,
            gio::DBusCallFlags::NONE,
            PORTAL_TIMEOUT_MS,
        )
        .await
        .ok()?;
    // The reply is `(v)`.
    reply.child_value(0).as_variant()
}

/// The setting a `SettingChanged` signal carries, or `None` for one that
/// does not decide the appearance.
fn portal_change(parameters: &glib::Variant) -> Option<PortalSetting> {
    let (namespace, key, value) = parameters.get::<(String, String, glib::Variant)>()?;
    PortalSetting::read(&namespace, &key, &value)
}

#[cfg(test)]
mod portal_tests;

#[cfg(test)]
mod tests;
