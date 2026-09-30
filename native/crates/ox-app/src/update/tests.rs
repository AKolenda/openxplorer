// SPDX-License-Identifier: AGPL-3.0-only
//! The updater as the interface uses it, against a simulated GitHub,
//! package manager and restart launcher: nothing is downloaded, installed
//! or restarted.
//!
//! Ports the dialog behaviour of `updatesDialog` in `desktop/ui/app.js`
//! and the doubles of `desktop/tests/test_rc2.py` (`UpdaterTests`), whose
//! release, `dpkg-deb`, APT and `dpkg-query` answers these fakes give.

use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex, PoisonError};

use gtk::glib;
use gtk::prelude::*;
use ox_core::transfer::Cancellation;
use ox_core::update::{
    installer_name, CommandOutput, FetchError, Installation, InstalledBuild, PackageCommand, PackageManager,
    ReleaseServer, ReleaseVersion, RestartLauncher, RuntimeIdentity, TrustedUrl, UpdateError, UpdateService,
    UpdateServiceParts, Updater, UpdaterParts, LATEST_RELEASE_URL, REPOSITORY, RUNTIME_PROTOCOL,
};
use tempfile::TempDir;

use super::{UpdateDialog, UpdateState, Updates};
use crate::test_support::harness::{capture_dialog, settle, wait_until, Fixture, TestWindow};

/// The running version in the simulation.
const RUNNING: ReleaseVersion = ReleaseVersion::new(1, 0, 0);
/// The version the simulated GitHub publishes.
const NEXT: ReleaseVersion = ReleaseVersion::new(1, 0, 1);
/// The simulated installer's bytes; never a real package.
const PACKAGE: &[u8] = b"Fictional package bytes. Never an executable Debian package.\n";

/// GitHub's answer for the release of [`NEXT`] with the simulated
/// installer, as Python's `release()` fixture.
fn release_answer() -> String {
    let name = installer_name(NEXT);
    let digest =
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, PACKAGE).expect("GLib computes SHA-256");
    let size = PACKAGE.len();
    format!(
        r#"{{"tag_name": "v{NEXT}", "draft": false, "prerelease": false, "body": "Notes.",
            "assets": [{{"name": "{name}",
              "browser_download_url": "{REPOSITORY}/releases/download/v{NEXT}/{name}",
              "digest": "sha256:{digest}", "size": {size}}}]}}"#
    )
}

/// A GitHub that answers the release and its installer.
#[derive(Debug, Clone, Copy)]
struct SimulatedGitHub;

impl ReleaseServer for SimulatedGitHub {
    fn open(&self, url: &TrustedUrl, _cancel: &Cancellation) -> Result<Box<dyn Read>, FetchError> {
        let body = if url.as_str() == LATEST_RELEASE_URL {
            release_answer().into_bytes()
        } else {
            PACKAGE.to_vec()
        };
        Ok(Box::new(Cursor::new(body)))
    }
}

/// A package manager that answers like a successful installation of
/// [`NEXT`], records every command, and can hold APT until the test lets
/// it go on.
#[derive(Debug, Clone)]
struct SimulatedPackages {
    commands: Arc<Mutex<Vec<&'static str>>>,
    hold_installation: Arc<Mutex<Option<Receiver<()>>>>,
}

impl SimulatedPackages {
    fn new() -> Self {
        Self {
            commands: Arc::default(),
            hold_installation: Arc::default(),
        }
    }

    /// The programs run so far.
    fn programs(&self) -> Vec<&'static str> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn success(stdout: &str) -> CommandOutput {
    CommandOutput {
        exit_status: 0,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

impl PackageManager for SimulatedPackages {
    fn run(&self, command: &PackageCommand) -> Result<CommandOutput, UpdateError> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command.program());
        let output = match command {
            PackageCommand::InspectInstaller(_) => success(&format!(
                "Package: openxplorer\nVersion: {NEXT}\nArchitecture: all\n"
            )),
            PackageCommand::Install(_) => {
                let hold = self
                    .hold_installation
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(hold) = hold {
                    // The test lets APT go on; a dropped sender does too.
                    let _ = hold.recv();
                }
                success("")
            }
            PackageCommand::QueryInstalled => success(&format!("install ok installed\n{NEXT}")),
        };
        Ok(output)
    }
}

/// A restart launcher that records what it would start.
#[derive(Debug, Clone, Default)]
struct RecordedLaunches(Arc<Mutex<Vec<Vec<String>>>>);

impl RestartLauncher for RecordedLaunches {
    fn launch(&self, argv: &[&str]) -> Result<(), UpdateError> {
        let argv = argv.iter().map(|word| (*word).to_owned()).collect();
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(argv);
        Ok(())
    }
}

/// The simulated update service of a build installed as `installation`.
struct SimulatedUpdates {
    updates: Updates,
    packages: SimulatedPackages,
    launches: RecordedLaunches,
    /// Holds the downloads and the "installed" executable.
    _root: TempDir,
}

impl SimulatedUpdates {
    fn new(installation: Installation) -> Self {
        let root = tempfile::tempdir().expect("a temporary folder");
        let installed = root.path().join("openxplorer");
        std::fs::write(&installed, b"installed build").expect("the temporary folder is writable");
        let packages = SimulatedPackages::new();
        let launches = RecordedLaunches::default();
        let service = UpdateService::new(UpdateServiceParts {
            updater: Updater::new(UpdaterParts {
                current_version: RUNNING,
                installation,
                updates_folder: root.path().join("updates"),
                server: Box::new(SimulatedGitHub),
                package_manager: Box::new(packages.clone()),
            }),
            running: running_identity(),
            installed_build: installed_build(&installed),
            launcher: Box::new(launches.clone()),
        });
        Self {
            updates: Updates::with_service(service),
            packages,
            launches,
            _root: root,
        }
    }

    /// A window using these updates.
    fn window(&self, fixture: &Fixture) -> TestWindow {
        let updates = self.updates.clone();
        TestWindow::open_prepared(&fixture.uri(), move |context| context.use_updates(updates))
    }
}

/// The running build, which no installed file matches.
fn running_identity() -> RuntimeIdentity {
    RuntimeIdentity {
        version: RUNNING.to_string(),
        protocol: RUNTIME_PROTOCOL,
        build: "the running build".to_owned(),
    }
}

fn installed_build(path: &Path) -> InstalledBuild {
    InstalledBuild::Executable {
        path: path.to_owned(),
        version: NEXT.to_string(),
    }
}

/// Opens the dialog over `test`'s window, which checks at once, and waits
/// for the check.
fn open_dialog(test: &TestWindow, updates: &Updates) -> UpdateDialog {
    let dialog = UpdateDialog::present_for(&test.window, updates, || ox_core::update::Activity::Idle);
    wait_until("the check", || !updates.state().is_busy());
    dialog
}

/// "Check for updates" checks GitHub at once and offers the newer release
/// the original repository publishes; the status bar and About say so in
/// every window.
///
/// parity: UPD-001, UPD-002
#[gtk::test]
fn checking_offers_a_newer_release_and_the_status_bar_says_so() {
    let fixture = Fixture::standard();
    let simulated = SimulatedUpdates::new(Installation::DebianPackage);
    let test = simulated.window(&fixture);

    let dialog = open_dialog(&test, &simulated.updates);

    assert_eq!(dialog.status(), "An update is available.");
    assert_eq!(
        dialog.shown_buttons(),
        [
            ("Close".to_owned(), true),
            ("Check again".to_owned(), true),
            ("Install update…".to_owned(), true),
        ]
    );
    let tooltip = test.window.status_bar_update_tooltip();
    assert_eq!(tooltip, "Check for updates\nOpenXplorer 1.0.1 is available.");
    capture_dialog(&dialog, "native-update-available.png");
    dialog.close();
}

/// Install update… downloads and verifies the installer, runs the fixed
/// package commands in order, and then offers only Restart now, which
/// starts the installed launcher. Until the restart no window opens.
///
/// parity: UPD-003, UPD-006, UPD-007
#[gtk::test]
fn installing_verifies_installs_and_offers_the_restart() {
    let fixture = Fixture::standard();
    let simulated = SimulatedUpdates::new(Installation::DebianPackage);
    let test = simulated.window(&fixture);
    let dialog = open_dialog(&test, &simulated.updates);

    dialog.click("Install update…");
    wait_until("the installation", || !simulated.updates.state().is_installing());

    assert_eq!(
        simulated.updates.state(),
        UpdateState::Installed { version: NEXT }
    );
    assert_eq!(
        dialog.status(),
        "OpenXplorer 1.0.1 is installed. Restart to use the update."
    );
    assert_eq!(
        simulated.packages.programs(),
        ["/usr/bin/dpkg-deb", "/usr/bin/pkexec", "/usr/bin/dpkg-query"]
    );
    assert_eq!(
        dialog.shown_buttons(),
        [
            ("Close".to_owned(), true),
            ("Check again".to_owned(), false),
            ("Restart now".to_owned(), true),
        ]
    );
    assert_eq!(
        simulated.updates.new_window_refusal().as_deref(),
        Some("Finish the application update and restart before opening another window.")
    );
    assert_eq!(
        simulated.updates.file_refusal().as_deref(),
        Some("Restart OpenXplorer to finish the application update before using files.")
    );
    dialog.click("Restart now");
    let launched = simulated
        .launches
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(launched, [["/usr/bin/openxplorer", "--restart"]]);
    dialog.close();
}

/// While the package manager runs, every window is locked: it cannot be
/// closed or used, and the dialog cannot be dismissed.
///
/// parity: UPD-005
#[gtk::test]
fn an_installing_update_locks_every_window() {
    let fixture = Fixture::standard();
    let simulated = SimulatedUpdates::new(Installation::DebianPackage);
    let (let_apt_finish, hold) = channel();
    simulated
        .packages
        .hold_installation
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(hold);
    let test = simulated.window(&fixture);
    let dialog = open_dialog(&test, &simulated.updates);

    dialog.click("Install update…");
    wait_until("APT to start", || {
        simulated.packages.programs().contains(&"/usr/bin/pkexec")
    });
    test.window.close();
    dialog.close();
    settle();

    assert!(test.window.is_visible(), "the window stays open");
    assert!(!test.window.is_sensitive(), "the window is locked");
    assert!(dialog.is_visible(), "the dialog stays open");
    assert_eq!(
        test.window.shown_message_text(),
        "Wait for the application update to finish before closing."
    );
    let_apt_finish.send(()).expect("APT is waiting");
    wait_until("the installation", || !simulated.updates.state().is_installing());
    assert!(test.window.is_sensitive(), "the window is usable again");
    dialog.close();
}

/// Install update… is refused while a file operation runs in any window,
/// before anything is downloaded or installed.
///
/// parity: UPD-005
#[gtk::test]
fn installing_waits_for_file_operations() {
    let fixture = Fixture::standard();
    let simulated = SimulatedUpdates::new(Installation::DebianPackage);
    let test = simulated.window(&fixture);
    let dialog = UpdateDialog::present_for(&test.window, &simulated.updates, || {
        ox_core::update::Activity::Busy
    });
    wait_until("the check", || !simulated.updates.state().is_busy());

    dialog.click("Install update…");

    assert_eq!(
        dialog.status(),
        "Finish file operations before installing the update."
    );
    assert!(!simulated.updates.state().is_installing());
    assert!(simulated.packages.programs().is_empty(), "nothing was installed");
    dialog.close();
}

/// A Flatpak never installs updates itself: the dialog says the release
/// exists and tells how Flatpak updates it.
///
/// parity: UPD-004
#[gtk::test]
fn a_flatpak_is_told_to_update_through_flatpak() {
    let fixture = Fixture::standard();
    let simulated = SimulatedUpdates::new(Installation::Flatpak);
    let test = simulated.window(&fixture);

    let dialog = open_dialog(&test, &simulated.updates);

    assert_eq!(
        dialog.status(),
        "An update is available. Automatic installation is unavailable."
    );
    let hint = dialog.hint().expect("the dialog says what updates a Flatpak");
    assert!(hint.contains("flatpak update"), "{hint}");
    assert!(
        dialog
            .shown_buttons()
            .contains(&("Install update…".to_owned(), false)),
        "Install update… is shown but cannot be used"
    );
    capture_dialog(&dialog, "native-update-flatpak.png");
    dialog.close();
    assert!(simulated.packages.programs().is_empty(), "nothing was installed");
}
