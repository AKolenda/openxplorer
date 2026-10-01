// SPDX-License-Identifier: AGPL-3.0-only
//! Checking for updates, one task at a time, and which builds may install.
//! Ports the check tests of `UpdaterTests` in
//! `v2.0.0:desktop/tests/test_updater.py`, with fictional HTTP bytes. No network
//! connection, administrator prompt or package installation is made.

mod update_support;

use std::sync::{mpsc, Arc};
use std::thread;

use ox_core::transfer::Cancellation;
use ox_core::update::{FetchError, Installation, UpdateError, LATEST_RELEASE_URL};
use update_support::{install, Response, UpdaterFixture, CURRENT, NEXT};

/// Ported from `v2.0.0:desktop/tests/test_updater.py::UpdaterTests::test_check_uses_fixed_endpoint_and_only_returns_public_metadata`
///
/// The installer's address, digest, size and name are not fields of
/// [`ox_core::update::UpdateStatus`], so they cannot leave the updater.
/// parity: UPD-001, UPD-002
#[test]
fn check_uses_fixed_endpoint_and_only_returns_public_metadata() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.updater(Installation::DebianPackage);

    let status = updater.check(&Cancellation::new()).unwrap();

    assert_eq!(fixture.server.opened(), [LATEST_RELEASE_URL]);
    assert!(status.is_available);
    assert!(status.can_install);
    assert!(!status.restart_required);
    assert_eq!(status.latest_version, NEXT);
    assert_eq!(status.current_version, CURRENT);
    assert_eq!(status.notes, "Fictional release notes.");
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// Ported from `v2.0.0:desktop/tests/test_updater.py::UpdaterTests::test_failed_check_discards_stale_release_and_releases_lock`
///
/// Python's `TimeoutError` is a connection failure like any other here.
/// parity: UPD-001, UPD-002
#[test]
fn failed_check_discards_stale_release_and_releases_lock() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    let failures = [
        (
            FetchError::Status(403),
            "GitHub could not check for updates (HTTP 403). Try again later.",
        ),
        (
            FetchError::Unreachable,
            "Could not reach GitHub. Check your connection and try again.",
        ),
    ];
    for (failure, message) in failures {
        fixture.server.answer_release(Response::Refused(failure));

        let error = updater.check(&Cancellation::new()).unwrap_err();

        assert_eq!(error.to_string(), message);
        let (install_result, _) = install(&updater, NEXT);
        assert!(
            matches!(install_result, Err(UpdateError::NotChecked)),
            "the stale release was discarded"
        );
        fixture.assert_idle_and_clean(&updater);
    }
}

/// Ported from `v2.0.0:desktop/tests/test_updater.py::UpdaterTests::test_oversized_or_invalid_metadata_response_fails_and_releases_lock`
/// parity: UPD-001, UPD-002
#[test]
fn oversized_or_invalid_metadata_response_fails_and_releases_lock() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.updater(Installation::DebianPackage);
    let oversized = vec![b'x'; 2 * 1024 * 1024 + 1];
    let answers = [(oversized, "too large"), (b"not JSON".to_vec(), "not valid JSON")];
    for (payload, message) in answers {
        fixture.server.answer_release(Response::Body(payload));

        let error = updater.check(&Cancellation::new()).unwrap_err();

        assert!(error.to_string().contains(message), "{error}");
        assert!(matches!(install(&updater, NEXT).0, Err(UpdateError::NotChecked)));
        fixture.assert_idle_and_clean(&updater);
    }
}

/// Ported from `v2.0.0:desktop/tests/test_updater.py::UpdaterTests::test_concurrent_tasks_are_rejected_without_network_or_processes`
///
/// Python holds the lock by hand; here a first check holds it while the
/// package manager of a second task is never reached.
/// parity: UPD-003
#[test]
fn concurrent_tasks_are_rejected_without_network_or_processes() {
    let fixture = UpdaterFixture::new();
    let (started, check_started) = mpsc::channel();
    let (resume, check_resumes) = mpsc::channel::<()>();
    let updater = Arc::new(fixture.updater(Installation::DebianPackage));
    fixture.server.pause_next_open(started, check_resumes);
    let running = {
        let updater = Arc::clone(&updater);
        let cancel = Cancellation::new();
        thread::spawn(move || updater.check(&cancel))
    };
    check_started.recv().unwrap();

    let second_check = updater.check(&Cancellation::new());
    let install_result = install(&updater, NEXT).0;

    resume.send(()).unwrap();
    running.join().unwrap().unwrap();
    assert!(second_check.unwrap_err().to_string().contains("already running"));
    assert!(install_result
        .unwrap_err()
        .to_string()
        .contains("already running"));
    assert_eq!(fixture.server.opened(), [LATEST_RELEASE_URL]);
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// Ported from `v2.0.0:desktop/tests/test_updater.py::UpdaterTests::test_source_build_cannot_install`
/// parity: UPD-004
#[test]
fn source_build_cannot_install() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.updater(Installation::Unpackaged);
    let status = updater.check(&Cancellation::new()).unwrap();

    let (result, _) = install(&updater, NEXT);

    assert!(!status.can_install);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("installed Debian package"));
    assert_eq!(fixture.server.opened(), [LATEST_RELEASE_URL]);
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// A Flatpak build checks for updates but leaves installing to Flatpak.
/// parity: UPD-004
#[test]
fn a_flatpak_build_refuses_to_install_and_names_flatpak() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.updater(Installation::Flatpak);
    updater.check(&Cancellation::new()).unwrap();

    let (result, _) = install(&updater, NEXT);

    let error = result.unwrap_err();
    assert!(matches!(error, UpdateError::InstallUnavailable { .. }));
    assert!(error.to_string().contains("Flatpak"), "{error}");
    assert!(fixture.packages.commands().is_empty());
}
