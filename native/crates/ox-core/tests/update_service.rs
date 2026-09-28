// SPDX-License-Identifier: AGPL-3.0-only
//! The application lock while an update installs, and the worker-thread
//! orchestration. Ports the installation tests of `BridgeTests` in
//! `desktop/tests/test_updater.py`, which run the update branches of
//! `dispatch`, `on_delete`, `create_window` and `quit_safely` in
//! `desktop/winspace.py`.
//!
//! The service is shared by every window, so "in every window" is one
//! service here.

mod update_support;

use std::cell::RefCell;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex};

use futures_util::FutureExt;
use ox_core::transfer::Cancellation;
use ox_core::update::{
    Activity, AppRequest, Confirmation, InstallRequest, UpdateError, UpdatePhase, LATEST_RELEASE_URL,
};
use update_support::service::{confirmed, ServiceFixture, ALL_REQUESTS};

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_running_update_blocks_file_actions_and_other_updates_in_every_window`
/// parity: UPD-005
#[test]
fn running_update_blocks_file_actions_and_other_updates_in_every_window() {
    let fixture = ServiceFixture::checked();

    let installed = fixture.while_installing(|service| {
        for request in ALL_REQUESTS {
            let result = service.check_request(request);
            if request == AppRequest::WindowChrome {
                assert!(result.is_ok());
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.to_string().contains("update is running"),
                    "{request:?}: {error}"
                );
            }
        }
        let restart = service.restart(Activity::Idle);
        assert!(matches!(restart, Err(UpdateError::UpdateRunning)));
    });

    installed.unwrap();
    assert!(fixture.launched().is_empty());
}

/// A second check or installation while one installs is refused before
/// it reaches the updater.
/// parity: UPD-005
#[test]
fn a_running_update_refuses_another_check_or_installation() {
    let fixture = ServiceFixture::checked();
    let second = RefCell::new(None);

    fixture
        .while_installing(|service| {
            // Both are refused before they await anything.
            let check = service.check(Cancellation::new()).now_or_never();
            let install = service.install(confirmed(Activity::Idle), |_| {}).now_or_never();
            second.replace(Some((check, install)));
        })
        .unwrap();

    let (check, install) = second.take().unwrap();
    assert!(matches!(check, Some(Err(UpdateError::UpdateRunning))));
    assert!(matches!(install, Some(Err(UpdateError::UpdateRunning))));
    assert_eq!(
        fixture.updates.server.opened().len(),
        2,
        "one check and one download"
    );
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_confirmation_is_required_before_worker_or_busy_state`
/// parity: UPD-003, UPD-005
#[test]
fn confirmation_is_required_before_worker_or_busy_state() {
    let fixture = ServiceFixture::checked();
    let request = InstallRequest {
        confirmation: Confirmation::Unconfirmed,
        ..confirmed(Activity::Idle)
    };

    let error = fixture.install(request).unwrap_err();

    assert!(error.to_string().starts_with("Confirm"), "{error}");
    assert_eq!(fixture.service.phase(), UpdatePhase::Idle);
    assert_eq!(fixture.updates.server.opened(), [LATEST_RELEASE_URL]);
    assert!(fixture.updates.packages.commands().is_empty());
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_active_work_in_any_window_blocks_install`
///
/// Python checks each window's jobs, writes, mount prompts and handoff,
/// and pending tab moves; the window side sums them up as [`Activity`].
/// parity: UPD-003, UPD-005
#[test]
fn active_work_in_any_window_blocks_install() {
    let fixture = ServiceFixture::checked();

    let error = fixture.install(confirmed(Activity::Busy)).unwrap_err();

    let message = error.to_string();
    assert!(message.contains("Wait for file operations"), "{message}");
    assert!(message.contains("tab moves"), "{message}");
    assert_eq!(fixture.service.phase(), UpdatePhase::Idle);
    assert!(fixture.updates.packages.commands().is_empty());
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_queued_install_blocks_second_window_before_worker_starts`
/// parity: UPD-005
#[test]
fn queued_install_blocks_second_window_before_worker_starts() {
    let fixture = ServiceFixture::checked();
    let observed = RefCell::new(None);

    let installed = fixture.while_installing(|service| {
        observed.replace(Some((service.phase(), service.check_request(AppRequest::Files))));
    });

    installed.unwrap();
    let (phase, search) = observed.take().unwrap();
    assert_eq!(phase, UpdatePhase::Installing);
    assert!(search.unwrap_err().to_string().contains("update is running"));
    assert_ne!(fixture.service.phase(), UpdatePhase::Installing);
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_bridge_only_passes_version_confirmation_and_progress_not_remote_commands`
///
/// An [`InstallRequest`] has no field for an address, path, command or
/// digest; progress reaches the caller's thread.
/// parity: UPD-003, UPD-005
#[test]
fn only_version_confirmation_and_progress_cross_to_the_installation() {
    let fixture = ServiceFixture::checked();
    let progress = RefCell::new(Vec::new());
    let thread = std::thread::current().id();

    let installed = fixture
        .context
        .block_on(fixture.service.install(confirmed(Activity::Idle), |step| {
            assert_eq!(
                std::thread::current().id(),
                thread,
                "progress arrives on the caller's thread"
            );
            progress.borrow_mut().push(step.to_string());
        }));

    installed.unwrap();
    assert_eq!(progress.borrow().len(), 3);
    assert_eq!(progress.borrow()[0], "Downloading OpenXplorer 1.0.1…");
    assert!(fixture.launched().is_empty());
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_worker_submission_failure_clears_app_busy`
///
/// A GIO worker cannot fail to start; the equivalent failure is a worker
/// that dies. The application must not stay locked, and since the
/// installation may have changed files, it waits for a restart.
/// parity: UPD-005
#[test]
fn a_worker_that_dies_does_not_leave_the_application_locked() {
    let fixture = ServiceFixture::checked();
    fixture
        .updates
        .packages
        .on_install(Arc::new(|| panic!("Fictional unavailable worker")));

    let outcome = panic::catch_unwind(AssertUnwindSafe(|| fixture.install(confirmed(Activity::Idle))));

    assert!(outcome.is_err(), "the worker's panic reaches the caller");
    assert_eq!(fixture.service.phase(), UpdatePhase::RestartRequired);
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_quit_and_window_close_refuse_while_update_runs`
/// parity: UPD-005
#[test]
fn quit_and_window_close_refuse_while_update_runs() {
    let fixture = ServiceFixture::checked();
    let refusals = RefCell::new(Vec::new());

    fixture
        .while_installing(|service| {
            let quit = service.check_quit().unwrap_err().to_string();
            let close = service.check_close_window().unwrap_err().to_string();
            refusals.replace(vec![quit, close]);
        })
        .unwrap();

    assert_eq!(
        refusals.take(),
        [
            "Wait for the application update to finish before quitting.",
            "Wait for the application update to finish before closing.",
        ]
    );
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_new_windows_are_blocked_during_update_and_until_restart`
/// parity: UPD-005, UPD-006
#[test]
fn new_windows_are_blocked_during_update_and_until_restart() {
    let fixture = ServiceFixture::checked();
    let during = RefCell::new(None);

    fixture
        .while_installing(|service| {
            during.replace(Some(service.check_new_window()));
        })
        .unwrap();
    let waiting = ServiceFixture::waiting_for_restart();

    let during = during.take().unwrap().unwrap_err();
    let after = waiting.service.check_new_window().unwrap_err();
    for error in [during, after] {
        assert!(
            error.to_string().starts_with("Finish the application update"),
            "{error}"
        );
    }
    assert!(ServiceFixture::new().service.check_new_window().is_ok());
}

/// An installation whose caller stopped waiting keeps the application
/// locked until APT has finished: quitting, closing a window and
/// restarting are refused while it runs, then the installed build decides
/// the phase as usual.
/// parity: UPD-005
#[test]
fn an_abandoned_installation_keeps_the_application_locked_until_apt_finishes() {
    let fixture = ServiceFixture::checked();
    let (reached, installing) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let executable = fixture.executable.clone();
    let hold_apt_then_upgrade = move || {
        reached.send(()).unwrap();
        released.lock().unwrap().recv().unwrap();
        fs::write(&executable, b"Fictional build 1.0.1").unwrap();
    };
    fixture
        .updates
        .packages
        .on_install(Arc::new(hold_apt_then_upgrade));
    let installation = fixture.service.install(confirmed(Activity::Idle), |_| {});

    // Polls the installation once on the fixture's main context, then
    // stops waiting for it.
    let abandoned = fixture.context.block_on(async { installation.now_or_never() });
    installing.recv().unwrap();

    assert!(abandoned.is_none());
    let service = &fixture.service;
    assert_eq!(service.phase(), UpdatePhase::Installing);
    assert!(service.check_request(AppRequest::Files).is_err());
    assert!(service.check_quit().is_err());
    assert!(service.check_close_window().is_err());
    assert!(service.restart(Activity::Idle).is_err());
    assert!(fixture.launched().is_empty());
    release.send(()).unwrap();
    fixture.wait_until_installation_ends();
    assert_eq!(service.phase(), UpdatePhase::RestartRequired);
    assert!(service.check_quit().is_ok());
}
