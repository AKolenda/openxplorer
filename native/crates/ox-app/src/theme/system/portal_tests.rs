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

/// A Settings portal that holds one colour scheme, exported at the
/// portal's object path on its own connection.
struct FakePortal {
    connection: gio::DBusConnection,
    color_scheme: Rc<Cell<u32>>,
    registration: Option<gio::RegistrationId>,
}

impl FakePortal {
    /// A portal that answers `color_scheme` (0 no preference, 1 dark,
    /// 2 light) and refuses every other setting, as the real one does.
    fn start(color_scheme: u32) -> Self {
        let connection = session_connection();
        let node = gio::DBusNodeInfo::for_xml(SETTINGS_XML).expect("the interface XML is valid");
        let interface = node
            .lookup_interface(PORTAL_SETTINGS)
            .expect("SETTINGS_XML declares the Settings interface");
        let color_scheme = Rc::new(Cell::new(color_scheme));
        let answered = Rc::clone(&color_scheme);
        let registration = connection
            .register_object(PORTAL_PATH, &interface)
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                let setting = parameters.get::<(String, String)>();
                let is_color_scheme = setting.is_some_and(|(namespace, key)| {
                    namespace == APPEARANCE_NAMESPACE && key == COLOR_SCHEME_KEY
                });
                if method == "ReadOne" && is_color_scheme {
                    // `(v)`: a tuple boxes the value it holds.
                    invocation.return_value(Some(&(answered.get().to_variant(),).to_variant()));
                } else {
                    invocation.return_dbus_error(
                        "org.freedesktop.portal.Error.NotFound",
                        "Requested setting not found",
                    );
                }
            })
            .build()
            .expect("the fake portal can be exported");
        Self {
            connection,
            color_scheme,
            registration: Some(registration),
        }
    }

    /// The bus name the portal answers under.
    fn name(&self) -> String {
        self.connection
            .unique_name()
            .expect("a bus connection has a unique name")
            .to_string()
    }

    /// Changes the colour scheme and announces it, as the portal does when
    /// the desktop switches.
    fn switch_to(&self, color_scheme: u32) {
        self.color_scheme.set(color_scheme);
        let parameters = (APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY, color_scheme.to_variant()).to_variant();
        self.connection
            .emit_signal(
                None,
                PORTAL_PATH,
                PORTAL_SETTINGS,
                "SettingChanged",
                Some(&parameters),
            )
            .expect("the signal is sent");
    }
}

impl Drop for FakePortal {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _ = self.connection.unregister_object(registration);
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}

/// A new connection to the test's session bus, as another process would
/// have.
fn session_connection() -> gio::DBusConnection {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .expect("the tests run on a private session bus");
    let flags =
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
    gio::DBusConnection::for_address_sync(&address, flags, None, gio::Cancellable::NONE)
        .expect("connect to the session bus")
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
