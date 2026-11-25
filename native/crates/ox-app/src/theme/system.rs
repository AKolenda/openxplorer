// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's light or dark preference.
//!
//! Ports `system_dark` and `on_system_theme_changed` in
//! `v2.0.0:desktop/winspace.py`, and asks every desktop, not only GNOME:
//!
//! 1. GNOME's `org.gnome.desktop.interface color-scheme` key, when it says
//!    `prefer-dark` or `prefer-light`.
//! 2. Else the XDG desktop portal's `org.freedesktop.appearance
//!    color-scheme`, when it has a preference. KDE Plasma, Cinnamon and
//!    COSMIC answer here, while GNOME's schema, which most distributions
//!    install anyway, keeps its `default`.
//! 3. Else dark when any GTK theme name the desktop gives contains "dark"
//!    (`ZorinBlue-Dark`, `Mint-Y-Dark`): GNOME's `gtk-theme` key, the one
//!    GNOME-based portals serve, or GTK's own theme name, which Xfce and
//!    MATE set over XSETTINGS; or when GTK's own dark preference was on at
//!    startup (`gtk_system_dark`). Light otherwise.
//!
//! Inside Flatpak the runtime ships GNOME's schema too, but the sandbox
//! cannot read the host's settings, so the schema holds only its defaults
//! and would always ask for light. There GNOME's keys never decide, and the
//! portal, which answers with the host's colour scheme, does.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// The appearance GNOME's `color-scheme` asks for, or `None` for
/// `default`, which leaves it to the other settings.
fn appearance_from_gnome_scheme(color_scheme: Option<&str>) -> Option<Appearance> {
    match color_scheme {
        Some("prefer-dark") => Some(Appearance::Dark),
        Some("prefer-light") => Some(Appearance::Light),
        _ => None,
    }
}

/// The appearance the desktop asks for: the first colour scheme with a
/// preference in `schemes` (GNOME's, then the portal's), else dark when
/// any of the `hints` from theme names and GTK's preference is dark, else
/// light.
fn decide(schemes: &[Option<Appearance>], hints: &[Option<Appearance>]) -> Appearance {
    if let Some(chosen) = schemes.iter().find_map(|scheme| *scheme) {
        return chosen;
    }
    if hints.contains(&Some(Appearance::Dark)) {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// Dark for a GTK theme whose name contains "dark" (`ZorinBlue-Dark`), as
/// `system_dark` in winspace.py decides, or that is GTK's
/// `HighContrastInverse`, light text on black; light otherwise.
fn appearance_of_theme_name(name: &str) -> Appearance {
    let name = name.to_lowercase();
    if name.contains("dark") || name.starts_with("highcontrastinverse") {
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
/// in the user's GTK settings, or GTK's `HighContrastInverse` theme, light
/// text on black). Read it before
/// [`Skin::install`](super::Skin::install) forces the flag off: afterwards
/// the flag only says which palette the skin drew last. winspace.py reads
/// `gtk_system_dark` before theming, too.
pub(crate) fn gtk_preference(display: &gdk::Display) -> Appearance {
    let settings = gtk::Settings::for_display(display);
    // GTK_THEME wins over the settings' theme name, as in GTK.
    let theme = std::env::var("GTK_THEME")
        .ok()
        .or_else(|| settings.gtk_theme_name().map(|name| name.to_string()))
        .unwrap_or_default();
    let inverse = theme.to_lowercase().starts_with("highcontrastinverse");
    if settings.is_gtk_application_prefer_dark_theme() || inverse {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// What GTK says of the desktop's style before
/// [`Skin::install`](super::Skin::install) forces GTK's own theme: its dark
/// preference and the appearance of its theme name.
#[derive(Debug, Clone)]
pub(crate) struct GtkDesktop {
    /// GTK's dark preference ([`gtk_preference`]).
    preference: Appearance,
    /// The appearance of GTK's theme name, which XSETTINGS sets on X11
    /// desktops such as Xfce and MATE.
    theme: Option<Appearance>,
    /// The display whose theme name the scheme follows; `None` in tests
    /// that set the preference themselves.
    display: Option<gdk::Display>,
}

impl GtkDesktop {
    /// Reads GTK's preference and theme name on `display`, before the skin
    /// is installed there.
    pub(crate) fn read(display: &gdk::Display) -> Self {
        let theme = gtk::Settings::for_display(display).gtk_theme_name();
        Self {
            preference: gtk_preference(display),
            theme: theme.as_deref().map(appearance_of_theme_name),
            display: Some(display.clone()),
        }
    }

    /// GTK on no display, preferring `preference` with no theme name.
    #[cfg(test)]
    pub(crate) fn preferring(preference: Appearance) -> Self {
        Self {
            preference,
            theme: None,
            display: None,
        }
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
    /// The portal's last colour scheme.
    portal_appearance: Cell<Option<Appearance>>,
    /// The appearance of the GTK theme name the portal reports.
    portal_theme: Cell<Option<Appearance>>,
    /// The appearance of GTK's own theme name, as the desktop last set it.
    gtk_theme: Cell<Option<Appearance>>,
    /// GTK's dark preference as read at startup, before the skin forced
    /// it.
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
            .field("gtk_theme", &self.gtk_theme)
            .field("gtk_fallback", &self.gtk_fallback)
            .finish_non_exhaustive()
    }
}

impl SystemScheme {
    /// Starts watching and calls `on_change` with the desktop's appearance
    /// whenever the scheme changes: GNOME's keys, when their schema is
    /// installed, GTK's theme name on the display `gtk` was read from, and
    /// the portal, which is queried asynchronously; `on_change` hears its
    /// answer.
    ///
    /// `gtk` is what GTK said of the desktop before the skin was installed.
    pub(crate) fn new(gtk: &GtkDesktop, on_change: impl Fn(Appearance) + 'static) -> Rc<Self> {
        let gnome_settings = gnome_interface_settings(Sandbox::detect());
        let scheme = Rc::new(Self::unwatched(
            gnome_settings,
            gtk.preference,
            Box::new(on_change),
        ));
        scheme.gtk_theme.set(gtk.theme);
        if let Some(settings) = &scheme.gnome_settings {
            scheme.watch_gnome_keys(settings);
        }
        if let Some(display) = &gtk.display {
            scheme.watch_gtk_theme(display);
        }
        glib::spawn_future_local(follow_session_portal(Rc::downgrade(&scheme)));
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
            gtk_theme: Cell::new(None),
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

    /// The appearance the desktop asks for: GNOME's colour scheme, else
    /// the portal's, else dark when a GTK theme name (GNOME's, the
    /// portal's or GTK's own) or GTK's preference at startup, before the
    /// skin forced it (`gtk_system_dark` in winspace.py), asks for dark.
    pub(crate) fn appearance(&self) -> Appearance {
        let (gnome_scheme, gnome_theme) = self.gnome_keys();
        decide(
            &[gnome_scheme, self.portal_appearance.get()],
            &[
                gnome_theme,
                self.portal_theme.get(),
                self.gtk_theme.get(),
                Some(self.gtk_fallback),
            ],
        )
    }

    fn notify(&self) {
        (self.on_change)(self.appearance());
    }

    /// The appearances GNOME's `color-scheme` and `gtk-theme` keys ask
    /// for, reading only the keys the installed schema has.
    fn gnome_keys(&self) -> (Option<Appearance>, Option<Appearance>) {
        let Some(settings) = self.gnome_settings.as_ref() else {
            return (None, None);
        };
        let Some(schema) = settings.settings_schema() else {
            return (None, None);
        };
        let color_scheme = string_key(settings, &schema, COLOR_SCHEME_KEY);
        let gtk_theme = string_key(settings, &schema, GTK_THEME_KEY);
        (
            appearance_from_gnome_scheme(color_scheme.as_deref()),
            gtk_theme.as_deref().map(appearance_of_theme_name),
        )
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

    /// Follows GTK's theme name on `display`, which X11 desktops change
    /// over XSETTINGS.
    fn watch_gtk_theme(self: &Rc<Self>, display: &gdk::Display) {
        display.connect_setting_changed(glib::clone!(
            #[weak(rename_to = scheme)]
            self,
            move |display, name| {
                if name != "gtk-theme-name" {
                    return;
                }
                let theme = super::providers::desktop_theme_name(display);
                scheme
                    .gtk_theme
                    .set(theme.as_deref().map(appearance_of_theme_name));
                scheme.notify();
            }
        ));
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

/// Set for a launch by D-Bus activation, which the desktop portal itself
/// may have started while it starts, to serve Open and Save dialogs
/// (INT-032).
static WAIT_FOR_PORTAL: AtomicBool = AtomicBool::new(false);

/// Makes reading the portal's settings wait for the portal to run rather
/// than start it. Without systemd, starting it from a backend the portal
/// is waiting for would make the bus spawn a second portal (INT-032).
pub(crate) fn wait_for_portal_before_reading() {
    WAIT_FOR_PORTAL.store(true, Ordering::Relaxed);
}

/// Returns once the desktop portal has an owner on `connection`, without
/// starting it, when [`wait_for_portal_before_reading`] was called; at
/// once otherwise.
pub(crate) async fn portal_ready(connection: &gio::DBusConnection) {
    if !WAIT_FOR_PORTAL.load(Ordering::Relaxed) {
        return;
    }
    let (appeared, appearance) = async_channel::bounded::<()>(1);
    let watcher = gio::bus_watch_name_on_connection(
        connection,
        DESKTOP_PORTAL_NAME,
        gio::BusNameWatcherFlags::NONE,
        move |_, _, _| {
            let _ = appeared.try_send(());
        },
        |_, _| {},
    );
    let _ = appearance.recv().await;
    gio::bus_unwatch_name(watcher);
}

/// Follows the XDG desktop portal on the session bus.
async fn follow_session_portal(scheme: Weak<SystemScheme>) {
    let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
        return;
    };
    portal_ready(&connection).await;
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
