// SPDX-License-Identifier: AGPL-3.0-only
//! Fake desktop portals: a second connection to the test's private session
//! bus, as the portal process would have, and a guard that exports one
//! portal interface there until the test ends. Tests call the fake under
//! its unique name, so no real portal is reached and no well-known name is
//! taken from other tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::DESKTOP_PORTAL_PATH;

/// The interface of the Settings portal.
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";

/// The interface of the Email portal.
const PORTAL_EMAIL: &str = "org.freedesktop.portal.Email";

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
        Self::export_with_version(xml, interface_name, None, method_call)
    }

    /// Exports the interface as [`Self::export`] does, serving `version`
    /// as its `version` property when there is one.
    pub(crate) fn export_with_version<F>(
        xml: &str,
        interface_name: &str,
        version: Option<u32>,
        method_call: F,
    ) -> Self
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
        let mut builder = connection
            .register_object(DESKTOP_PORTAL_PATH, &interface)
            .method_call(method_call);
        if let Some(version) = version {
            builder = builder.property(move |_, _, _, _, _| version.to_variant());
        }
        let registration = builder.build().expect("the fake portal can be exported");
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

/// `org.freedesktop.portal.Email` as version 4 declares it.
const EMAIL_XML: &str = r#"<node>
  <interface name="org.freedesktop.portal.Email">
    <method name="ComposeEmail">
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="o" name="handle" direction="out"/>
    </method>
    <property name="version" type="u" access="read"/>
  </interface>
</node>"#;

/// What an [`EmailPortal`] was asked for: the files whose descriptors
/// came with the call, and the names of the options given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComposedEmail {
    pub(crate) attachments: Vec<PathBuf>,
    pub(crate) options: Vec<String>,
}

/// An Email portal that records each `ComposeEmail` and answers it with
/// a fixed `Response` code, as a desktop's portal does once its email
/// app started (0) or could not start (2).
pub(crate) struct EmailPortal {
    portal: ExportedPortal,
    calls: Rc<RefCell<Vec<ComposedEmail>>>,
}

impl EmailPortal {
    /// A portal answering every request with `response`: 0 for an email
    /// app that opened, 1 for the user cancelling, 2 for a failure.
    pub(crate) fn answering(response: u32) -> Self {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&calls);
        let portal = ExportedPortal::export_with_version(
            EMAIL_XML,
            PORTAL_EMAIL,
            Some(4),
            move |connection, _, _, _, _, parameters, invocation| {
                let options = parameters.child_value(1);
                let mut names: Vec<String> = options
                    .iter()
                    .filter_map(|entry| entry.child_value(0).get::<String>())
                    .collect();
                names.sort();
                let options = glib::VariantDict::new(Some(&options));
                let fds = invocation.message().unix_fd_list();
                let attachments = options
                    .lookup_value("attachment_fds", None)
                    .map(|handles| {
                        handles
                            .iter()
                            .filter_map(|handle| handle.get::<glib::variant::Handle>())
                            .filter_map(|handle| fds.as_ref()?.get(handle.0).ok())
                            .filter_map(|fd| {
                                std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                recorded.borrow_mut().push(ComposedEmail {
                    attachments,
                    options: names,
                });
                let token = options
                    .lookup_value("handle_token", None)
                    .and_then(|token| token.get::<String>())
                    .unwrap_or_default();
                let handle = format!("{DESKTOP_PORTAL_PATH}/request/test/{token}");
                let path = glib::variant::ObjectPath::try_from(handle.clone()).expect("a valid path");
                invocation.return_value(Some(&(path,).to_variant()));
                let answer = (response, glib::VariantDict::new(None)).to_variant();
                connection
                    .emit_signal(
                        None,
                        &handle,
                        "org.freedesktop.portal.Request",
                        "Response",
                        Some(&answer),
                    )
                    .expect("the answer is sent");
            },
        );
        Self { portal, calls }
    }

    /// The bus name the portal answers under.
    pub(crate) fn name(&self) -> String {
        self.portal.name()
    }

    /// The requests so far, in order.
    pub(crate) fn calls(&self) -> Vec<ComposedEmail> {
        self.calls.borrow().clone()
    }
}
