// SPDX-License-Identifier: AGPL-3.0-only
//! The executable as the desktop starts it: separate processes of one
//! application on the test's private session bus.
//!
//! These start `openxplorer-native` itself, so they run only inside the
//! isolation `native/tools/check.py` sets up (a private display and
//! session bus, and HOME inside the temporary folder); anywhere else they
//! skip, so a process never reaches the live desktop.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output};
use std::thread;
use std::time::{Duration, Instant};

use gio::prelude::*;
use ox_core::integration::{RevealPaths, RevealRegistration, Sandbox, BUS_NAME};

/// The executable under test.
const APP: &str = env!("CARGO_BIN_EXE_openxplorer-native");

/// The application ID the executable was built with.
const APP_ID: &str = env!("OX_APP_ID");

/// How long a process may take to start, answer or quit.
const PATIENCE: Duration = Duration::from_secs(30);

/// Whether the test runs in check.py's isolation: its marker, which it
/// sets only inside its private display and session bus, and HOME and the
/// XDG folders Show in folder writes to inside a private temporary folder.
fn isolated() -> bool {
    let temporary = std::env::temp_dir();
    let is_private = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .is_some_and(|path| path.starts_with(&temporary))
    };
    std::env::var_os("OX_ISOLATED_SESSION").is_some_and(|marker| marker == "1")
        && temporary != Path::new("/tmp")
        && ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]
            .into_iter()
            .all(is_private)
        && std::env::var_os("DISPLAY").is_some()
}

/// A started instance, stopped when the test ends however it ends.
struct Instance(Child);

impl Instance {
    fn start(arguments: &[&str]) -> Self {
        Self(Command::new(APP).args(arguments).spawn().expect("the app starts"))
    }

    /// Waits until the process exits and returns how.
    fn wait(&mut self, what: &str) -> ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(status) = self.0.try_wait().expect("the process can be asked") {
                return status;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn is_running(&mut self) -> bool {
        self.0.try_wait().expect("the process can be asked").is_none()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Runs the app with `arguments` to the end.
fn run(arguments: &[&str]) -> Output {
    let mut child = Command::new(APP)
        .args(arguments)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the app starts");
    let deadline = Instant::now() + PATIENCE;
    while child.try_wait().expect("the process can be asked").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("openxplorer {arguments:?} did not finish");
        }
        thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().expect("the output can be read")
}

/// The unique bus name that owns the application ID, if any.
fn owner() -> Option<String> {
    owner_of(APP_ID)
}

/// The unique bus name that owns `name`, if any. Never starts a service.
fn owner_of(name: &str) -> Option<String> {
    let bus =
        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("the private session bus");
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&(name,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            1000,
            gio::Cancellable::NONE,
        )
        .ok()?;
    reply.get::<(String,)>().map(|(name,)| name)
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(100));
    }
}

/// One application per session: a later launch hands its command line to
/// the running instance and exits; `--diagnose` reads that instance's
/// identity; `--restart` replaces it and `--quit` ends it, without killing
/// either. The Show in folder service exits at once without the opt-in,
/// and with it stays running to own `FileManager1` until `--quit`.
///
/// parity: INT-001, INT-004, INT-017, UPD-008, UPD-009, UPD-010
#[test]
fn launches_share_one_instance_that_quits_and_restarts_on_request() {
    if !isolated() {
        eprintln!("skipped: run inside native/tools/check.py, which gives the app a private session");
        return;
    }
    let folder = tempfile::tempdir().expect("a temporary folder");
    let folder = folder.path().to_str().expect("a UTF-8 path").to_owned();

    let version = run(&["--version"]);
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("OpenXplorer {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = String::from_utf8_lossy(&run(&["--help-all"]).stdout).into_owned();
    for option in [
        "--new-window",
        "--select",
        "--check",
        "--diagnose",
        "--restart",
        "--quit",
    ] {
        assert!(help.contains(option), "--help lists {option}: {help}");
    }
    let invalid = run(&["--select"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        "--select needs a file path.\n"
    );

    let without_opt_in = run(&["--filemanager-service"]);
    assert!(
        without_opt_in.status.success(),
        "the service without the opt-in exits"
    );
    assert_eq!(owner(), None, "and leaves nothing running");

    let settings = ox_core::settings::Settings::default_directory();
    let reveal = RevealRegistration::new(&RevealPaths::for_user(&settings), Sandbox::Host);
    reveal
        .enable()
        .expect("Show in folder can be enabled in the private session");
    let mut service = Instance::start(&["--filemanager-service"]);
    wait_until("the service to own FileManager1", || {
        owner().is_some() && owner_of(BUS_NAME) == owner()
    });
    assert!(service.is_running(), "the service stays running without a window");
    assert!(run(&["--quit"]).status.success());
    assert!(service.wait("the service to quit").success());
    reveal.disable().expect("Show in folder can be disabled again");

    let mut first = Instance::start(&[&folder]);
    wait_until("the first instance", || owner().is_some());
    let first_owner = owner();

    let second = run(&["--windows"]);
    assert!(second.status.success());
    assert!(first.is_running(), "the first instance keeps running");
    assert_eq!(owner(), first_owner, "no second instance took over");

    wait_until("the running instance to report this build", || {
        let report = run(&["--diagnose"]);
        let text = String::from_utf8_lossy(&report.stdout);
        text.contains("\"matches\": true") && text.contains("\"legacyProcess\": false")
    });

    let mut restarted = Instance::start(&["--restart", &folder]);
    assert!(first.wait("the first instance to quit").success());
    wait_until("the restarted instance", || {
        owner().is_some_and(|name| Some(&name) != first_owner.as_ref())
    });

    assert!(run(&["--quit"]).status.success());
    assert!(restarted.wait("the restarted instance to quit").success());
    assert_eq!(owner(), None);
}
