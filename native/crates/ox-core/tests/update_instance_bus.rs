// SPDX-License-Identifier: AGPL-3.0-only
//! The running-instance guard over a real session bus, against a stand-in
//! application that exports its actions as `org.gtk.Actions`, as
//! `GApplication` does. Ports `v2.0.0:desktop/tests/native_runtime_guard.py`,
//! which drives the production `Session` against a stand-in application on
//! an isolated bus.
//!
//! `native/tools/check.py` runs this on a private session bus. Each
//! stand-in owns a test-only application ID that contains this process's
//! ID, so no test ever addresses a real instance of the app.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gio::prelude::*;
use ox_core::update::{
    application_object_path, InstanceBus, InstanceError, InstanceGuard, InstanceStatus, LaunchMode,
    RuntimeIdentity, SessionBus, StopTiming, QUIT_ACTION, RUNTIME_INFO_ACTION, RUNTIME_PROTOCOL,
};

/// `DBUS_NAME_FLAG_DO_NOT_QUEUE`: fail rather than wait for the name.
const DO_NOT_QUEUE: u32 = 4;

/// What the stand-in reports through `runtime-info`.
#[derive(Debug, Clone)]
enum Reports {
    /// A current release.
    Identity(RuntimeIdentity),
    /// A release from before `runtime-info`.
    Nothing,
}

/// What the stand-in does when asked to quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnQuit {
    /// Releases its name, as an idle application quits.
    Quits,
    /// Keeps running, as an application does while a file operation writes.
    RefusesWhileWriting,
}

/// A stand-in application on its own thread and bus connection.
struct StandIn {
    application_id: String,
    unique_name: String,
    main_loop: glib::MainLoop,
    thread: Option<JoinHandle<()>>,
}

impl StandIn {
    fn start(reports: Reports, on_quit: OnQuit) -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let number = COUNT.fetch_add(1, Ordering::Relaxed);
        let application_id = format!("io.winspace.NativeTest.Guard{}_{number}", std::process::id());
        let (ready, started) = mpsc::channel();
        let thread = {
            let application_id = application_id.clone();
            thread::spawn(move || serve(&application_id, &reports, on_quit, &ready))
        };
        let (unique_name, main_loop) = started.recv().expect("the stand-in starts");
        Self {
            application_id,
            unique_name,
            main_loop,
            thread: Some(thread),
        }
    }

    /// The guard's view of this stand-in's application ID.
    fn bus(&self) -> SessionBus {
        SessionBus::connect(&self.application_id).expect("the private session bus")
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.main_loop.quit();
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the stand-in thread ends cleanly");
        }
    }
}

/// Runs the stand-in on this thread until its main loop quits.
fn serve(
    application_id: &str,
    reports: &Reports,
    on_quit: OnQuit,
    ready: &mpsc::Sender<(String, glib::MainLoop)>,
) {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let connection = private_connection();
            let actions = stand_in_actions(&connection, application_id, reports, on_quit);
            let path = application_object_path(application_id);
            let _export = connection
                .export_action_group(&path, &actions)
                .expect("export actions");
            bus_call(
                &connection,
                "RequestName",
                &(application_id, DO_NOT_QUEUE).to_variant(),
            );
            let main_loop = glib::MainLoop::new(Some(&context), false);
            let unique_name = connection
                .unique_name()
                .expect("a bus connection has a unique name");
            ready.send((unique_name.to_string(), main_loop.clone())).unwrap();
            main_loop.run();
        })
        .expect("the stand-in owns its main context");
}

/// A connection of the stand-in's own, as a separate process would have.
fn private_connection() -> gio::DBusConnection {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .expect("a session bus address");
    let flags =
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
    gio::DBusConnection::for_address_sync(&address, flags, None, gio::Cancellable::NONE)
        .expect("a connection to the session bus")
}

/// `runtime-info` (unless the stand-in is a legacy release) and `quit`.
fn stand_in_actions(
    connection: &gio::DBusConnection,
    application_id: &str,
    reports: &Reports,
    on_quit: OnQuit,
) -> gio::SimpleActionGroup {
    let actions = gio::SimpleActionGroup::new();
    if let Reports::Identity(identity) = reports {
        let state = identity.to_action_state().to_variant();
        actions.add_action(&gio::SimpleAction::new_stateful(
            RUNTIME_INFO_ACTION,
            None,
            &state,
        ));
    }
    let quit = gio::SimpleAction::new(QUIT_ACTION, None);
    let connection = connection.clone();
    let application_id = application_id.to_owned();
    quit.connect_activate(move |_, _| {
        if on_quit == OnQuit::Quits {
            bus_call(
                &connection,
                "ReleaseName",
                &(application_id.as_str(),).to_variant(),
            );
        }
    });
    actions.add_action(&quit);
    actions
}

/// Calls the bus daemon.
fn bus_call(connection: &gio::DBusConnection, method: &str, parameters: &glib::Variant) {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            Some(parameters),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .expect("the bus daemon answers");
}

fn current() -> RuntimeIdentity {
    RuntimeIdentity {
        version: "1.1.4".to_owned(),
        protocol: RUNTIME_PROTOCOL,
        build: "fictional-current-build".to_owned(),
    }
}

/// A guard that waits at most a second for an instance to quit.
fn guard(bus: SessionBus) -> InstanceGuard<SessionBus> {
    let timing = StopTiming {
        timeout: Duration::from_secs(1),
        poll_interval: Duration::from_millis(20),
        ..StopTiming::default()
    };
    InstanceGuard::with_timing(bus, timing)
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("No pre-existing
/// isolated application owner").
/// parity: UPD-008
#[test]
fn nobody_owns_an_application_that_does_not_run() {
    let application_id = format!("io.winspace.NativeTest.Absent{}", std::process::id());
    let guard = guard(SessionBus::connect(&application_id).unwrap());

    let status = guard
        .require_current(&current(), LaunchMode::Normal, None)
        .unwrap();

    assert_eq!(status.owner, None);
    assert!(!status.is_legacy_process());
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("current actual
/// bus owner found", "Runtime identity read through real org.gtk.Actions")
/// and `v2.0.0:desktop/tests/test_rc2.py::RuntimeTests::test_runtime_response_parsed`.
/// parity: UPD-008
#[test]
fn the_identity_is_read_through_org_gtk_actions() {
    let stand_in = StandIn::start(Reports::Identity(current()), OnQuit::Quits);
    let bus = stand_in.bus();

    let owner = bus.owner().unwrap();

    assert_eq!(owner.as_deref(), Some(stand_in.unique_name.as_str()));
    assert_eq!(bus.reported_identity(&stand_in.unique_name), Some(current()));
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("Normal
/// current-version activation leaves process running").
/// parity: UPD-008
#[test]
fn a_normal_launch_leaves_the_current_instance_running() {
    let stand_in = StandIn::start(Reports::Identity(current()), OnQuit::Quits);
    let guard = guard(stand_in.bus());

    let status = guard
        .require_current(&current(), LaunchMode::Normal, None)
        .unwrap();

    assert_eq!(status.matches(), Some(true));
    assert_eq!(
        guard.bus().owner().unwrap().as_deref(),
        Some(stand_in.unique_name.as_str())
    );
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("Explicit restart
/// waits for exact owner to exit").
/// parity: UPD-008, UPD-009
#[test]
fn an_explicit_restart_waits_for_the_exact_owner_to_exit() {
    let stand_in = StandIn::start(Reports::Identity(current()), OnQuit::Quits);
    let guard = guard(stand_in.bus());

    guard
        .require_current(&current(), LaunchMode::Restart, None)
        .unwrap();

    assert_eq!(guard.bus().owner().unwrap(), None);
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("Busy server
/// refuses restart without being killed").
/// parity: UPD-008, UPD-009
#[test]
fn a_busy_instance_refuses_restart_and_is_not_killed() {
    let stand_in = StandIn::start(Reports::Identity(current()), OnQuit::RefusesWhileWriting);
    let guard = guard(stand_in.bus());

    let error = guard
        .require_current(&current(), LaunchMode::Restart, None)
        .unwrap_err();

    assert!(matches!(error, InstanceError::StillRunning), "{error}");
    assert_eq!(
        guard.bus().owner().unwrap().as_deref(),
        Some(stand_in.unique_name.as_str())
    );
}

/// Ported from `v2.0.0:desktop/tests/native_runtime_guard.py` ("legacy child
/// registered", "Legacy process detected without identity action", "Legacy
/// safe quit releases the bus name") and
/// `v2.0.0:desktop/tests/test_rc2.py::RuntimeTests::test_legacy_missing_action`.
/// parity: UPD-008
#[test]
fn a_legacy_instance_is_detected_and_quits_safely() {
    let stand_in = StandIn::start(Reports::Nothing, OnQuit::Quits);
    let guard = guard(stand_in.bus());
    let agree = |status: &InstanceStatus| status.is_legacy_process();

    let status = guard
        .require_current(&current(), LaunchMode::Normal, Some(&agree))
        .unwrap();

    assert_eq!(status.owner.as_deref(), Some(stand_in.unique_name.as_str()));
    assert_eq!(status.running, None);
    assert_eq!(guard.bus().owner().unwrap(), None);
}
