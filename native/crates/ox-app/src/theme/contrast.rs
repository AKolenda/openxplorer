// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's high-contrast setting.
//!
//! Ports the `@media (prefers-contrast: more)` rules of
//! `v2.0.0:desktop/ui/style.css`. Any of three asks for more contrast
//! (ui-spec.md §4.13, §8):
//!
//! - GNOME's `org.gnome.desktop.a11y.interface high-contrast` key
//!   (Settings > Accessibility), on the host with the schema installed;
//! - the XDG desktop portal's `org.freedesktop.appearance contrast` set
//!   to 1, which is how the desktop says it inside Flatpak (where the
//!   host's GNOME keys cannot be read) and on desktops other than GNOME;
//! - GTK's own `HighContrast` or `HighContrastInverse` theme, chosen in the
//!   user's GTK settings or with `GTK_THEME`, as it was before the skin
//!   put GTK's default theme under itself ([`theme_contrast`]).
//!
//! Before, only GNOME's key counted, so a Flatpak install and a desktop
//! that chose a `HighContrast` theme drew the normal skin.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::integration::DESKTOP_PORTAL_NAME;

const ACCESSIBILITY_SCHEMA: &str = "org.gnome.desktop.a11y.interface";

/// GNOME's key behind Settings > Accessibility > High Contrast.
pub(crate) const HIGH_CONTRAST_KEY: &str = "high-contrast";

/// The portal's key for the contrast the desktop asks for: 0 no
/// preference, 1 higher contrast.
pub(crate) const PORTAL_CONTRAST_KEY: &str = "contrast";

/// How much contrast the desktop asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Contrast {
    /// The skin as designed.
    #[default]
    Normal,
    /// Stronger outlines for the selected sidebar row and keyboard focus.
    High,
}

impl Contrast {
    /// High when `asked`.
    fn from_asked(asked: bool) -> Self {
        if asked {
            Contrast::High
        } else {
            Contrast::Normal
        }
    }
}

/// What asks for more contrast now.
struct Sources {
    /// GNOME's accessibility settings, when the schema is installed.
    settings: Option<gio::Settings>,
    /// Whether the portal's contrast asks for more.
    portal: Cell<bool>,
    /// Whether a `SettingChanged` signal for the contrast arrived, which
    /// is newer than the first read's reply.
    portal_signal_seen: Cell<bool>,
    /// Whether the GTK theme the user chose is a high-contrast one.
    theme: bool,
    /// Hears every later change.
    on_change: Box<dyn Fn(Contrast)>,
    /// The portal's `SettingChanged` subscription, while it lives.
    subscription: std::cell::RefCell<Option<gio::SignalSubscription>>,
}

impl Sources {
    fn contrast(&self) -> Contrast {
        let gnome = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.boolean(HIGH_CONTRAST_KEY));
        Contrast::from_asked(gnome || self.portal.get() || self.theme)
    }

    /// Records the portal's `value` from a `SettingChanged` signal.
    fn record_portal_change(&self, value: &glib::Variant) {
        self.portal_signal_seen.set(true);
        self.record_portal(value);
    }

    /// Records the portal's `value` read at the start, unless a signal
    /// came while the read was on its way: its value is newer, as
    /// `SystemScheme::follow_portal` keeps it for the color scheme.
    fn record_portal_reply(&self, value: &glib::Variant) {
        if !self.portal_signal_seen.get() {
            self.record_portal(value);
        }
    }

    /// Records the portal's `value` and reports the contrast when it
    /// changed.
    fn record_portal(&self, value: &glib::Variant) {
        let Some(asked) = contrast_from_portal(value) else {
            return;
        };
        let before = self.contrast();
        self.portal.set(asked);
        let now = self.contrast();
        if now != before {
            (self.on_change)(now);
        }
    }
}

/// Follows the desktop's contrast while it lives.
pub(crate) struct ContrastSetting {
    sources: Rc<Sources>,
}

impl std::fmt::Debug for ContrastSetting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContrastSetting")
            .field("contrast", &self.contrast())
            .finish_non_exhaustive()
    }
}

impl ContrastSetting {
    /// Starts following GNOME's key and the desktop portal; `theme` is
    /// what the user's GTK theme asks for ([`theme_contrast`]), and
    /// `on_change` hears every later change.
    pub(crate) fn watch(theme: Contrast, on_change: impl Fn(Contrast) + 'static) -> Self {
        let setting = Self::with_portal(theme, on_change, None);
        let sources = Rc::downgrade(&setting.sources);
        glib::MainContext::default().spawn_local(async move {
            let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
                return;
            };
            super::system::portal_ready(&connection).await;
            follow_portal(sources, &connection, DESKTOP_PORTAL_NAME).await;
        });
        setting
    }

    /// A setting that follows GNOME's key now and, given `portal`, the
    /// portal of that name on the session bus.
    fn with_portal(theme: Contrast, on_change: impl Fn(Contrast) + 'static, portal: Option<&str>) -> Self {
        let settings = accessibility_settings();
        let sources = Rc::new(Sources {
            settings,
            portal: Cell::new(false),
            portal_signal_seen: Cell::new(false),
            theme: theme == Contrast::High,
            on_change: Box::new(on_change),
            subscription: std::cell::RefCell::default(),
        });
        if let Some(settings) = &sources.settings {
            let weak = Rc::downgrade(&sources);
            settings.connect_changed(Some(HIGH_CONTRAST_KEY), move |_, _| {
                if let Some(sources) = weak.upgrade() {
                    (sources.on_change)(sources.contrast());
                }
            });
        }
        if let Some(portal) = portal {
            let weak = Rc::downgrade(&sources);
            let portal = portal.to_owned();
            glib::MainContext::default().spawn_local(async move {
                let Ok(connection) = gio::bus_get_future(gio::BusType::Session).await else {
                    return;
                };
                follow_portal(weak, &connection, &portal).await;
            });
        }
        Self { sources }
    }

    /// The contrast the desktop asks for now.
    pub(crate) fn contrast(&self) -> Contrast {
        self.sources.contrast()
    }
}

/// Follows the contrast of the portal that `portal` names on
/// `connection`: every change, and the current value.
async fn follow_portal(sources: Weak<Sources>, connection: &gio::DBusConnection, portal: &str) {
    let weak = sources.clone();
    let subscription = connection.subscribe_to_signal(
        Some(portal),
        Some(super::system::PORTAL_SETTINGS),
        Some("SettingChanged"),
        Some(ox_core::integration::DESKTOP_PORTAL_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let Some(sources) = weak.upgrade() else {
                return;
            };
            let Some((namespace, key, value)) = signal.parameters.get::<(String, String, glib::Variant)>()
            else {
                return;
            };
            if namespace == super::system::APPEARANCE_NAMESPACE && key == PORTAL_CONTRAST_KEY {
                sources.record_portal_change(&value);
            }
        },
    );
    if let Some(sources) = sources.upgrade() {
        sources.subscription.replace(Some(subscription));
    }
    let value = super::system::read_portal(
        connection,
        portal,
        super::system::APPEARANCE_NAMESPACE,
        PORTAL_CONTRAST_KEY,
    )
    .await;
    if let (Some(sources), Some(value)) = (sources.upgrade(), value) {
        sources.record_portal_reply(&value);
    }
}

/// Whether the portal's contrast `value` asks for more contrast (1), or
/// `None` for a value of the wrong type.
fn contrast_from_portal(value: &glib::Variant) -> Option<bool> {
    value.get::<u32>().map(|value| value == 1)
}

/// What the user's GTK theme asks for on `display`, read before the skin
/// puts GTK's default theme under itself: `GTK_THEME` wins, as in GTK,
/// over the theme name of the GTK settings.
pub(crate) fn theme_contrast(display: &gdk::Display) -> Contrast {
    let from_environment = std::env::var("GTK_THEME").ok();
    let from_settings = gtk::Settings::for_display(display)
        .gtk_theme_name()
        .map(|name| name.to_string());
    let name = from_environment.or(from_settings).unwrap_or_default();
    Contrast::from_asked(is_high_contrast_theme(&name))
}

/// Whether the GTK theme `name` is a high-contrast one: GTK's
/// `HighContrast` and `HighContrastInverse`, and its `Default-hc` and
/// `Default-hc-dark`; a `:dark` variant suffix is ignored.
fn is_high_contrast_theme(name: &str) -> bool {
    let name = name.split(':').next().unwrap_or_default().to_lowercase();
    name.starts_with("highcontrast") || name.ends_with("-hc") || name.ends_with("-hc-dark")
}

/// GNOME's accessibility settings, when the schema and its
/// [`HIGH_CONTRAST_KEY`] exist.
pub(crate) fn accessibility_settings() -> Option<gio::Settings> {
    let settings = super::desktop_settings(ACCESSIBILITY_SCHEMA)?;
    let has_key = settings.settings_schema()?.has_key(HIGH_CONTRAST_KEY);
    has_key.then_some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::wait_until;
    use crate::test_support::portal::SettingsPortal;

    /// The portal's contrast asks for more contrast at 1 and none at 0,
    /// from the start and when the desktop changes it, as inside Flatpak,
    /// where GNOME's key cannot be read; a `HighContrast` GTK theme asks for
    /// it whatever the portal says.
    ///
    /// parity: ACC-011
    #[gtk::test]
    fn the_portal_and_a_high_contrast_theme_ask_for_more_contrast() {
        let appearance = super::super::system::APPEARANCE_NAMESPACE;
        let portal = SettingsPortal::serving(&[(appearance, PORTAL_CONTRAST_KEY, 1_u32.to_variant())]);
        let heard = Rc::new(Cell::new(Contrast::Normal));
        let hears = Rc::clone(&heard);
        let setting = ContrastSetting::with_portal(
            Contrast::Normal,
            move |contrast| hears.set(contrast),
            Some(&portal.name()),
        );
        wait_until("the portal's contrast", || setting.contrast() == Contrast::High);
        assert_eq!(heard.get(), Contrast::High, "the change is heard");
        portal.change(appearance, PORTAL_CONTRAST_KEY, 0_u32.to_variant());
        wait_until("the desktop's normal contrast", || {
            setting.contrast() == Contrast::Normal
        });
        assert_eq!(heard.get(), Contrast::Normal);

        let themed = ContrastSetting::with_portal(Contrast::High, |_| {}, Some(&portal.name()));
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(200));
        assert_eq!(themed.contrast(), Contrast::High);
    }

    /// A change the portal signals while the first read is on its way
    /// wins over that read's older reply, as the color scheme's does: high
    /// contrast stays as the desktop last said, not as it was a moment
    /// before.
    ///
    /// parity: ACC-011
    #[gtk::test]
    fn a_signalled_change_wins_over_an_older_first_reply() {
        let setting = ContrastSetting::with_portal(Contrast::Normal, |_| {}, None);
        let sources = &setting.sources;
        let gnome = sources
            .settings
            .as_ref()
            .is_some_and(|settings| settings.boolean(HIGH_CONTRAST_KEY));
        if gnome {
            return; // GNOME's own key asks for it whatever the portal says.
        }
        sources.record_portal_change(&1_u32.to_variant());
        sources.record_portal_reply(&0_u32.to_variant());
        assert_eq!(setting.contrast(), Contrast::High, "the signal is newer");
        sources.record_portal_change(&0_u32.to_variant());
        assert_eq!(setting.contrast(), Contrast::Normal, "a later signal counts");
    }
}
