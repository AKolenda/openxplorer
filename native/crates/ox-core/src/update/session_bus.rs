// SPDX-License-Identifier: AGPL-3.0-only
//! The running-instance guard's calls on the D-Bus session bus. Ports
//! `Session.call`, `Session.owner`, `Session.running` and the quit call of
//! `Session.stop` in `desktop/runtime_guard.py`.
//!
//! A `GApplication` exports its actions as `org.gtk.Actions` on its object
//! path, so the guard reads `runtime-info` with `Describe` and quits with
//! `Activate`, the same calls GTK itself makes.

use std::collections::HashMap;

use gio::prelude::*;

use super::{InstanceBus, InstanceError, RuntimeIdentity};

/// How long one call may take, in milliseconds.
const CALL_TIMEOUT_MS: i32 = 3000;

/// The bus daemon.
const BUS_NAME: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// The error the bus daemon answers `GetNameOwner` with when nobody owns
/// the name.
const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";

/// The interface `GApplication` exports its actions on.
const ACTIONS_INTERFACE: &str = "org.gtk.Actions";

/// The stateful action whose state is the instance's [`RuntimeIdentity`].
pub const RUNTIME_INFO_ACTION: &str = "runtime-info";

/// The action that quits an instance safely.
pub const QUIT_ACTION: &str = "quit";

/// An application on the session bus, found by its application ID.
#[derive(Debug, Clone)]
pub struct SessionBus {
    connection: gio::DBusConnection,
    application_id: String,
    object_path: String,
}

impl SessionBus {
    /// The application `application_id` on the user's session bus.
    ///
    /// # Errors
    ///
    /// [`InstanceError::Bus`] if there is no session bus.
    pub fn connect(application_id: &str) -> Result<Self, InstanceError> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        Ok(Self::on_connection(connection, application_id))
    }

    /// The application `application_id` on `connection`.
    pub fn on_connection(connection: gio::DBusConnection, application_id: &str) -> Self {
        Self {
            connection,
            application_id: application_id.to_owned(),
            object_path: application_object_path(application_id),
        }
    }

    /// The connection, for `--diagnose` reports that ask the bus more.
    pub fn connection(&self) -> &gio::DBusConnection {
        &self.connection
    }

    /// Calls `method` without starting a service for it, as the Python
    /// guard does with `NO_AUTO_START`.
    fn call(&self, target: Call<'_>, parameters: &glib::Variant) -> Result<glib::Variant, glib::Error> {
        self.connection.call_sync(
            Some(target.destination),
            target.path,
            target.interface,
            target.method,
            Some(parameters),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            CALL_TIMEOUT_MS,
            gio::Cancellable::NONE,
        )
    }

    /// `org.gtk.Actions.<method>` on the instance at `owner`.
    fn actions_call<'a>(&'a self, owner: &'a str, method: &'a str) -> Call<'a> {
        Call {
            destination: owner,
            path: &self.object_path,
            interface: ACTIONS_INTERFACE,
            method,
        }
    }
}

impl InstanceBus for SessionBus {
    fn owner(&self) -> Result<Option<String>, InstanceError> {
        let target = Call {
            destination: BUS_NAME,
            path: BUS_PATH,
            interface: BUS_NAME,
            method: "GetNameOwner",
        };
        match self.call(target, &(self.application_id.as_str(),).to_variant()) {
            Ok(reply) => Ok(reply.try_get::<(String,)>().ok().map(|(owner,)| owner)),
            Err(error) if gio::DBusError::remote_error(&error).as_deref() == Some(NAME_HAS_NO_OWNER) => {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn reported_identity(&self, owner: &str) -> Option<RuntimeIdentity> {
        let target = self.actions_call(owner, "Describe");
        let reply = self.call(target, &(RUNTIME_INFO_ACTION,).to_variant()).ok()?;
        identity_from_description(&reply)
    }

    fn request_quit(&self, owner: &str) -> Result<(), InstanceError> {
        let target = self.actions_call(owner, "Activate");
        let parameter: Vec<glib::Variant> = Vec::new();
        let platform_data: HashMap<String, glib::Variant> = HashMap::new();
        self.call(target, &(QUIT_ACTION, parameter, platform_data).to_variant())?;
        Ok(())
    }
}

/// Reads the identity from a `Describe` reply, `((bgav))`: whether the
/// action is enabled, its parameter type and its state, a one-element
/// array holding the JSON text. Anything else, including an error for an
/// unknown action, means an older release without `runtime-info`.
fn identity_from_description(reply: &glib::Variant) -> Option<RuntimeIdentity> {
    if reply.type_().as_str() != "((bgav))" {
        return None;
    }
    let state = reply.child_value(0).child_value(2);
    let boxed_state = state.iter().next()?;
    let state_text = boxed_state.as_variant()?;
    RuntimeIdentity::from_action_state(state_text.str()?)
}

/// Where one D-Bus method call goes.
#[derive(Debug, Clone, Copy)]
struct Call<'a> {
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
    method: &'a str,
}

/// The object path `GApplication` exports an application ID on, as
/// `g_application_id_to_object_path`: `/` first, `.` becomes `/`, `-`
/// becomes `_`, and a path element starting with a digit gets a leading
/// `_`.
pub fn application_object_path(application_id: &str) -> String {
    let elements: Vec<String> = application_id.split('.').map(object_path_element).collect();
    format!("/{}", elements.join("/"))
}

/// One element of [`application_object_path`].
fn object_path_element(part: &str) -> String {
    let underscored = part.replace('-', "_");
    if underscored.starts_with(|first: char| first.is_ascii_digit()) {
        format!("_{underscored}")
    } else {
        underscored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_ids_become_gapplication_object_paths() {
        assert_eq!(
            application_object_path("io.winspace.Development"),
            "/io/winspace/Development"
        );
        assert_eq!(
            application_object_path("io.winspace.Development.Native"),
            "/io/winspace/Development/Native"
        );
        assert_eq!(
            application_object_path("org.example.2nd-app"),
            "/org/example/_2nd_app"
        );
    }
}
