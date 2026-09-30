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

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

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
    /// GTK's dark preference as read at startup, before the skin forced
    /// it; the desktop's appearance when neither GNOME's keys nor the
    /// portal decide.
    gtk_fallback: Appearance,
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
        let gnome_settings = super::desktop_settings(INTERFACE_SCHEMA);
        let scheme = Rc::new(Self::unwatched(gnome_settings, gtk_fallback, Box::new(on_change)));
        match &scheme.gnome_settings {
            Some(settings) => scheme.watch_gnome_keys(settings),
            None => {
                glib::spawn_future_local(follow_portal(Rc::downgrade(&scheme)));
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
            gtk_fallback,
            on_change,
            portal_subscription: RefCell::new(None),
            portal_signal_seen: Cell::new(false),
        }
    }

    /// A scheme as it is on a desktop without GNOME's schema before the
    /// portal answers, for tests that set the portal's value themselves.
    #[cfg(test)]
    fn without_gnome_schema(gtk_fallback: Appearance) -> Self {
        Self::unwatched(None, gtk_fallback, Box::new(|_| {}))
    }

    /// The appearance the desktop asks for: GNOME's keys decide, else the
    /// portal, else the GTK preference read at startup, before the skin
    /// forced it (`gtk_system_dark` in winspace.py).
    pub(crate) fn appearance(&self) -> Appearance {
        self.gnome_appearance()
            .or(self.portal_appearance.get())
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
    use crate::test_support::harness::wait_until;

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
    fn portal_values_one_and_two_prefer_dark_and_light() {
        assert_eq!(appearance_from_portal(1), Some(Appearance::Dark));
        assert_eq!(appearance_from_portal(2), Some(Appearance::Light));
        assert_eq!(appearance_from_portal(0), None);
    }

    /// Without GNOME's schema, a portal with no preference (0) leaves the
    /// GTK preference read at startup in charge, not the palette the skin
    /// has drawn on the display since.
    ///
    /// parity: LOOK-004
    #[gtk::test]
    fn a_portal_without_preference_falls_back_to_the_startup_gtk_preference() {
        let display = gdk::Display::default().expect("GTK tests run on a private display");
        let drawn_on_display = gtk_preference(&display);
        let startup_preference = match drawn_on_display {
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Light,
        };
        let scheme = SystemScheme::without_gnome_schema(startup_preference);
        assert_eq!(
            scheme.appearance(),
            startup_preference,
            "before the portal answers"
        );

        scheme.set_portal_value(2);
        assert_eq!(scheme.appearance(), Appearance::Light, "the portal prefers light");

        scheme.set_portal_value(0);
        assert_eq!(
            scheme.appearance(),
            startup_preference,
            "the portal has no preference"
        );
    }

    /// A change of GNOME's colour scheme reaches the app at once, both
    /// ways, while the app writes nothing back (the tests' settings stay
    /// in memory).
    ///
    /// parity: LOOK-004
    #[gtk::test]
    fn a_changed_gnome_color_scheme_is_followed_at_once() {
        let Some(settings) = super::super::desktop_settings(INTERFACE_SCHEMA) else {
            return;
        };
        let has_key = settings
            .settings_schema()
            .is_some_and(|schema| schema.has_key(COLOR_SCHEME_KEY));
        if !has_key {
            return;
        }
        let heard = Rc::new(Cell::new(None));
        let scheme = SystemScheme::new(Appearance::Light, {
            let heard = Rc::clone(&heard);
            move |appearance| heard.set(Some(appearance))
        });
        for (value, expected) in [("prefer-dark", Appearance::Dark), ("prefer-light", Appearance::Light)] {
            settings
                .set_string(COLOR_SCHEME_KEY, value)
                .expect("the key is writable in memory");
            wait_until(value, || heard.get() == Some(expected));
            assert_eq!(scheme.appearance(), expected);
        }
        settings.reset(COLOR_SCHEME_KEY);
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
