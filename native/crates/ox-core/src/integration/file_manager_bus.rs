// SPDX-License-Identifier: AGPL-3.0-only
//! The standard `org.freedesktop.FileManager1` session-bus service, which
//! browsers and other applications call for "Show in folder".
//!
//! Ports `desktop/filemanager_bus.py`. The service is registered only
//! after the user enables "Show in folder" (INT-013). Its three methods
//! accept 1 to 100 locations, which are checked by
//! [`FileManagerRequest`] and handed to the app with the caller's startup
//! ID, so the window can take focus (INT-023). The service lives on the
//! main thread: `GDBus` calls its handlers from the main loop.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use gio::prelude::*;

use super::file_manager_request::FileManagerRequest;

/// The well-known bus name of the file manager service.
pub const BUS_NAME: &str = "org.freedesktop.FileManager1";

/// The object path of the file manager service.
pub const OBJECT_PATH: &str = "/org/freedesktop/FileManager1";

/// The interface name, which is the same as the bus name.
const INTERFACE_NAME: &str = "org.freedesktop.FileManager1";

/// The interface as `filemanager_bus.py` declares it.
const INTERFACE_XML: &str = r#"<node><interface name="org.freedesktop.FileManager1">
<method name="ShowFolders"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
<method name="ShowItems"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
<method name="ShowItemProperties"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
</interface></node>"#;

/// The longest startup ID handed to the app, in characters.
const MAX_STARTUP_ID_CHARS: usize = 4096;

/// The D-Bus error for a request that was refused.
const INVALID_ARGS_ERROR: &str = "org.freedesktop.DBus.Error.InvalidArgs";

/// The D-Bus error for a request the app could not carry out.
const FAILED_ERROR: &str = "org.freedesktop.DBus.Error.Failed";

/// How long a status query of the bus daemon may take, in milliseconds.
const STATUS_TIMEOUT_MS: i32 = 1000;

/// The longest process name shown as the owner, in characters.
const MAX_OWNER_LABEL_CHARS: usize = 80;

/// The owner label when the app owns the name.
const OPENXPLORER_LABEL: &str = "OpenXplorer";

/// The app could not carry out a request. Returned to the caller as
/// `org.freedesktop.DBus.Error.Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("OpenXplorer could not open the requested location.")]
pub struct RequestNotOpened;

/// The service could not be registered on the bus.
#[derive(Debug, thiserror::Error)]
#[error("Show in folder could not be registered: {0}")]
pub struct RegistrationFailed(#[from] glib::Error);

/// Who owns `org.freedesktop.FileManager1` (INT-016).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusStatus {
    /// The unique bus name of the owner, if the name has one.
    pub owner: Option<String>,
    /// The app's own name when it owns the bus name, otherwise the owning
    /// process's name when it can be read; empty when unknown.
    pub owner_label: String,
    /// The app owns the service, not merely the MIME default.
    pub is_owned_by_openxplorer: bool,
}

/// What a name-ownership callback reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Acquired,
    Lost,
}

/// What the app does with a checked request and the caller's startup ID.
type RequestHandler = dyn Fn(FileManagerRequest, String) -> Result<(), RequestNotOpened>;

/// The app's `org.freedesktop.FileManager1` service on one bus
/// connection.
pub struct FileManagerBus {
    connection: gio::DBusConnection,
    handler: Rc<RequestHandler>,
    on_ownership_change: Rc<dyn Fn()>,
    /// Shared with the name-ownership callbacks, which run later on the
    /// main loop.
    is_owned: Rc<Cell<bool>>,
    owner_id: Option<gio::OwnerId>,
    registration: Option<gio::RegistrationId>,
}

impl fmt::Debug for FileManagerBus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileManagerBus")
            .field("connection", &self.connection)
            .field("is_owned", &self.is_owned.get())
            .field("is_enabled", &self.owner_id.is_some())
            .finish_non_exhaustive()
    }
}

impl FileManagerBus {
    /// A service on `connection` that is not registered yet. `handler`
    /// receives every valid request; `on_ownership_change` runs whenever
    /// the name is acquired, lost or released, so the Settings status can
    /// be refreshed.
    pub fn new(
        connection: gio::DBusConnection,
        handler: impl Fn(FileManagerRequest, String) -> Result<(), RequestNotOpened> + 'static,
        on_ownership_change: impl Fn() + 'static,
    ) -> Self {
        Self {
            connection,
            handler: Rc::new(handler),
            on_ownership_change: Rc::new(on_ownership_change),
            is_owned: Rc::new(Cell::new(false)),
            owner_id: None,
            registration: None,
        }
    }

    /// True while the app owns the bus name.
    pub fn is_owned(&self) -> bool {
        self.is_owned.get()
    }

    /// Exports the object and asks for the bus name. Enabling again does
    /// nothing. Ownership arrives later, on the main loop.
    ///
    /// # Errors
    ///
    /// [`RegistrationFailed`] when the object cannot be exported.
    pub fn enable(&mut self) -> Result<(), RegistrationFailed> {
        if self.owner_id.is_some() {
            return Ok(());
        }
        self.registration = Some(self.register_object()?);
        // Safety rule "an explicit opt-in is not silently given up"
        // (`enable` in filemanager_bus.py): REPLACE takes the name from an
        // owner that allows it, and OpenXplorer does not allow replacement
        // itself. Another file manager is never stopped.
        let acquired = self.ownership_callback(Ownership::Acquired);
        let lost = self.ownership_callback(Ownership::Lost);
        let owner_id = gio::bus_own_name_on_connection(
            &self.connection,
            BUS_NAME,
            gio::BusNameOwnerFlags::REPLACE,
            move |_, _| acquired(),
            move |_, _| lost(),
        );
        self.owner_id = Some(owner_id);
        Ok(())
    }

    /// Releases the name and removes the object.
    pub fn disable(&mut self) {
        self.release();
        self.is_owned.set(false);
        (self.on_ownership_change)();
    }

    /// Reads who owns the name from the bus daemon (INT-016). Failures
    /// leave the fields empty.
    ///
    /// Safety rule "only look, never start or stop another file manager"
    /// (`status` in `filemanager_bus.py`): the queries go to the bus daemon
    /// with `NO_AUTO_START`, so asking never activates a service.
    pub async fn status(&self) -> BusStatus {
        let mut status = BusStatus::default();
        let Some(owner) = self.ask_bus_daemon("GetNameOwner", BUS_NAME).await else {
            return status;
        };
        let owner_name = owner.get::<(String,)>().map(|(name,)| name);
        let is_ours = owner_name.as_deref() == self.connection.unique_name().as_deref();
        status.is_owned_by_openxplorer = is_ours && self.is_owned();
        status.owner_label = if status.is_owned_by_openxplorer {
            OPENXPLORER_LABEL.to_owned()
        } else {
            self.process_name_of(owner_name.as_deref())
                .await
                .unwrap_or_default()
        };
        status.owner = owner_name;
        status
    }

    /// Exports the interface at [`OBJECT_PATH`].
    fn register_object(&self) -> Result<gio::RegistrationId, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(INTERFACE_XML)?;
        let interface = node
            .lookup_interface(INTERFACE_NAME)
            .expect("INTERFACE_XML declares the FileManager1 interface");
        let handler = Rc::clone(&self.handler);
        self.connection
            .register_object(OBJECT_PATH, &interface)
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                answer_call(handler.as_ref(), method, &parameters, invocation);
            })
            .build()
    }

    /// A callback that records `ownership` and notifies the app. It holds
    /// only the shared flag and the notifier, not `self`.
    fn ownership_callback(&self, ownership: Ownership) -> impl Fn() + 'static {
        let is_owned = Rc::clone(&self.is_owned);
        let notify = Rc::clone(&self.on_ownership_change);
        move || {
            is_owned.set(ownership == Ownership::Acquired);
            notify();
        }
    }

    /// Stops asking for the name and removes the object, if either exists.
    fn release(&mut self) {
        if let Some(owner_id) = self.owner_id.take() {
            gio::bus_unown_name(owner_id);
        }
        if let Some(registration) = self.registration.take() {
            // The ID came from this connection and was not unregistered
            // yet, so this cannot fail.
            let _ = self.connection.unregister_object(registration);
        }
    }

    /// Calls `method` of the bus daemon with one string argument.
    async fn ask_bus_daemon(&self, method: &str, argument: &str) -> Option<glib::Variant> {
        let reply = self.connection.call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            Some(&(argument,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            STATUS_TIMEOUT_MS,
        );
        reply.await.ok()
    }

    /// The process name (`/proc/<pid>/comm`) of the connection `owner`.
    async fn process_name_of(&self, owner: Option<&str>) -> Option<String> {
        let reply = self.ask_bus_daemon("GetConnectionUnixProcessID", owner?).await?;
        let (process_id,) = reply.get::<(u32,)>()?;
        if process_id == 0 {
            return None;
        }
        // procfs files are generated in memory, so this read never waits
        // on a disk.
        let name = std::fs::read_to_string(format!("/proc/{process_id}/comm")).ok()?;
        Some(name.trim().chars().take(MAX_OWNER_LABEL_CHARS).collect())
    }
}

impl Drop for FileManagerBus {
    /// Releases the name and the object, so a dropped service never keeps
    /// answering requests for a window that is gone.
    fn drop(&mut self) {
        self.release();
    }
}

/// Checks one call and hands it to the app, then answers the caller.
///
/// Safety rule "locations are data, never commands"
/// (`call` in `filemanager_bus.py`): only a checked [`FileManagerRequest`]
/// reaches the app; nothing from the call is evaluated or executed.
fn answer_call(
    handler: &RequestHandler,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    // GDBus has already checked the argument types against INTERFACE_XML.
    let Some((locations, startup_id)) = parameters.get::<(Vec<String>, String)>() else {
        invocation.return_dbus_error(INVALID_ARGS_ERROR, "Expected 1–100 file locations.");
        return;
    };
    let request = match FileManagerRequest::from_method_name(method, &locations) {
        Ok(request) => request,
        Err(refusal) => {
            invocation.return_dbus_error(INVALID_ARGS_ERROR, &refusal.to_string());
            return;
        }
    };
    let startup_id = startup_id.chars().take(MAX_STARTUP_ID_CHARS).collect();
    match handler(request, startup_id) {
        Ok(()) => invocation.return_value(None),
        Err(failure) => invocation.return_dbus_error(FAILED_ERROR, &failure.to_string()),
    }
}
