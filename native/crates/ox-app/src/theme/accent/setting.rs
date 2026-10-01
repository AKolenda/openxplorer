// SPDX-License-Identifier: AGPL-3.0-only
//! Where the desktop's accent comes from, followed live.
//!
//! On the host GNOME's interface keys decide: an accent the user picked
//! in `accent-color` (GNOME 47 and later) first, else the Zorin OS theme
//! variant in `gtk-theme` (Zorin 18 is GNOME 46, which has no accent key).
//! Without the schema, and inside Flatpak where it holds only the
//! runtime's defaults, the Settings portal reports the same two settings:
//! `org.freedesktop.appearance accent-color` as a colour, mapped to the
//! nearest GNOME accent, and the `gtk-theme` that GNOME-based portals
//! serve.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{DESKTOP_PORTAL_NAME, DESKTOP_PORTAL_PATH};

use super::Accent;
use crate::theme::system::{read_portal, APPEARANCE_NAMESPACE, GTK_THEME_KEY, PORTAL_SETTINGS};
use crate::theme::{host_desktop_key, INTERFACE_SCHEMA};

/// The key behind GNOME's Settings > Appearance > Accent Color, and the
/// portal's key for the same choice.
const ACCENT_COLOR_KEY: &str = "accent-color";

/// The accent of a picked accent and a theme variant: a picked accent
/// other than blue wins, else the theme's, else the Windows blue.
fn chosen(picked: Option<Accent>, theme: Option<Accent>) -> Accent {
    picked
        .filter(|accent| *accent != Accent::Windows)
        .or(theme)
        .unwrap_or_default()
}

/// A portal setting that decides the accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortalAccent {
    /// `org.freedesktop.appearance accent-color`.
    Picked(Option<Accent>),
    /// GNOME's `gtk-theme`.
    Theme(Option<Accent>),
}

impl PortalAccent {
    /// The setting `value` holds for `key` in `namespace`, or `None` for
    /// any other setting or a value of the wrong type.
    fn read(namespace: &str, key: &str, value: &glib::Variant) -> Option<Self> {
        match (namespace, key) {
            (APPEARANCE_NAMESPACE, ACCENT_COLOR_KEY) => {
                let (red, green, blue) = value.get::<(f64, f64, f64)>()?;
                Some(Self::Picked(Accent::nearest_to(red, green, blue)))
            }
            (INTERFACE_SCHEMA, GTK_THEME_KEY) => {
                let name = value.get::<String>()?;
                Some(Self::Theme(Accent::from_zorin_theme(&name)))
            }
            _ => None,
        }
    }
}

/// What [`AccentSetting`] shares with its change handlers.
struct Source {
    /// GNOME's interface settings on the host, when they have `gtk-theme`.
    settings: Option<gio::Settings>,
    /// The portal's picked accent and theme accent, without the schema.
    portal_picked: Cell<Option<Accent>>,
    portal_theme: Cell<Option<Accent>>,
    /// Whether a `SettingChanged` signal set each portal value, so the
    /// slower first read does not overwrite it.
    picked_seen: Cell<bool>,
    theme_seen: Cell<bool>,
    /// Keeps the portal's `SettingChanged` subscription alive.
    subscription: RefCell<Option<gio::SignalSubscription>>,
    on_change: Box<dyn Fn(Accent)>,
}

impl Source {
    fn accent(&self) -> Accent {
        let Some(settings) = &self.settings else {
            return chosen(self.portal_picked.get(), self.portal_theme.get());
        };
        let has_picked = settings
            .settings_schema()
            .is_some_and(|schema| schema.has_key(ACCENT_COLOR_KEY));
        let picked = has_picked.then(|| Accent::from_gnome(&settings.string(ACCENT_COLOR_KEY)));
        chosen(picked, Accent::from_zorin_theme(&settings.string(GTK_THEME_KEY)))
    }

    fn notify(&self) {
        (self.on_change)(self.accent());
    }

    /// Records one portal setting.
    fn record(&self, setting: PortalAccent) {
        match setting {
            PortalAccent::Picked(accent) => self.portal_picked.set(accent),
            PortalAccent::Theme(accent) => self.portal_theme.set(accent),
        }
    }

    /// Whether a signal already set `setting`'s value.
    fn seen(&self, setting: PortalAccent) -> &Cell<bool> {
        match setting {
            PortalAccent::Picked(_) => &self.picked_seen,
            PortalAccent::Theme(_) => &self.theme_seen,
        }
    }
}

/// Follows the desktop's accent while it lives.
pub(crate) struct AccentSetting {
    source: Rc<Source>,
}

impl std::fmt::Debug for AccentSetting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccentSetting")
            .field("accent", &self.accent())
            .finish_non_exhaustive()
    }
}

impl AccentSetting {
    /// Starts following the desktop; `on_change` hears every later change.
    pub(crate) fn watch(on_change: impl Fn(Accent) + 'static) -> Self {
        let settings = host_desktop_key(INTERFACE_SCHEMA, GTK_THEME_KEY);
        let follows_portal = settings.is_none();
        let setting = Self::following(settings, on_change);
        if follows_portal {
            glib::spawn_future_local(follow_session_portal(Rc::downgrade(&setting.source)));
        }
        setting
    }

    /// Follows `settings`, or with none the portal values recorded later.
    fn following(settings: Option<gio::Settings>, on_change: impl Fn(Accent) + 'static) -> Self {
        let source = Rc::new(Source {
            settings,
            portal_picked: Cell::new(None),
            portal_theme: Cell::new(None),
            picked_seen: Cell::new(false),
            theme_seen: Cell::new(false),
            subscription: RefCell::new(None),
            on_change: Box::new(on_change),
        });
        if let Some(settings) = &source.settings {
            let watching = Rc::downgrade(&source);
            settings.connect_changed(None, move |_, key| {
                if ![ACCENT_COLOR_KEY, GTK_THEME_KEY].contains(&key) {
                    return;
                }
                if let Some(source) = watching.upgrade() {
                    source.notify();
                }
            });
        }
        Self { source }
    }

    /// The accent the desktop asks for now.
    pub(crate) fn accent(&self) -> Accent {
        self.source.accent()
    }
}

/// Follows the XDG desktop portal on the session bus.
async fn follow_session_portal(source: Weak<Source>) {
    let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
        return;
    };
    follow_portal(source, &connection, DESKTOP_PORTAL_NAME).await;
}

/// Follows the accent settings of the portal that `portal` names on
/// `connection`: every change, and the current values unless a change
/// arrived first.
async fn follow_portal(source: Weak<Source>, connection: &gio::DBusConnection, portal: &str) {
    let watching = source.clone();
    let subscription = connection.subscribe_to_signal(
        Some(portal),
        Some(PORTAL_SETTINGS),
        Some("SettingChanged"),
        Some(DESKTOP_PORTAL_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let Some(source) = watching.upgrade() else {
                return;
            };
            let change = signal.parameters.get::<(String, String, glib::Variant)>();
            let Some(setting) =
                change.and_then(|(namespace, key, value)| PortalAccent::read(&namespace, &key, &value))
            else {
                return;
            };
            source.seen(setting).set(true);
            source.record(setting);
            source.notify();
        },
    );
    let Some(watching) = source.upgrade() else {
        return;
    };
    watching.subscription.replace(Some(subscription));
    // Hold no strong reference across the reads.
    drop(watching);
    let mut settings = Vec::new();
    for (namespace, key) in [
        (APPEARANCE_NAMESPACE, ACCENT_COLOR_KEY),
        (INTERFACE_SCHEMA, GTK_THEME_KEY),
    ] {
        let value = read_portal(connection, portal, namespace, key).await;
        settings.extend(value.and_then(|value| PortalAccent::read(namespace, key, &value)));
    }
    let Some(source) = source.upgrade() else {
        return;
    };
    let fresh: Vec<_> = settings
        .into_iter()
        .filter(|setting| !source.seen(*setting).get())
        .collect();
    if fresh.is_empty() {
        return;
    }
    for setting in fresh {
        source.record(setting);
    }
    source.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::desktop_setting::DesktopSetting;
    use crate::test_support::harness::wait_until;
    use crate::test_support::portal::{session_connection, SettingsPortal};

    /// The accents a setting reported, last one last.
    type Heard = Rc<Cell<Option<Accent>>>;

    fn recorder() -> (Heard, impl Fn(Accent) + 'static) {
        let heard = Heard::default();
        let recorder = Rc::clone(&heard);
        (heard, move |accent| recorder.set(Some(accent)))
    }

    /// On the host a changed Zorin theme variant changes the accent at
    /// once, an accent picked in GNOME's key wins over it, and a theme of
    /// no accent brings the Windows blue back.
    ///
    /// parity: LOOK-024
    #[gtk::test]
    fn a_changed_desktop_accent_is_followed_at_once() {
        let Some(settings) = host_desktop_key(INTERFACE_SCHEMA, GTK_THEME_KEY) else {
            return;
        };
        let Some(theme) = DesktopSetting::in_memory(settings.clone(), GTK_THEME_KEY) else {
            return;
        };
        let (heard, on_change) = recorder();
        let setting = AccentSetting::watch(on_change);
        theme.set_string("ZorinGreen-Light");
        wait_until("the Zorin green", || heard.get() == Some(Accent::Green));
        assert_eq!(setting.accent(), Accent::Green);

        let has_picked = settings
            .settings_schema()
            .is_some_and(|schema| schema.has_key(ACCENT_COLOR_KEY));
        if let Some(picked) = has_picked
            .then(|| DesktopSetting::in_memory(settings.clone(), ACCENT_COLOR_KEY))
            .flatten()
        {
            picked.set_string("purple");
            wait_until("GNOME's purple", || heard.get() == Some(Accent::Purple));
            picked.set_string("blue");
            wait_until("the theme's green", || heard.get() == Some(Accent::Green));
        }

        theme.set_string("Adwaita");
        wait_until("the Windows blue", || heard.get() == Some(Accent::Windows));
    }

    /// Without GNOME's schema, as inside Flatpak, the portal's accent
    /// colour decides, its Zorin theme name when the colour is unset, and
    /// every change is followed.
    ///
    /// parity: LOOK-024
    #[gtk::test]
    fn the_portal_accent_is_followed_without_the_schema() {
        let unset = (-1.0_f64, -1.0_f64, -1.0_f64).to_variant();
        let portal = SettingsPortal::serving(&[
            (APPEARANCE_NAMESPACE, ACCENT_COLOR_KEY, unset.clone()),
            (INTERFACE_SCHEMA, GTK_THEME_KEY, "ZorinOrange-Dark".to_variant()),
        ]);
        let (heard, on_change) = recorder();
        let setting = AccentSetting::following(None, on_change);
        let client = session_connection();
        let name = portal.name();
        let source = Rc::downgrade(&setting.source);
        glib::spawn_future_local(async move { follow_portal(source, &client, &name).await });
        wait_until("the Zorin orange", || heard.get() == Some(Accent::Orange));

        let teal = (0.13_f64, 0.56_f64, 0.64_f64).to_variant();
        portal.change(APPEARANCE_NAMESPACE, ACCENT_COLOR_KEY, teal);
        wait_until("the portal's teal", || heard.get() == Some(Accent::Teal));

        portal.change(APPEARANCE_NAMESPACE, ACCENT_COLOR_KEY, unset);
        portal.change(INTERFACE_SCHEMA, GTK_THEME_KEY, "Yaru".to_variant());
        wait_until("the Windows blue", || heard.get() == Some(Accent::Windows));
        assert_eq!(setting.accent(), Accent::Windows);
    }
}
