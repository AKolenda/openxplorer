// SPDX-License-Identifier: AGPL-3.0-only
//! Installing an update: the private, verified download and the package
//! commands around the administrator prompt. Ports the installation tests
//! of `UpdaterTests` in `desktop/tests/test_updater.py`, with fictional
//! HTTP bytes and a package manager double. No network connection,
//! administrator prompt or package installation is made.

mod update_support;

use std::sync::Arc;

use ox_core::transfer::Cancellation;
use ox_core::update::{
    Confirmation, FetchError, Installation, PackageCommand, ReleaseVersion, UpdateError, LATEST_RELEASE_URL,
    REPOSITORY,
};
use update_support::{
    failure, install, install_cancellable, mode, next_installer_url, release_answer, success, Response,
    UpdaterFixture, CURRENT, NEXT, PACKAGE,
};

/// A command line as text.
fn argv_text(command: &PackageCommand) -> Vec<String> {
    let argv = command.argv();
    argv.iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect()
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_success_verifies_bytes_and_metadata_before_admin_prompt_and_cleans_up`
/// parity: UPD-003
#[test]
fn success_verifies_bytes_and_metadata_before_admin_prompt_and_cleans_up() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();

    let (result, progress) = install(&updater, NEXT);

    result.unwrap();
    assert_eq!(updater.installed_version(), Some(NEXT));
    assert_eq!(
        fixture.packages.programs(),
        ["/usr/bin/dpkg-deb", "/usr/bin/pkexec", "/usr/bin/dpkg-query"]
    );
    assert_eq!(
        progress,
        [
            "Downloading OpenXplorer 1.0.1…",
            "Approve the system administrator prompt to install. Do not close OpenXplorer…",
            "Update installed. Restart OpenXplorer to use it.",
        ]
    );
    assert_eq!(
        fixture.server.opened(),
        [LATEST_RELEASE_URL.to_owned(), next_installer_url()]
    );
    assert_eq!(mode(&fixture.updates_folder), 0o700);
    fixture.assert_idle_and_clean(&updater);
}

/// The exact commands of an installation: fixed programs, the downloaded
/// installer as the only variable part, and no shell.
/// parity: UPD-003
#[test]
fn installation_runs_only_the_fixed_package_commands() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();

    install(&updater, NEXT).0.unwrap();

    let commands = fixture.packages.commands();
    let installer = fixture.packages.installer_paths()[0].clone();
    let installer_text = installer.to_string_lossy().into_owned();
    let inspect = [
        "/usr/bin/dpkg-deb",
        "-f",
        &installer_text,
        "Package",
        "Version",
        "Architecture",
    ];
    let apt = [
        "/usr/bin/apt-get",
        "-y",
        "--no-remove",
        "install",
        &installer_text,
    ];
    let query = [
        "/usr/bin/dpkg-query",
        "-W",
        "-f=${Status}\n${Version}",
        "openxplorer",
    ];
    assert_eq!(argv_text(&commands[0]), inspect);
    assert_eq!(argv_text(&commands[1])[0], "/usr/bin/pkexec");
    assert_eq!(argv_text(&commands[1])[1..], apt);
    assert_eq!(argv_text(&commands[2]), query);
    assert_eq!(commands[1], PackageCommand::Install(installer));
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_install_requires_literal_confirmation_before_any_network_or_process`
///
/// Python's `False`, `None`, `1`, `'true'`, `[]` and `{}` are all the one
/// value [`Confirmation::Unconfirmed`] here.
/// parity: UPD-003, UPD-005
#[test]
fn install_requires_literal_confirmation_before_any_network_or_process() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();

    let result = updater.install(NEXT, Confirmation::Unconfirmed, &|_| {}, &Cancellation::new());

    let error = result.unwrap_err();
    assert!(error.to_string().starts_with("Confirm"), "{error}");
    assert_eq!(fixture.server.opened(), [LATEST_RELEASE_URL]);
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_install_requires_matching_previously_checked_newer_version`
///
/// Python's hostile versions (`'../../example.deb'`, `'--allow-unauthenticated'`,
/// ...) are not [`ReleaseVersion`]s, so they cannot reach `install`.
/// Python marks the checked release unavailable by hand; here the check
/// finds a release that is not newer.
/// parity: UPD-003
#[test]
fn install_requires_matching_previously_checked_newer_version() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.updater(Installation::DebianPackage);
    let (before_check, _) = install(&updater, NEXT);
    assert!(before_check
        .unwrap_err()
        .to_string()
        .contains("Check for updates"));

    updater.check(&Cancellation::new()).unwrap();
    let hostile = [
        "../../example.deb",
        "/tmp/example.deb",
        "--allow-unauthenticated",
        "1.0.1; echo example",
    ];
    assert!(hostile.iter().all(|text| text.parse::<ReleaseVersion>().is_err()));
    assert!(matches!(
        install(&updater, CURRENT).0,
        Err(UpdateError::NotChecked)
    ));

    fixture
        .server
        .answer_release(Response::json(&release_answer(CURRENT, REPOSITORY)));
    updater.check(&Cancellation::new()).unwrap();
    assert!(matches!(
        install(&updater, CURRENT).0,
        Err(UpdateError::NotChecked)
    ));
    assert!(matches!(install(&updater, NEXT).0, Err(UpdateError::NotChecked)));

    assert_eq!(fixture.server.opened(), [LATEST_RELEASE_URL, LATEST_RELEASE_URL]);
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// A download that does not match the release, and the error it must give.
struct DownloadCase {
    name: &'static str,
    payload: Vec<u8>,
    is_expected: fn(&UpdateError) -> bool,
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_bad_checksum_short_download_and_oversized_download_never_prompt`
/// parity: UPD-003
#[test]
fn bad_checksum_short_download_and_oversized_download_never_prompt() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    let cases = [
        DownloadCase {
            name: "wrong bytes",
            payload: vec![b'X'; PACKAGE.len()],
            is_expected: |error| matches!(error, UpdateError::ChecksumMismatch),
        },
        DownloadCase {
            name: "short",
            payload: PACKAGE[..PACKAGE.len() - 1].to_vec(),
            is_expected: |error| matches!(error, UpdateError::ChecksumMismatch),
        },
        DownloadCase {
            name: "oversized",
            payload: [PACKAGE, b"oversize"].concat(),
            is_expected: |error| matches!(error, UpdateError::InstallerTooLarge),
        },
    ];

    for case in cases {
        fixture.server.answer_installer(Response::Body(case.payload));

        let (result, _) = install(&updater, NEXT);

        let error = result.unwrap_err();
        assert!((case.is_expected)(&error), "{}: {error:?}", case.name);
        assert!(fixture.packages.commands().is_empty());
        assert_eq!(updater.installed_version(), None);
        fixture.assert_idle_and_clean(&updater);
    }
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_wrong_package_identity_version_or_architecture_never_prompt`
/// parity: UPD-003
#[test]
fn wrong_package_identity_version_or_architecture_never_prompt() {
    let wrong_fields = [
        "Package: different\nVersion: 1.0.1\nArchitecture: all\n",
        "Package: openxplorer\nVersion: 1.0.0\nArchitecture: all\n",
        "Package: openxplorer\nVersion: 1.0.1\nArchitecture: amd64\n",
        "Package: openxplorer\nVersion: 1.0.1\n",
    ];
    for fields in wrong_fields {
        let fixture = UpdaterFixture::new();
        let updater = fixture.checked_updater();
        fixture.packages.answer_inspection(success(fields));

        let (result, _) = install(&updater, NEXT);

        let error = result.unwrap_err();
        assert!(error.to_string().contains("metadata"), "{fields:?}: {error}");
        assert_eq!(fixture.packages.programs(), ["/usr/bin/dpkg-deb"]);
        fixture.assert_idle_and_clean(&updater);
    }
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_download_failure_cleans_partial_files_and_releases_lock`
/// parity: UPD-003
#[test]
fn download_failure_cleans_partial_files_and_releases_lock() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    fixture
        .server
        .answer_installer(Response::BrokenAfter(PACKAGE[..10].to_vec()));

    let (result, _) = install(&updater, NEXT);

    assert!(matches!(result, Err(UpdateError::Unreachable)), "{result:?}");
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// A download refused with an HTTP error says so and installs nothing.
/// parity: UPD-003
#[test]
fn a_refused_download_names_its_http_status() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    fixture
        .server
        .answer_installer(Response::Refused(FetchError::Status(404)));

    let (result, _) = install(&updater, NEXT);

    let error = result.unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub could not provide the installer (HTTP 404). Nothing was installed."
    );
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_polkit_cancellation_or_apt_failure_cleans_up_without_marking_installed`
/// parity: UPD-003
#[test]
fn polkit_cancellation_or_apt_failure_cleans_up_without_marking_installed() {
    for exit_status in [126, 127, 100] {
        let fixture = UpdaterFixture::new();
        let updater = fixture.checked_updater();
        fixture
            .packages
            .answer_installation(failure(exit_status, "Fictional refusal"));

        let (result, _) = install(&updater, NEXT);

        let error = result.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Installation was cancelled or failed. Fictional refusal"
        );
        assert_eq!(
            fixture.packages.programs(),
            ["/usr/bin/dpkg-deb", "/usr/bin/pkexec"]
        );
        assert_eq!(updater.installed_version(), None);
        fixture.assert_idle_and_clean(&updater);
    }
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_package_inspection_failure_cleans_up_and_does_not_prompt`
/// parity: UPD-003
#[test]
fn package_inspection_failure_cleans_up_and_does_not_prompt() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    fixture.packages.answer_inspection(failure(2, "dpkg-deb: error"));

    let (result, _) = install(&updater, NEXT);

    assert!(matches!(
        result,
        Err(UpdateError::PackageToolFailed { status: 2, .. })
    ));
    assert_eq!(fixture.packages.programs(), ["/usr/bin/dpkg-deb"]);
    fixture.assert_idle_and_clean(&updater);
}

/// Ported from `desktop/tests/test_updater.py::UpdaterTests::test_installed_version_is_verified_after_package_manager_returns_success`
/// parity: UPD-003
#[test]
fn installed_version_is_verified_after_package_manager_returns_success() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    fixture
        .packages
        .answer_query(success(&format!("install ok installed\n{CURRENT}")));

    let (result, _) = install(&updater, NEXT);

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("expected installed version"),
        "{error}"
    );
    assert_eq!(updater.installed_version(), None);
    fixture.assert_idle_and_clean(&updater);
}

/// Cancelling an installation stops it before the administrator prompt.
/// parity: UPD-003
#[test]
fn a_cancelled_installation_never_prompts() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    let cancel = Cancellation::new();
    cancel.cancel();

    let result = updater.install(NEXT, Confirmation::Confirmed, &|_| {}, &cancel);

    assert!(matches!(result, Err(UpdateError::Cancelled)));
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

/// Cancelling while `dpkg-deb` inspects the verified download still stops
/// the installation before the administrator prompt: the last moment
/// cancelling can.
/// parity: UPD-003
#[test]
fn cancelling_during_the_inspection_never_prompts() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_updater();
    let cancel = Cancellation::new();
    let cancel_during_inspection = cancel.clone();
    fixture
        .packages
        .on_inspection(Arc::new(move || cancel_during_inspection.cancel()));

    let (result, progress) = install_cancellable(&updater, NEXT, &cancel);

    assert!(matches!(result, Err(UpdateError::Cancelled)), "{result:?}");
    assert_eq!(fixture.packages.programs(), ["/usr/bin/dpkg-deb"]);
    assert_eq!(progress, ["Downloading OpenXplorer 1.0.1…"]);
    assert_eq!(updater.installed_version(), None);
    fixture.assert_idle_and_clean(&updater);
}
