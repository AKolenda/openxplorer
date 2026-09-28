// SPDX-License-Identifier: AGPL-3.0-only
//! The `org.freedesktop.FileManager1` service on a real, private D-Bus
//! daemon.
//!
//! Ports `BusTests` of `desktop/tests/test_v07.py`. The Python tests used
//! doubles for GIO; these start their own `dbus-daemon` with no service
//! directories, so a test can own the name without reaching the session
//! bus, and nothing can be activated. This file holds the shared fixture.
//!
//! | Case file | What it covers | Ports |
//! |---|---|---|
//! | `service` | Owning the name and answering calls | `BusTests` of `desktop/tests/test_v07.py` |
//! | `requests` | The checks every request passes | `HandoffTests` of `desktop/tests/test_v07.py` |
//! | `status` | Who owns the name | `FileManagerBus.status` of `desktop/filemanager_bus.py` |

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gio::prelude::*;
use ox_core::integration::{FileManagerBus, FileManagerRequest, RequestNotOpened, BUS_NAME, OBJECT_PATH};
use tempfile::TempDir;

/// A `dbus-daemon` of its own, stopped when dropped.
struct PrivateBus {
    daemon: Child,
    address: String,
    _directory: TempDir,
}

impl PrivateBus {
    /// Starts a daemon that listens in a temporary folder, allows every
    /// connection to own any name, and has no service files to activate.
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("temporary folder");
        let config = directory.path().join("bus.conf");
        let listen = format!("unix:dir={}", directory.path().display());
        fs::write(&config, bus_configuration(&listen)).expect("bus configuration");
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon is installed with dbus-run-session");
        let stdout = daemon.stdout.take().expect("standard output is piped");
        let mut address = String::new();
        BufReader::new(stdout)
            .read_line(&mut address)
            .expect("the daemon prints its address");
        Self {
            daemon,
            address: address.trim().to_owned(),
            _directory: directory,
        }
    }

    /// A new connection to the daemon, as another application would have.
    fn connect(&self) -> gio::DBusConnection {
        let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
        gio::DBusConnection::for_address_sync(&self.address, flags, None, gio::Cancellable::NONE)
            .expect("connect to the private bus")
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

/// A session-type bus configuration that listens on `listen` and has no
/// `<servicedir>`, so nothing can be activated on it.
fn bus_configuration(listen: &str) -> String {
    format!(
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>{listen}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#
    )
}

/// A request the handler received, with its startup ID.
type Received = Rc<RefCell<Vec<(FileManagerRequest, String)>>>;

/// The app's service on a private bus, a second connection to call it
/// from, and what the handler received.
struct Fixture {
    context: glib::MainContext,
    service: FileManagerBus,
    service_connection: gio::DBusConnection,
    caller: gio::DBusConnection,
    received: Received,
    ownership_changes: Rc<Cell<u32>>,
    bus: PrivateBus,
}

impl Fixture {
    /// Sets up the service with a handler that records every request and
    /// answers `answer`. `GDBus` calls back on the thread-default context,
    /// which is this fixture's own.
    fn new(answer: Result<(), RequestNotOpened>) -> Self {
        let context = glib::MainContext::new();
        let bus = PrivateBus::start();
        let received = Received::default();
        let ownership_changes = Rc::new(Cell::new(0));
        let service_connection = bus.connect();
        let service = context
            .with_thread_default(|| {
                let recorded = Rc::clone(&received);
                let counted = Rc::clone(&ownership_changes);
                FileManagerBus::new(
                    service_connection.clone(),
                    move |request, startup_id| {
                        recorded.borrow_mut().push((request, startup_id));
                        answer
                    },
                    move || counted.set(counted.get() + 1),
                )
            })
            .expect("the context is free");
        let caller = bus.connect();
        Self {
            context,
            service,
            service_connection,
            caller,
            received,
            ownership_changes,
            bus,
        }
    }

    /// Enables the service and waits until it owns the name.
    fn enable_and_wait(&mut self) {
        let context = self.context.clone();
        context
            .with_thread_default(|| self.service.enable().expect("enable"))
            .expect("the context is free");
        self.run_until(|fixture| fixture.service.is_owned());
    }

    /// Runs the fixture's main context until `condition` holds, failing
    /// the test after five seconds.
    fn run_until(&self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition(self) {
            assert!(Instant::now() < deadline, "timed out waiting on the bus");
            self.context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Calls `method` of the service from the second connection.
    fn call(&self, method: &str, locations: &[&str], startup_id: &str) -> Result<glib::Variant, glib::Error> {
        let arguments = (locations.to_vec(), startup_id).to_variant();
        let reply = self.caller.call_future(
            Some(BUS_NAME),
            OBJECT_PATH,
            BUS_NAME,
            method,
            Some(&arguments),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
        );
        self.context.block_on(reply)
    }

    /// The unique name of the connection that owns `name`, if any.
    fn owner_of(&self, name: &str) -> Option<String> {
        let reply = self.caller.call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&(name,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
        );
        let owner = self.context.block_on(reply).ok()?;
        owner.get::<(String,)>().map(|(owner,)| owner)
    }

    fn received(&self) -> Vec<(FileManagerRequest, String)> {
        self.received.borrow().clone()
    }

    /// The unique bus name of the service's connection.
    fn service_connection_name(&self) -> String {
        let name = self
            .service_connection
            .unique_name()
            .expect("a bus connection has a unique name");
        name.to_string()
    }
}

/// The D-Bus error name GIO attached to a remote error.
fn remote_error_name(error: &glib::Error) -> Option<String> {
    gio::DBusError::remote_error(error).map(|name| name.to_string())
}

#[path = "integration_file_manager/requests.rs"]
mod requests;
#[path = "integration_file_manager/service.rs"]
mod service;
#[path = "integration_file_manager/status.rs"]
mod status;
