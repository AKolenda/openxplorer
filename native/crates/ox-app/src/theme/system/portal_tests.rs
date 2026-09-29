// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's colour scheme as the Settings portal reports it, which
//! decides inside Flatpak. A fake portal on a second connection to the
//! test's private session bus answers `ReadOne` and emits
//! `SettingChanged` under its unique name, so no real portal is reached
//! and no well-known name is taken from other tests.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::test_support::harness::wait_until;
use crate::test_support::portal::{session_connection, ExportedPortal};

/// The part of `org.freedesktop.portal.Settings` the app uses.
const SETTINGS_XML: &str = r#"<node>
  <interface name="org.freedesktop.portal.Settings">
    <method name="ReadOne">
      <arg type="s" name="namespace" direction="in"/>
      <arg type="s" name="key" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <signal name="SettingChanged">
      <arg type="s" name="namespace"/>
      <arg type="s" name="key"/>
      <arg type="v" name="value"/>
    </signal>
  </interface>
</node>"#;

/// A Settings portal that holds one colour scheme and, as GNOME-based
/// portals do, may serve a GTK theme name, exported at the portal's
/// object path on its own connection.
struct FakePortal {
    portal: ExportedPortal,
    color_scheme: Rc<Cell<u32>>,
    gtk_theme: Rc<RefCell<Option<String>>>,
}

impl FakePortal {
    /// A portal that answers `color_scheme` (0 no preference, 1 dark,
    /// 2 light) and refuses every other setting, as a non-GNOME one does.
    fn start(color_scheme: u32) -> Self {
        Self::with_gtk_theme(color_scheme, None)
    }

    /// A portal that answers `color_scheme` and, when there is one,
    /// `gtk_theme` for GNOME's `gtk-theme` key.
    fn with_gtk_theme(color_scheme: u32, gtk_theme: Option<&str>) -> Self {
        let color_scheme = Rc::new(Cell::new(color_scheme));
        let gtk_theme = Rc::new(RefCell::new(gtk_theme.map(str::to_owned)));
        let (scheme_answer, theme_answer) = (Rc::clone(&color_scheme), Rc::clone(&gtk_theme));
        let portal = ExportedPortal::export(
            SETTINGS_XML,
            PORTAL_SETTINGS,
            move |_, _, _, _, method, parameters, invocation| {
                let setting = parameters.get::<(String, String)>().unwrap_or_default();
                let value = match (method, setting.0.as_str(), setting.1.as_str()) {
                    ("ReadOne", APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY) => {
                        Some(scheme_answer.get().to_variant())
                    }
                    ("ReadOne", INTERFACE_SCHEMA, GTK_THEME_KEY) => {
                        theme_answer.borrow().as_deref().map(ToVariant::to_variant)
                    }
                    _ => None,
                };
                match value {
                    // `(v)`: a tuple boxes the value it holds.
                    Some(value) => invocation.return_value(Some(&(value,).to_variant())),
                    None => invocation.return_dbus_error(
                        "org.freedesktop.portal.Error.NotFound",
                        "Requested setting not found",
                    ),
                }
            },
        );
        Self {
            portal,
            color_scheme,
            gtk_theme,
        }
    }

    /// The bus name the portal answers under.
    fn name(&self) -> String {
        self.portal.name()
    }

    /// Changes the colour scheme and announces it, as the portal does when
    /// the desktop switches.
    fn switch_to(&self, color_scheme: u32) {
        self.color_scheme.set(color_scheme);
        self.announce(APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY, color_scheme.to_variant());
    }

    /// Changes the GTK theme name and announces it.
    fn switch_gtk_theme_to(&self, gtk_theme: &str) {
        self.gtk_theme.replace(Some(gtk_theme.to_owned()));
        self.announce(INTERFACE_SCHEMA, GTK_THEME_KEY, gtk_theme.to_variant());
    }

    /// Emits `SettingChanged` for `key` in `namespace`.
    fn announce(&self, namespace: &str, key: &str, value: glib::Variant) {
        let parameters = (namespace, key, value).to_variant();
        self.portal
            .connection()
            .emit_signal(
                None,
                DESKTOP_PORTAL_PATH,
                PORTAL_SETTINGS,
                "SettingChanged",
                Some(&parameters),
            )
            .expect("the signal is sent");
    }
}

/// The appearances a scheme reported, in order.
type Reported = Rc<RefCell<Vec<Appearance>>>;

/// A scheme inside Flatpak that follows `portal`, as [`SystemScheme::new`]
/// sets one up there, with GTK's preference at startup `gtk_fallback`,
/// and the appearances it reports.
fn follow_in_flatpak(gtk_fallback: Appearance, portal: &FakePortal) -> (Rc<SystemScheme>, Reported) {
    let reported = Reported::default();
    let recorder = Rc::clone(&reported);
    let scheme = Rc::new(SystemScheme::unwatched(
        gnome_interface_settings(Sandbox::Flatpak),
        gtk_fallback,
        Box::new(move |appearance| recorder.borrow_mut().push(appearance)),
    ));
    let client = session_connection();
    let name = portal.name();
    let watching = Rc::downgrade(&scheme);
    glib::spawn_future_local(async move { follow_portal(watching, &client, &name).await });
    (scheme, reported)
}

/// Inside Flatpak GNOME's keys are the runtime's defaults, so they never
/// decide, even where the schema is installed.
///
/// parity: LOOK-004
#[test]
fn inside_flatpak_gnome_keys_never_decide() {
    assert!(gnome_interface_settings(Sandbox::Flatpak).is_none());
}

/// Inside Flatpak a dark desktop, as the portal reports it, turns the
/// System theme dark at once, and each later switch follows.
///
/// parity: LOOK-004
#[gtk::test]
fn inside_flatpak_the_system_theme_follows_the_portal() {
    let portal = FakePortal::start(1);

    let (scheme, reported) = follow_in_flatpak(Appearance::Light, &portal);

    wait_until("the portal's dark scheme", || {
        reported.borrow().last() == Some(&Appearance::Dark)
    });
    assert_eq!(scheme.appearance(), Appearance::Dark);
    portal.switch_to(2);
    wait_until("the switch to light", || {
        reported.borrow().last() == Some(&Appearance::Light)
    });
    portal.switch_to(1);
    wait_until("the switch back to dark", || {
        scheme.appearance() == Appearance::Dark
    });
}

/// A portal without a preference leaves the GTK preference read at
/// startup in charge.
///
/// parity: LOOK-004
#[gtk::test]
fn a_portal_without_a_preference_keeps_the_startup_preference() {
    let portal = FakePortal::start(0);

    let (scheme, reported) = follow_in_flatpak(Appearance::Dark, &portal);

    wait_until("the portal's answer", || !reported.borrow().is_empty());
    assert_eq!(scheme.appearance(), Appearance::Dark);
}

/// While the portal's colour scheme has no preference, the GTK theme name
/// that GNOME-based portals serve decides, as GNOME's own key does on the
/// host (`ZorinBlue-Dark` asks for dark); a colour scheme with a
/// preference still wins over it.
///
/// parity: LOOK-004
#[gtk::test]
fn a_portal_without_a_preference_follows_the_gtk_theme_name() {
    let portal = FakePortal::with_gtk_theme(0, Some("ZorinBlue-Dark"));

    let (scheme, reported) = follow_in_flatpak(Appearance::Light, &portal);

    wait_until("the dark theme name", || {
        reported.borrow().last() == Some(&Appearance::Dark)
    });
    assert_eq!(scheme.appearance(), Appearance::Dark);
    portal.switch_gtk_theme_to("ZorinBlue-Light");
    wait_until("the switch to a light theme", || {
        reported.borrow().last() == Some(&Appearance::Light)
    });
    portal.switch_gtk_theme_to("ZorinBlue-Dark");
    wait_until("the switch back to a dark theme", || {
        scheme.appearance() == Appearance::Dark
    });
    portal.switch_to(2);
    wait_until("the colour scheme preferring light", || {
        scheme.appearance() == Appearance::Light
    });
}
