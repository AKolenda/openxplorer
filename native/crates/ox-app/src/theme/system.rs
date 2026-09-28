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
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gio, glib};

use super::Appearance;

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
const COLOR_SCHEME_KEY: &str = "color-scheme";
const GTK_THEME_KEY: &str = "gtk-theme";

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";

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
    /// The portal's last answer, when there is no GNOME schema.
    portal_appearance: Cell<Option<Appearance>>,
    /// Hears the desktop's appearance whenever it may have changed.
    on_change: Box<dyn Fn(Appearance)>,
    /// Keeps the portal's `SettingChanged` subscription alive.
    portal_subscription: RefCell<Option<gio::SignalSubscription>>,
    /// A `SettingChanged` signal arrived; it is newer than the first read.
    portal_signal_seen: Cell<bool>,
}

impl std::fmt::Debug for SystemScheme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemScheme")
            .field("gnome_settings", &self.gnome_settings.is_some())
            .field("portal_appearance", &self.portal_appearance)
            .finish_non_exhaustive()
    }
}

impl SystemScheme {
    /// Starts watching and calls `on_change` with the desktop's appearance
    /// whenever the scheme changes. With no GNOME schema the portal is
    /// queried asynchronously, and `on_change` hears its answer.
    pub(crate) fn new(on_change: impl Fn(Appearance) + 'static) -> Rc<Self> {
        let gnome_settings = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(INTERFACE_SCHEMA, true))
            .map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None));
        let scheme = Rc::new(Self {
            gnome_settings,
            portal_appearance: Cell::new(None),
            on_change: Box::new(on_change),
            portal_subscription: RefCell::new(None),
            portal_signal_seen: Cell::new(false),
        });
        match &scheme.gnome_settings {
            Some(settings) => scheme.watch_gnome_keys(settings),
            None => {
                glib::spawn_future_local(follow_portal(Rc::downgrade(&scheme)));
            }
        }
        scheme
    }

    /// The appearance the desktop asks for: GNOME's keys decide, else the
    /// portal, else GTK's own dark preference.
    pub(crate) fn appearance(&self) -> Appearance {
        if let Some(appearance) = self.gnome_appearance() {
            return appearance;
        }
        if let Some(appearance) = self.portal_appearance.get() {
            return appearance;
        }
        let prefers_dark =
            gtk::Settings::default().is_some_and(|settings| settings.is_gtk_application_prefer_dark_theme());
        if prefers_dark {
            Appearance::Dark
        } else {
            Appearance::Light
        }
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

    /// Records the portal's value and reports the scheme.
    fn set_portal_value(&self, value: u32) {
        self.portal_appearance.set(appearance_from_portal(value));
        self.notify();
    }
}

/// Follows the XDG desktop portal's colour scheme: every change, and the
/// current value unless a change arrived first.
async fn follow_portal(scheme: Weak<SystemScheme>) {
    let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
        return;
    };
    let subscription = subscribe_to_portal_changes(&connection, scheme.clone());
    let Some(watching) = scheme.upgrade() else {
        return;
    };
    watching.portal_subscription.replace(Some(subscription));
    // Hold no strong reference across the read, so the application can
    // drop the scheme while the portal is slow to answer.
    drop(watching);
    let Some(value) = read_portal(&connection).await else {
        return;
    };
    let Some(scheme) = scheme.upgrade() else {
        return;
    };
    // A newer SettingChanged signal wins over the initial reply.
    if !scheme.portal_signal_seen.get() {
        scheme.set_portal_value(value);
    }
}

/// Subscribes `scheme` to the portal's `SettingChanged` signals for the
/// appearance namespace.
fn subscribe_to_portal_changes(
    connection: &gio::DBusConnection,
    scheme: Weak<SystemScheme>,
) -> gio::SignalSubscription {
    connection.subscribe_to_signal(
        Some(PORTAL_NAME),
        Some(PORTAL_SETTINGS),
        Some("SettingChanged"),
        Some(PORTAL_PATH),
        Some(APPEARANCE_NAMESPACE),
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let Some(scheme) = scheme.upgrade() else {
                return;
            };
            let Some(value) = portal_change(signal.parameters) else {
                return;
            };
            scheme.portal_signal_seen.set(true);
            scheme.set_portal_value(value);
        },
    )
}

/// Reads the portal's colour scheme, or `None` without a portal.
async fn read_portal(connection: &gio::DBusConnection) -> Option<u32> {
    let arguments = (APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY).to_variant();
    let reply = connection
        .call_future(
            Some(PORTAL_NAME),
            PORTAL_PATH,
            PORTAL_SETTINGS,
            "ReadOne",
            Some(&arguments),
            None,
            gio::DBusCallFlags::NONE,
            PORTAL_TIMEOUT_MS,
        )
        .await
        .ok()?;
    // The reply is `(v)`; the variant holds a `u`.
    let boxed = reply.child_value(0);
    let inner = boxed.as_variant()?;
    inner.get::<u32>()
}

/// The colour-scheme value a `SettingChanged` signal carries, or `None`
/// for any other setting.
fn portal_change(parameters: &glib::Variant) -> Option<u32> {
    let (namespace, key, value) = parameters.get::<(String, String, glib::Variant)>()?;
    if namespace != APPEARANCE_NAMESPACE || key != COLOR_SCHEME_KEY {
        return None;
    }
    value.get::<u32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: LOOK-004
    #[test]
    fn color_scheme_decides_first() {
        assert_eq!(
            appearance_from_gnome_keys(Some("prefer-dark"), Some("Adwaita")),
            Some(Appearance::Dark)
        );
        assert_eq!(
            appearance_from_gnome_keys(Some("prefer-light"), Some("ZorinBlue-Dark")),
            Some(Appearance::Light)
        );
    }

    /// parity: LOOK-004
    #[test]
    fn default_scheme_falls_back_to_the_theme_name() {
        assert_eq!(
            appearance_from_gnome_keys(Some("default"), Some("ZorinBlue-Dark")),
            Some(Appearance::Dark)
        );
        assert_eq!(
            appearance_from_gnome_keys(Some("default"), Some("ZorinBlue-Light")),
            Some(Appearance::Light)
        );
        assert_eq!(appearance_from_gnome_keys(None, None), None);
    }

    /// parity: LOOK-004
    #[test]
    fn portal_values_follow_the_specification() {
        assert_eq!(appearance_from_portal(1), Some(Appearance::Dark));
        assert_eq!(appearance_from_portal(2), Some(Appearance::Light));
        assert_eq!(appearance_from_portal(0), None);
    }

    /// One `SettingChanged` signal and the colour-scheme value expected
    /// from it.
    struct PortalSignalCase {
        namespace: &'static str,
        key: &'static str,
        value: glib::Variant,
        expected: Option<u32>,
    }

    /// parity: LOOK-004
    #[test]
    fn portal_changes_accept_only_the_appearance_color_scheme() {
        let cases = [
            PortalSignalCase {
                namespace: APPEARANCE_NAMESPACE,
                key: "color-scheme",
                value: 1u32.to_variant(),
                expected: Some(1),
            },
            PortalSignalCase {
                namespace: APPEARANCE_NAMESPACE,
                key: "color-scheme",
                value: 0u32.to_variant(),
                expected: Some(0),
            },
            PortalSignalCase {
                namespace: "another.namespace",
                key: "color-scheme",
                value: 1u32.to_variant(),
                expected: None,
            },
            PortalSignalCase {
                namespace: APPEARANCE_NAMESPACE,
                key: "accent-color",
                value: 1u32.to_variant(),
                expected: None,
            },
            PortalSignalCase {
                namespace: APPEARANCE_NAMESPACE,
                key: "color-scheme",
                value: "dark".to_variant(),
                expected: None,
            },
        ];
        for case in cases {
            let parameters = (case.namespace, case.key, case.value).to_variant();
            assert_eq!(
                portal_change(&parameters),
                case.expected,
                "{}.{}",
                case.namespace,
                case.key
            );
        }
    }
}
