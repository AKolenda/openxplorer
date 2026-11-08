// SPDX-License-Identifier: AGPL-3.0-only
//! Installing an update through the update helper: the app's side, which
//! hands the helper only the version, and the helper's side, which
//! fetches, verifies and installs that release as root. New in the native
//! app; the Python app ran `pkexec apt-get` itself.
//!
//! The helper's side runs here in-process as the ordinary test user,
//! with a fixture GitHub and package manager, so its refusals are proven
//! without anything becoming root, reaching the network or installing a
//! package. The polkit action and rule that let an administrator run it
//! without a password are checked by `native/tools/test_package_data.py`.

mod update_support;

use std::cell::RefCell;
use std::sync::Arc;

use ox_core::transfer::Cancellation;
use ox_core::update::{
    install_as_root, HelperFailure, HelperReport, PackageCommand, ReleaseVersion, RootInstallParts,
    UpdateError, LATEST_RELEASE_URL,
};
use update_support::{
    failure, helper_output, install, install_cancellable, mode, next_installer_url, release_answer, success,
    PackageFixture, Response, UpdaterFixture, CURRENT, HELPER, NEXT,
};

/// A command line as text.
fn argv_text(command: &PackageCommand) -> Vec<String> {
    command
        .argv()
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect()
}

/// `dpkg-query`'s answer for an installed `version`.
fn installed(version: ReleaseVersion) -> ox_core::update::CommandOutput {
    success(&format!("install ok installed\n{version}"))
}

// The app's side.

/// The app runs only `pkexec <helper> <version>` and then asks
/// `dpkg-query` itself: it downloads nothing and runs no `pkexec apt-get`,
/// and the dialog shows the same steps as before.
/// parity: UPD-003
#[test]
fn the_app_hands_the_helper_only_the_version() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_helper_updater();

    let (result, progress) = install(&updater, NEXT);

    result.unwrap();
    assert_eq!(updater.installed_version(), Some(NEXT));
    let commands = fixture.packages.commands();
    assert_eq!(
        fixture.packages.programs(),
        ["/usr/bin/pkexec", "/usr/bin/dpkg-query"]
    );
    assert_eq!(argv_text(&commands[0]), ["/usr/bin/pkexec", HELPER, "1.0.1"]);
    assert_eq!(commands[0].time_limit(), None, "APT is never interrupted");
    assert_eq!(
        fixture.server.opened(),
        [LATEST_RELEASE_URL],
        "the app downloads nothing"
    );
    assert_eq!(
        progress,
        [
            "Downloading OpenXplorer 1.0.1…",
            "Approve the system administrator prompt to install. Do not close OpenXplorer…",
            "Update installed. Restart OpenXplorer to use it.",
        ]
    );
    fixture.assert_idle_and_clean(&updater);
}

/// The helper's refusals read exactly as the app's own checks did.
/// parity: UPD-003
#[test]
fn a_helper_failure_reads_as_the_apps_own_error() {
    let cases = [
        (HelperFailure::ChecksumMismatch, UpdateError::ChecksumMismatch),
        (HelperFailure::MetadataMismatch, UpdateError::MetadataMismatch),
        (HelperFailure::NotChecked, UpdateError::NotChecked),
        (HelperFailure::Cancelled, UpdateError::Cancelled),
        (
            HelperFailure::DownloadRefused { status: 404 },
            UpdateError::DownloadRefused { status: 404 },
        ),
    ];
    for (failure, expected) in cases {
        let fixture = UpdaterFixture::new();
        let updater = fixture.checked_helper_updater();
        let reports = [HelperReport::Downloading, HelperReport::Failed(failure)];
        fixture.packages.answer_helper(helper_output(1, &reports));

        let (result, _) = install(&updater, NEXT);

        assert_eq!(result.unwrap_err().to_string(), expected.to_string());
        assert_eq!(updater.installed_version(), None);
        assert_eq!(fixture.packages.programs(), ["/usr/bin/pkexec"], "{expected}");
        fixture.assert_idle_and_clean(&updater);
    }
}

/// A refused or dismissed administrator prompt (a user who is not an
/// administrator) fails as a refused `pkexec apt-get` did.
/// parity: UPD-003
#[test]
fn a_refused_prompt_fails_as_before() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_helper_updater();
    fixture.packages.answer_helper(failure(
        126,
        "Error executing command as another user: Not authorized",
    ));

    let (result, progress) = install(&updater, NEXT);

    assert_eq!(
        result.unwrap_err().to_string(),
        "Installation was cancelled or failed. Error executing command as another user: Not authorized"
    );
    assert_eq!(updater.installed_version(), None);
    assert_eq!(progress, ["Downloading OpenXplorer 1.0.1…"]);
    fixture.assert_idle_and_clean(&updater);
}

/// A helper that cannot reach GitHub as root (a proxy only the user's
/// session knows) leaves the installation to the app's own download and
/// `pkexec apt-get`, which asks for the password as before.
/// parity: UPD-003
#[test]
fn a_helper_that_cannot_reach_github_falls_back_to_the_apps_download() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_helper_updater();
    let reports = [HelperReport::Failed(HelperFailure::Unreachable)];
    fixture.packages.answer_helper(helper_output(1, &reports));

    let (result, progress) = install(&updater, NEXT);

    result.unwrap();
    assert_eq!(
        fixture.packages.programs(),
        [
            "/usr/bin/pkexec",
            "/usr/bin/dpkg-deb",
            "/usr/bin/pkexec",
            "/usr/bin/dpkg-query"
        ]
    );
    let installer = fixture.packages.installer_paths()[1].clone();
    assert_eq!(fixture.packages.commands()[2], PackageCommand::Install(installer));
    assert_eq!(
        fixture.server.opened(),
        [LATEST_RELEASE_URL.to_owned(), next_installer_url()]
    );
    assert_eq!(progress.len(), 3, "{progress:?}");
    fixture.assert_idle_and_clean(&updater);
}

/// The helper's word is not enough: `dpkg-query` must confirm the
/// version, and a helper that ends without a report, or reports another
/// version, installed nothing the app accepts.
/// parity: UPD-003
#[test]
fn success_still_needs_the_package_managers_confirmation() {
    let unconfirmed = UpdaterFixture::new();
    let updater = unconfirmed.checked_helper_updater();
    unconfirmed.packages.answer_query(installed(CURRENT));
    assert!(matches!(
        install(&updater, NEXT).0,
        Err(UpdateError::InstallNotConfirmed)
    ));
    assert_eq!(updater.installed_version(), None);

    let silent = UpdaterFixture::new();
    let updater = silent.checked_helper_updater();
    silent.packages.answer_helper(success(""));
    assert!(matches!(
        install(&updater, NEXT).0,
        Err(UpdateError::InstallNotConfirmed)
    ));
    assert_eq!(silent.packages.programs(), ["/usr/bin/pkexec"]);

    let other_version = UpdaterFixture::new();
    let updater = other_version.checked_helper_updater();
    let reports = [HelperReport::Installed(ReleaseVersion::new(1, 0, 2))];
    other_version.packages.answer_helper(helper_output(0, &reports));
    assert!(install(&updater, NEXT).0.is_err());
    assert_eq!(updater.installed_version(), None);
}

/// Cancelling before the helper starts runs nothing.
/// parity: UPD-003
#[test]
fn a_cancelled_installation_never_starts_the_helper() {
    let fixture = UpdaterFixture::new();
    let updater = fixture.checked_helper_updater();
    let cancel = Cancellation::new();
    cancel.cancel();

    let (result, _) = install_cancellable(&updater, NEXT, &cancel);

    assert!(matches!(result, Err(UpdateError::Cancelled)));
    assert!(fixture.packages.commands().is_empty());
    fixture.assert_idle_and_clean(&updater);
}

// The helper's side, as root.

/// The helper's fixtures: the installed [`CURRENT`], the fixture release
/// of [`NEXT`], and a staging folder.
struct RootFixture {
    updates: UpdaterFixture,
}

impl RootFixture {
    fn new() -> Self {
        let updates = UpdaterFixture::new();
        updates.packages.answer_next_query(installed(CURRENT));
        Self { updates }
    }

    fn packages(&self) -> &PackageFixture {
        &self.updates.packages
    }

    /// Runs the helper's installation of `requested`, collecting its
    /// reports.
    fn install(
        &self,
        requested: ReleaseVersion,
        cancel: &Cancellation,
    ) -> (Result<(), UpdateError>, Vec<HelperReport>) {
        let reports = RefCell::new(Vec::new());
        let parts = RootInstallParts {
            server: &self.updates.server,
            package_manager: &self.updates.packages,
            staging_folder: &self.updates.updates_folder,
        };
        let result = install_as_root(
            &parts,
            requested,
            &mut |report| reports.borrow_mut().push(report),
            cancel,
        );
        (result, reports.into_inner())
    }

    /// No download is left in the staging folder, and every installer a
    /// command named is gone.
    fn assert_clean(&self) {
        let folder = &self.updates.updates_folder;
        if folder.exists() {
            assert_eq!(
                std::fs::read_dir(folder).unwrap().count(),
                0,
                "no download is left"
            );
        }
        for installer in self.packages().installer_paths() {
            assert!(!installer.exists(), "{} was deleted", installer.display());
        }
    }
}

/// The helper asks what is installed, fetches the latest release itself,
/// downloads it privately, checks it with `dpkg-deb`, runs APT without any
/// input, and reports success only when `dpkg-query` confirms it.
/// parity: UPD-003
#[test]
fn the_helper_verifies_the_latest_release_before_apt_runs() {
    let fixture = RootFixture::new();

    let (result, reports) = fixture.install(NEXT, &Cancellation::new());

    result.unwrap();
    assert_eq!(
        reports,
        [
            HelperReport::Downloading,
            HelperReport::Installing,
            HelperReport::Installed(NEXT)
        ]
    );
    assert_eq!(
        fixture.packages().programs(),
        [
            "/usr/bin/dpkg-query",
            "/usr/bin/dpkg-deb",
            "/usr/bin/apt-get",
            "/usr/bin/dpkg-query"
        ]
    );
    let commands = fixture.packages().commands();
    let installer = fixture.packages().installer_paths()[0]
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        argv_text(&commands[2]),
        [
            "/usr/bin/apt-get",
            "-y",
            "--no-remove",
            "install",
            installer.as_str()
        ]
    );
    assert!(!commands[2].reads_input(), "APT reads nothing from the caller");
    assert_eq!(
        fixture.updates.server.opened(),
        [LATEST_RELEASE_URL.to_owned(), next_installer_url()]
    );
    assert_eq!(mode(&fixture.updates.updates_folder), 0o700);
    fixture.assert_clean();
}

/// Only the latest release installs: a newer-looking version GitHub does
/// not publish as latest is refused before any download.
/// parity: UPD-003
#[test]
fn the_helper_refuses_a_version_that_is_not_the_latest_release() {
    let fixture = RootFixture::new();

    let (result, reports) = fixture.install(ReleaseVersion::new(1, 0, 2), &Cancellation::new());

    assert!(matches!(result, Err(UpdateError::NotChecked)), "{result:?}");
    assert!(reports.is_empty());
    assert_eq!(fixture.updates.server.opened(), [LATEST_RELEASE_URL]);
    assert_eq!(fixture.packages().programs(), ["/usr/bin/dpkg-query"]);
    fixture.assert_clean();
}

/// Never a downgrade or a reinstallation: a version that is not newer
/// than the installed package is refused before anything is fetched,
/// even when GitHub's latest release is that version.
/// parity: UPD-003
#[test]
fn the_helper_never_downgrades_or_reinstalls() {
    for installed_version in [NEXT, ReleaseVersion::new(1, 0, 2), ReleaseVersion::new(2, 0, 0)] {
        let fixture = UpdaterFixture::new();
        fixture.packages.answer_next_query(installed(installed_version));
        let root = RootFixture { updates: fixture };

        let (result, _) = root.install(NEXT, &Cancellation::new());

        assert!(
            matches!(result, Err(UpdateError::NotChecked)),
            "{installed_version}"
        );
        assert!(root.updates.server.opened().is_empty(), "{installed_version}");
        assert_eq!(root.packages().programs(), ["/usr/bin/dpkg-query"]);
    }
}

/// The helper only updates an installed `openxplorer` package with a
/// plain release version.
/// parity: UPD-003
#[test]
fn the_helper_refuses_when_openxplorer_is_not_installed() {
    let answers = [
        failure(1, "dpkg-query: no packages found matching openxplorer"),
        success("deinstall ok config-files\n1.0.0"),
        success("install ok installed\n1.0.0-1ubuntu1"),
        success(""),
    ];
    for answer in answers {
        let fixture = UpdaterFixture::new();
        fixture.packages.answer_next_query(answer.clone());
        let root = RootFixture { updates: fixture };

        let (result, _) = root.install(NEXT, &Cancellation::new());

        assert!(
            matches!(result, Err(UpdateError::InstallUnavailable { .. })),
            "{answer:?}: {result:?}"
        );
        assert!(root.updates.server.opened().is_empty());
    }
}

/// A release whose installer is not published by the original repository
/// or the openxplorer organisation is never downloaded.
/// parity: UPD-003, SAFE-016
#[test]
fn the_helper_refuses_an_installer_from_elsewhere() {
    let fixture = RootFixture::new();
    let foreign = release_answer(NEXT, "https://github.com/someone-else/openxplorer");
    fixture.updates.server.answer_release(Response::json(&foreign));

    let (result, reports) = fixture.install(NEXT, &Cancellation::new());

    assert!(matches!(result, Err(UpdateError::MissingInstaller)), "{result:?}");
    assert!(reports.is_empty());
    assert_eq!(fixture.updates.server.opened(), [LATEST_RELEASE_URL]);
}

/// Bytes that are not the release's never reach `dpkg-deb` or APT.
/// parity: UPD-003
#[test]
fn the_helper_refuses_a_download_that_is_not_the_releases() {
    let fixture = RootFixture::new();
    fixture.updates.server.answer_installer(Response::Body(
        b"Fictional other bytes of a tampered download.\n".to_vec(),
    ));

    let (result, reports) = fixture.install(NEXT, &Cancellation::new());

    assert!(matches!(result, Err(UpdateError::ChecksumMismatch)), "{result:?}");
    assert_eq!(reports, [HelperReport::Downloading]);
    assert_eq!(fixture.packages().programs(), ["/usr/bin/dpkg-query"]);
    fixture.assert_clean();
}

/// A package that `dpkg-deb` says is not `openxplorer` at that version is
/// never installed.
/// parity: UPD-003
#[test]
fn the_helper_refuses_another_package() {
    let fields = [
        format!("Package: evil\nVersion: {NEXT}\nArchitecture: all\n"),
        format!("Package: openxplorer\nVersion: {CURRENT}\nArchitecture: all\n"),
        format!("Package: openxplorer\nVersion: {NEXT}\nArchitecture: amd64\n"),
    ];
    for answer in fields {
        let fixture = RootFixture::new();
        fixture.packages().answer_inspection(success(&answer));

        let (result, _) = fixture.install(NEXT, &Cancellation::new());

        assert!(matches!(result, Err(UpdateError::MetadataMismatch)), "{answer}");
        assert_eq!(
            fixture.packages().programs(),
            ["/usr/bin/dpkg-query", "/usr/bin/dpkg-deb"]
        );
        fixture.assert_clean();
    }
}

/// The app's `cancel` stops the helper up to the moment APT would start.
/// parity: UPD-003
#[test]
fn a_cancelled_helper_stops_before_apt() {
    let fixture = RootFixture::new();
    let cancel = Cancellation::new();
    let cancelling = cancel.clone();
    fixture
        .packages()
        .on_inspection(Arc::new(move || cancelling.cancel()));

    let (result, reports) = fixture.install(NEXT, &cancel);

    assert!(matches!(result, Err(UpdateError::Cancelled)));
    assert_eq!(reports, [HelperReport::Downloading]);
    assert!(!fixture.packages().programs().contains(&"/usr/bin/apt-get"));
    fixture.assert_clean();
}

/// APT's failure, and a version `dpkg-query` does not confirm, are
/// failures, never an installed update.
/// parity: UPD-003
#[test]
fn the_helper_reports_success_only_when_dpkg_confirms_it() {
    let apt_failed = RootFixture::new();
    apt_failed.packages().answer_installation(failure(
        100,
        "E: Sub-process /usr/bin/dpkg returned an error code (1)",
    ));
    let (result, reports) = apt_failed.install(NEXT, &Cancellation::new());
    assert!(
        matches!(result, Err(UpdateError::InstallFailed { .. })),
        "{result:?}"
    );
    assert!(!reports.contains(&HelperReport::Installed(NEXT)));
    apt_failed.assert_clean();

    let unconfirmed = RootFixture::new();
    unconfirmed.packages().answer_query(installed(CURRENT));
    let (result, reports) = unconfirmed.install(NEXT, &Cancellation::new());
    assert!(
        matches!(result, Err(UpdateError::InstallNotConfirmed)),
        "{result:?}"
    );
    assert!(!reports.contains(&HelperReport::Installed(NEXT)));
}
