// SPDX-License-Identifier: AGPL-3.0-only
//! After an installation: waiting for a restart when the installed files
//! changed, and restarting only through the fixed launcher. Ports the
//! restart tests of `BridgeTests` in `desktop/tests/test_updater.py`.

mod update_support;

use std::fs;
use std::sync::Arc;

use ox_core::transfer::Cancellation;
use ox_core::update::{Activity, AppRequest, UpdateCheck, UpdatePhase, RESTART_COMMAND};
use update_support::service::{confirmed, ServiceFixture};
use update_support::{failure, NEXT};

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_installed_update_blocks_files_but_allows_status_without_new_check`
/// parity: UPD-005, UPD-006
#[test]
fn installed_update_blocks_files_but_allows_status_without_new_check() {
    let fixture = ServiceFixture::waiting_for_restart();
    let opened_before = fixture.updates.server.opened();

    for request in [AppRequest::Files, AppRequest::UpdateInstall] {
        let error = fixture.service.check_request(request).unwrap_err();

        assert!(
            error.to_string().contains("Restart OpenXplorer"),
            "{request:?}: {error}"
        );
    }
    let allowed = [
        AppRequest::Environment,
        AppRequest::UpdateCheck,
        AppRequest::UpdateRestart,
        AppRequest::Quit,
        AppRequest::WindowChrome,
    ];
    assert!(allowed
        .iter()
        .all(|request| fixture.service.check_request(*request).is_ok()));
    let status = fixture
        .context
        .block_on(fixture.service.check(Cancellation::new()));
    assert_eq!(
        status.unwrap(),
        UpdateCheck::RestartPending {
            installed_version: Some(NEXT)
        }
    );
    assert_eq!(
        fixture.updates.server.opened(),
        opened_before,
        "GitHub was not asked"
    );
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_modified_installed_files_require_restart_after_success_or_failure`
/// parity: UPD-005, UPD-006
#[test]
fn modified_installed_files_require_restart_after_success_or_failure() {
    let succeeded = ServiceFixture::checked();
    succeeded.replace_build_during_installation();

    succeeded.install(confirmed(Activity::Idle)).unwrap();

    assert_eq!(succeeded.service.phase(), UpdatePhase::RestartRequired);
    let failed = ServiceFixture::checked();
    failed.replace_build_during_installation();
    failed
        .updates
        .packages
        .answer_installation(failure(1, "Fictional failed configuration"));

    let error = failed.install(confirmed(Activity::Idle)).unwrap_err();

    assert!(error.to_string().contains("failed configuration"), "{error}");
    assert_eq!(failed.service.phase(), UpdatePhase::RestartRequired);
}

/// An installation that failed before it changed any file leaves the
/// application usable.
/// parity: UPD-006
#[test]
fn an_installation_that_changed_nothing_needs_no_restart() {
    let fixture = ServiceFixture::checked();
    fixture
        .updates
        .packages
        .answer_installation(failure(126, "Fictional refusal"));

    let error = fixture.install(confirmed(Activity::Idle)).unwrap_err();

    assert!(error.to_string().contains("cancelled or failed"), "{error}");
    assert_eq!(fixture.service.phase(), UpdatePhase::Idle);
    assert!(fixture.service.check_request(AppRequest::Files).is_ok());
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_identity_read_failure_always_clears_busy_and_requires_restart`
/// parity: UPD-005, UPD-006
#[test]
fn identity_read_failure_always_clears_busy_and_requires_restart() {
    let fixture = ServiceFixture::checked();
    let executable = fixture.executable.clone();
    let remove = move || fs::remove_file(&executable).unwrap();
    fixture.updates.packages.on_install(Arc::new(remove));

    fixture.install(confirmed(Activity::Idle)).unwrap();

    assert_eq!(fixture.service.phase(), UpdatePhase::RestartRequired);
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_pending_restart_allows_safe_quit`
/// parity: UPD-007
#[test]
fn pending_restart_allows_safe_quit() {
    let fixture = ServiceFixture::waiting_for_restart();

    assert!(fixture.service.check_quit().is_ok());
    assert!(fixture.service.check_close_window().is_ok());
}

/// Ported from `desktop/tests/test_updater.py::BridgeTests::test_restart_requires_pending_update_idle_writes_and_fixed_launcher`
/// parity: UPD-007
#[test]
fn restart_requires_pending_update_idle_writes_and_fixed_launcher() {
    let idle = ServiceFixture::new();
    let nothing_installed = idle.service.restart(Activity::Idle).unwrap_err();
    assert!(nothing_installed.to_string().contains("No installed update"));
    let fixture = ServiceFixture::waiting_for_restart();

    let writing = fixture.service.restart(Activity::Busy).unwrap_err();
    assert!(writing.to_string().contains("file operations"), "{writing}");
    assert!(fixture.launched().is_empty());
    fixture.service.restart(Activity::Idle).unwrap();

    assert_eq!(fixture.launched(), [RESTART_COMMAND.map(String::from).to_vec()]);
    assert_eq!(RESTART_COMMAND, ["/usr/bin/openxplorer", "--restart"]);
    assert!(idle.launched().is_empty());
}
