// SPDX-License-Identifier: AGPL-3.0-only
//! The running-instance guard over a scripted bus, and build identities.
//! Ports `RuntimeTests` of `desktop/tests/test_rc2.py`;
//! `update_instance_bus.rs` runs the guard over a real session bus.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::time::Duration;

use ox_core::update::{
    InstanceBus, InstanceError, InstanceGuard, InstanceStatus, LaunchMode, RuntimeIdentity, StopTiming,
    RUNTIME_PROTOCOL,
};

const OWNER: &str = ":1.55";

/// A bus whose owner answers come from a script, and which records quit
/// requests. Once the script runs out, the last answer repeats.
#[derive(Debug, Default)]
struct ScriptedBus {
    owners: RefCell<VecDeque<Option<String>>>,
    running: Option<RuntimeIdentity>,
    refuses_quit: bool,
    quit_requests: RefCell<Vec<String>>,
}

impl ScriptedBus {
    /// `owners` answer the owner questions in turn; `running` is what the
    /// instance reports.
    fn new(owners: &[Option<&str>], running: Option<RuntimeIdentity>) -> Self {
        let owners = owners.iter().map(|owner| owner.map(str::to_owned)).collect();
        Self {
            owners: RefCell::new(owners),
            running,
            ..Self::default()
        }
    }

    fn quit_requests(&self) -> Vec<String> {
        self.quit_requests.borrow().clone()
    }
}

impl InstanceBus for ScriptedBus {
    fn owner(&self) -> Result<Option<String>, InstanceError> {
        let mut owners = self.owners.borrow_mut();
        let answer = if owners.len() > 1 {
            owners.pop_front()
        } else {
            owners.front().cloned()
        };
        Ok(answer.flatten())
    }

    fn reported_identity(&self, _owner: &str) -> Option<RuntimeIdentity> {
        self.running.clone()
    }

    fn request_quit(&self, owner: &str) -> Result<(), InstanceError> {
        self.quit_requests.borrow_mut().push(owner.to_owned());
        if self.refuses_quit {
            let refusal = glib::Error::new(gio::IOErrorEnum::Busy, "Fictional active file operations");
            return Err(InstanceError::Bus(refusal));
        }
        Ok(())
    }
}

/// The installed build of the fixtures.
fn installed() -> RuntimeIdentity {
    RuntimeIdentity {
        version: "1.1.4".to_owned(),
        protocol: RUNTIME_PROTOCOL,
        build: "abc".to_owned(),
    }
}

/// An older build than [`installed`].
fn older() -> RuntimeIdentity {
    RuntimeIdentity {
        build: "older".to_owned(),
        ..installed()
    }
}

/// A guard that asks the bus again at once instead of every 80 ms.
fn guard(bus: ScriptedBus) -> InstanceGuard<ScriptedBus> {
    let timing = StopTiming {
        poll_interval: Duration::ZERO,
        ..StopTiming::default()
    };
    InstanceGuard::with_timing(bus, timing)
}

/// A guard that also gives up at once instead of after six seconds.
fn impatient(bus: ScriptedBus) -> InstanceGuard<ScriptedBus> {
    let timing = StopTiming {
        timeout: Duration::ZERO,
        poll_interval: Duration::ZERO,
    };
    InstanceGuard::with_timing(bus, timing)
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_identity_changes_with_python_and_ui`
/// parity: UPD-008
#[test]
fn identity_changes_with_python_and_ui() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path();
    fs::create_dir(folder.join("ui")).unwrap();
    fs::write(folder.join("x.py"), "a=1").unwrap();
    fs::write(folder.join("ui/app.js"), "x=1;").unwrap();
    let one = RuntimeIdentity::of_python_install(folder, "1.1.4").unwrap();

    fs::write(folder.join("x.py"), "a=2").unwrap();
    let two = RuntimeIdentity::of_python_install(folder, "1.1.4").unwrap();
    fs::write(folder.join("ui/app.js"), "x=2;").unwrap();
    let three = RuntimeIdentity::of_python_install(folder, "1.1.4").unwrap();

    assert_ne!(one.build, two.build);
    assert_ne!(two.build, three.build);
}

/// Files outside the build's inputs do not change its identity.
/// parity: UPD-008
#[test]
fn identity_ignores_files_that_are_not_part_of_the_build() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path();
    fs::create_dir(folder.join("ui")).unwrap();
    fs::write(folder.join("core.py"), "a=1").unwrap();
    let before = RuntimeIdentity::of_python_install(folder, "1.1.4").unwrap();

    fs::write(folder.join("notes.txt"), "not code").unwrap();
    fs::write(folder.join("ui/readme.md"), "not code").unwrap();
    fs::create_dir(folder.join("ui/nested")).unwrap();

    assert_eq!(
        RuntimeIdentity::of_python_install(folder, "1.1.4").unwrap(),
        before
    );
    assert_eq!(before.protocol, RUNTIME_PROTOCOL);
    assert_eq!(before.version, "1.1.4");
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_new_process`
/// parity: UPD-008
#[test]
fn new_process() {
    let guard = impatient(ScriptedBus::new(&[None], None));

    let status = guard
        .require_current(&installed(), LaunchMode::Normal, None)
        .unwrap();

    assert_eq!(status.owner, None);
    assert_eq!(status.matches(), None);
    assert!(guard.bus().quit_requests().is_empty());
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_existing_current_process`
/// parity: UPD-008
#[test]
fn existing_current_process() {
    let guard = impatient(ScriptedBus::new(&[Some(OWNER)], Some(installed())));

    let status = guard
        .require_current(&installed(), LaunchMode::Normal, None)
        .unwrap();

    assert_eq!(status.matches(), Some(true));
    assert!(guard.bus().quit_requests().is_empty());
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_old_process_needs_consent`
/// parity: UPD-008
#[test]
fn old_process_needs_consent() {
    let guard = impatient(ScriptedBus::new(&[Some(OWNER)], Some(older())));

    let error = guard
        .require_current(&installed(), LaunchMode::Normal, None)
        .unwrap_err();

    assert!(matches!(error, InstanceError::OutdatedInstance));
    assert!(error.to_string().contains("restart"), "{error}");
    assert!(guard.bus().quit_requests().is_empty());
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_decline_never_stops`
/// parity: UPD-008
#[test]
fn decline_never_stops() {
    let guard = impatient(ScriptedBus::new(&[Some(OWNER)], Some(older())));
    let decline = |_: &InstanceStatus| false;

    let result = guard.require_current(&installed(), LaunchMode::Normal, Some(&decline));

    assert!(matches!(result, Err(InstanceError::OutdatedInstance)));
    assert!(guard.bus().quit_requests().is_empty());
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_confirm_stops_only_exact_owner`
/// parity: UPD-008
#[test]
fn confirm_stops_only_exact_owner() {
    let guard = guard(ScriptedBus::new(&[Some(OWNER), Some(OWNER), None], Some(older())));
    let asked = RefCell::new(None);
    let confirm = |status: &InstanceStatus| {
        asked.replace(Some(status.clone()));
        true
    };

    guard
        .require_current(&installed(), LaunchMode::Normal, Some(&confirm))
        .unwrap();

    assert_eq!(guard.bus().quit_requests(), [OWNER]);
    let asked = asked.take().unwrap();
    assert_eq!(asked.running, Some(older()));
    assert_eq!(asked.matches(), Some(false));
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_explicit_restart_of_current_process`
/// parity: UPD-008, UPD-009
#[test]
fn explicit_restart_of_current_process() {
    let guard = guard(ScriptedBus::new(
        &[Some(OWNER), Some(OWNER), None],
        Some(installed()),
    ));

    guard
        .require_current(&installed(), LaunchMode::Restart, None)
        .unwrap();

    assert_eq!(guard.bus().quit_requests(), [OWNER]);
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_refused_quit_is_not_ignored`
///
/// Python's mocked `stop` raises; here the instance refuses the quit
/// request and keeps its name, which is what a writing instance does.
/// parity: UPD-008, UPD-009
#[test]
fn refused_quit_is_not_ignored() {
    let bus = ScriptedBus {
        refuses_quit: true,
        ..ScriptedBus::new(&[Some(OWNER)], Some(older()))
    };
    let guard = impatient(bus);

    let error = guard
        .require_current(&installed(), LaunchMode::Restart, None)
        .unwrap_err();

    assert!(error.to_string().contains("active file operations"), "{error}");
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_session_stop_waits_for_release`
/// parity: UPD-008, UPD-009
#[test]
fn session_stop_waits_for_release() {
    let guard = guard(ScriptedBus::new(&[Some(OWNER), None], None));

    guard.stop(OWNER).unwrap();

    assert_eq!(guard.bus().quit_requests(), [OWNER]);
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_busy_process_is_never_forced`
/// parity: UPD-008, UPD-009
#[test]
fn busy_process_is_never_forced() {
    let guard = impatient(ScriptedBus::new(&[Some(OWNER)], None));

    let error = guard.stop(OWNER).unwrap_err();

    assert!(matches!(error, InstanceError::StillRunning));
    assert!(error.to_string().contains("No process was killed"));
    assert_eq!(guard.bus().quit_requests().len(), 1);
}

/// Ported from `desktop/tests/test_rc2.py::RuntimeTests::test_owner_changed_during_restart`
/// parity: UPD-009
#[test]
fn owner_changed_during_restart() {
    let guard = impatient(ScriptedBus::new(&[Some(":1.80")], None));

    let error = guard.stop(OWNER).unwrap_err();

    assert!(matches!(error, InstanceError::AnotherInstance));
    assert!(error.to_string().starts_with("Another OpenXplorer"));
}

/// A failed quit request from an instance that already quit is not an
/// error: the name is gone, which is what the guard waits for.
/// parity: UPD-009
#[test]
fn a_failed_quit_from_an_instance_that_already_quit_is_fine() {
    let bus = ScriptedBus {
        refuses_quit: true,
        ..ScriptedBus::new(&[None], None)
    };
    let guard = impatient(bus);

    guard.stop(OWNER).unwrap();
}

/// `--diagnose` reports a release without `runtime-info` as a legacy
/// process that does not match.
/// parity: UPD-008
#[test]
fn a_process_without_an_identity_is_a_legacy_process() {
    let guard = impatient(ScriptedBus::new(&[Some(OWNER)], None));

    let status = guard.status(&installed()).unwrap();

    assert!(status.is_legacy_process());
    assert_eq!(status.matches(), Some(false));
    assert!(!impatient(ScriptedBus::new(&[None], None))
        .status(&installed())
        .unwrap()
        .is_legacy_process());
}

/// A running instance matches only if version, protocol and build all
/// match, as `same_build` compares them.
/// parity: UPD-008
#[test]
fn a_match_needs_version_protocol_and_build() {
    let variants = [
        RuntimeIdentity {
            version: "1.1.3".to_owned(),
            ..installed()
        },
        RuntimeIdentity {
            protocol: 0,
            ..installed()
        },
        older(),
    ];
    for running in variants {
        let guard = impatient(ScriptedBus::new(&[Some(OWNER)], Some(running.clone())));

        let status = guard.status(&installed()).unwrap();

        assert_eq!(status.matches(), Some(false), "{running:?}");
    }
}
