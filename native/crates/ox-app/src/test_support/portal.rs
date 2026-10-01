// SPDX-License-Identifier: AGPL-3.0-only
//! Fake desktop portals: a second connection to the test's private session
//! bus, as the portal process would have, and a guard that exports one
//! portal interface there until the test ends. Tests call the fake under
//! its unique name, so no real portal is reached and no well-known name is
//! taken from other tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::DESKTOP_PORTAL_PATH;

/// The interface of the Settings portal.
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";

/// The part of `org.freedesktop.portal.Settings` the app uses.
pub(crate) const SETTINGS_XML: &str = r#"<node>
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

/// A new connection to the test's session bus, as another process would
/// have.
pub(crate) fn session_connection() -> gio::DBusConnection {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .expect("the tests run on a private session bus");
    let flags =
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
    gio::DBusConnection::for_address_sync(&address, flags, None, gio::Cancellable::NONE)
        .expect("connect to the session bus")
}

/// One portal interface exported at the portal's object path on its own
/// connection; dropping it unexports the interface and closes the
/// connection.
pub(crate) struct ExportedPortal {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
}

impl ExportedPortal {
    /// Exports the interface `interface_name` that `xml` declares on a new
    /// connection, answering its method calls with `method_call`.
    pub(crate) fn export<F>(xml: &str, interface_name: &str, method_call: F) -> Self
    where
        F: Fn(
                gio::DBusConnection,
                Option<&str>,
                &str,
                Option<&str>,
                &str,
                glib::Variant,
                gio::DBusMethodInvocation,
            ) + 'static,
    {
        let connection = session_connection();
        let node = gio::DBusNodeInfo::for_xml(xml).expect("the interface XML is valid");
        let interface = node
            .lookup_interface(interface_name)
            .expect("the XML declares the portal interface");
        let registration = connection
            .register_object(DESKTOP_PORTAL_PATH, &interface)
            .method_call(method_call)
            .build()
            .expect("the fake portal can be exported");
        Self {
            connection,
            registration: Some(registration),
        }
    }

    /// The connection the portal answers on, for the signals it emits.
    pub(crate) fn connection(&self) -> &gio::DBusConnection {
        &self.connection
    }

    /// The bus name the portal answers under.
    pub(crate) fn name(&self) -> String {
        self.connection
            .unique_name()
            .expect("a bus connection has a unique name")
            .to_string()
    }
}

impl Drop for ExportedPortal {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _ = self.connection.unregister_object(registration);
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}

/// The values a [`SettingsPortal`] serves, by namespace and key.
type PortalValues = Rc<RefCell<HashMap<(String, String), glib::Variant>>>;

/// A Settings portal that answers `ReadOne` from the values it was given
/// and refuses every other setting, as a portal does for keys it lacks.
pub(crate) struct SettingsPortal {
    portal: ExportedPortal,
    values: PortalValues,
}

impl SettingsPortal {
    /// A portal serving `values`, each `(namespace, key, value)`.
    pub(crate) fn serving(values: &[(&str, &str, glib::Variant)]) -> Self {
        let values: PortalValues = Rc::new(RefCell::new(
            values
                .iter()
                .map(|(namespace, key, value)| (((*namespace).to_owned(), (*key).to_owned()), value.clone()))
                .collect(),
        ));
        let answers = Rc::clone(&values);
        let portal = ExportedPortal::export(
            SETTINGS_XML,
            PORTAL_SETTINGS,
            move |_, _, _, _, method, parameters, invocation| {
                let setting = parameters.get::<(String, String)>().unwrap_or_default();
                let value = (method == "ReadOne")
                    .then(|| answers.borrow().get(&setting).cloned())
                    .flatten();
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
        Self { portal, values }
    }

    /// The bus name the portal answers under.
    pub(crate) fn name(&self) -> String {
        self.portal.name()
    }

    /// Changes `key` in `namespace` and announces it with
    /// `SettingChanged`, as the portal does when the desktop changes.
    pub(crate) fn change(&self, namespace: &str, key: &str, value: glib::Variant) {
        self.values
            .borrow_mut()
            .insert((namespace.to_owned(), key.to_owned()), value.clone());
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
