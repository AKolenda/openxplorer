// SPDX-License-Identifier: AGPL-3.0-only
//! Checking for and installing one update at a time. Ports `Updater` in
//! `v2.0.0:desktop/updater.py`.
//!
//! Both operations block on the network and on processes; run them on a
//! worker thread, as [`UpdateService`](super::UpdateService) does.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};

use super::download::download_installer;
use super::package::{check_installed_version, check_installer_fields, installation_failure};
use super::server::read_release_answer;
use super::{
    parse_release, CommandOutput, GitHubReleases, HelperReport, Installation, PackageCommand, PackageManager,
    Release, ReleaseServer, ReleaseVersion, SystemPackageManager, UpdateError,
};
use crate::transfer::Cancellation;

/// What a check found, without the installer's address, digest, size or
/// name, which never leave the updater.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    /// The running version.
    pub current_version: ReleaseVersion,
    /// The latest release's version.
    pub latest_version: ReleaseVersion,
    /// The latest release is newer.
    pub is_available: bool,
    /// Its release notes, at most 20,000 characters. The update dialog
    /// does not show them.
    pub notes: String,
    /// Its release page.
    pub release_url: String,
    /// This build may install it (see [`Installation::can_install`]).
    pub can_install: bool,
    /// An update was installed and waits for a restart.
    pub restart_required: bool,
}

/// Whether the user confirmed the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// The user chose "Install update…" and confirmed.
    Confirmed,
    /// Anything else.
    Unconfirmed,
}

/// A step of an installation, reported as it starts. `Display` is the
/// progress message the update dialog shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProgress {
    /// Downloading and verifying the installer.
    Downloading(ReleaseVersion),
    /// Waiting for the administrator prompt and APT.
    AwaitingApproval,
    /// The package manager confirmed the new version.
    Installed,
}

impl fmt::Display for InstallProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Downloading(version) => write!(formatter, "Downloading OpenXplorer {version}…"),
            Self::AwaitingApproval => formatter
                .write_str("Approve the system administrator prompt to install. Do not close OpenXplorer…"),
            Self::Installed => formatter.write_str("Update installed. Restart OpenXplorer to use it."),
        }
    }
}

/// The parts an [`Updater`] is made of.
pub struct UpdaterParts {
    /// The running version.
    pub current_version: ReleaseVersion,
    /// How this build was installed.
    pub installation: Installation,
    /// Where downloads are kept while they are verified and installed,
    /// normally `~/.cache/winspace/updates`.
    pub updates_folder: PathBuf,
    /// Where releases are fetched from.
    pub server: Box<dyn ReleaseServer>,
    /// What runs the package tools.
    pub package_manager: Box<dyn PackageManager>,
    /// The update helper the package installed
    /// ([`Installation::install_helper`]), or `None` to run `pkexec
    /// apt-get` on the app's own download, as a package without one does.
    pub install_helper: Option<PathBuf>,
}

/// Checks GitHub for a newer release and installs it through the system
/// package manager, one task at a time.
pub struct Updater {
    current_version: ReleaseVersion,
    installation: Installation,
    updates_folder: PathBuf,
    server: Box<dyn ReleaseServer>,
    package_manager: Box<dyn PackageManager>,
    install_helper: Option<PathBuf>,
    /// The release the last check found, cleared when a check starts.
    /// Held for the whole of a check or installation, so only one runs.
    checked_release: Mutex<Option<Release>>,
    /// The version the last installation installed. A lock of its own,
    /// so reading it never makes a starting task look concurrent.
    installed_version: Mutex<Option<ReleaseVersion>>,
}

impl Updater {
    /// An updater made of `parts`.
    pub fn new(parts: UpdaterParts) -> Self {
        Self {
            current_version: parts.current_version,
            installation: parts.installation,
            updates_folder: parts.updates_folder,
            server: parts.server,
            package_manager: parts.package_manager,
            install_helper: parts.install_helper,
            checked_release: Mutex::new(None),
            installed_version: Mutex::new(None),
        }
    }

    /// The updater of the native app whose executable is at `executable`
    /// (see [`Installation::detect_for_executable`]): GitHub, the system's
    /// package tools, the package's update helper if it has one, and
    /// `~/.cache/winspace/updates`.
    pub fn for_executable(current_version: ReleaseVersion, executable: &Path) -> Self {
        let installation = Installation::detect_for_executable(executable, Path::new("/"));
        Self::new(UpdaterParts {
            current_version,
            installation,
            updates_folder: default_updates_folder(),
            server: Box::new(GitHubReleases::new(current_version)),
            package_manager: Box::new(SystemPackageManager),
            install_helper: installation.install_helper(Path::new("/")),
        })
    }

    /// How this build was installed.
    pub fn installation(&self) -> Installation {
        self.installation
    }

    /// The version an installation installed, if one did.
    pub fn installed_version(&self) -> Option<ReleaseVersion> {
        *lock_ignoring_poison(&self.installed_version)
    }

    /// Asks GitHub for the latest release. Only the fixed endpoint is
    /// asked, and a failed check forgets the previous one.
    ///
    /// # Errors
    ///
    /// [`UpdateError::TaskRunning`] while another task runs, a failed
    /// fetch in the check's wording, and every refusal of
    /// [`parse_release`].
    pub fn check(&self, cancel: &Cancellation) -> Result<UpdateStatus, UpdateError> {
        let mut checked_release = self.start_task()?;
        *checked_release = None;
        let answer = read_release_answer(self.server.as_ref(), cancel)?;
        let release = parse_release(&answer, self.current_version)?;
        let status = UpdateStatus {
            current_version: self.current_version,
            latest_version: release.version,
            is_available: release.is_newer,
            notes: release.notes.clone(),
            release_url: release.release_url.clone(),
            can_install: self.installation.can_install(),
            restart_required: self.installed_version().is_some(),
        };
        *checked_release = Some(release);
        Ok(status)
    }

    /// Installs `version`, which the last check must have found newer.
    /// `on_progress` hears each step as it starts.
    ///
    /// Safety rule "confirmed, checked, verified, then prompted"
    /// (`Updater.install` in `v2.0.0:desktop/updater.py`): nothing is fetched or
    /// run without the user's confirmation and a check for exactly this
    /// version; the administrator prompt appears only after the download's
    /// size, digest and package fields matched. Only the version comes
    /// from the caller; the address, digest and size are the check's.
    ///
    /// Where the package installed the update helper, the helper downloads,
    /// verifies and installs the release as root instead
    /// ([`install_update_command`](super::install_update_command)): the
    /// app hands it only the version, and an administrator at the machine
    /// is not asked for a password. If the helper cannot reach GitHub,
    /// which can happen behind a proxy only the user's session knows, the
    /// app downloads the release itself and installs it through `pkexec
    /// apt-get`, as before.
    ///
    /// Cancelling stops the task before the administrator prompt, or tells
    /// the helper to stop before APT starts. Once APT runs, cancelling has
    /// no effect: interrupting it can damage the system's package state.
    ///
    /// # Errors
    ///
    /// [`UpdateError::NotConfirmed`], [`UpdateError::TaskRunning`],
    /// [`UpdateError::NotChecked`], [`UpdateError::InstallUnavailable`],
    /// every download and verification error, whether the app or the
    /// helper found it, and the package manager's refusals. The download
    /// is deleted in every case.
    pub fn install(
        &self,
        version: ReleaseVersion,
        confirmation: Confirmation,
        on_progress: &dyn Fn(InstallProgress),
        cancel: &Cancellation,
    ) -> Result<(), UpdateError> {
        if confirmation != Confirmation::Confirmed {
            return Err(UpdateError::NotConfirmed);
        }
        let checked_release = self.start_task()?;
        let release = checked_release
            .as_ref()
            .filter(|release| release.is_newer && release.version == version)
            .ok_or(UpdateError::NotChecked)?;
        if !self.installation.can_install() {
            return Err(UpdateError::InstallUnavailable {
                installation: self.installation,
            });
        }
        on_progress(InstallProgress::Downloading(version));
        let through_helper = self
            .install_helper
            .as_deref()
            .map(|helper| self.install_through_helper(helper, version, on_progress, cancel));
        match through_helper {
            Some(Ok(())) => {}
            // No helper, or it could not reach GitHub: the app's own path.
            None | Some(Err(UpdateError::Unreachable)) => {
                let staged = download_installer(
                    self.server.as_ref(),
                    &release.installer,
                    &self.updates_folder,
                    cancel,
                )?;
                self.install_download(&staged.path(), version, on_progress, cancel)?;
            }
            Some(Err(error)) => return Err(error),
        }
        *lock_ignoring_poison(&self.installed_version) = Some(version);
        on_progress(InstallProgress::Installed);
        Ok(())
    }

    /// Runs the update helper for `version` through pkexec, shows its
    /// steps, and checks with `dpkg-query` that `version` is installed.
    ///
    /// Safety rule "success means the package manager confirms it": the
    /// helper's own report is not enough; the app asks `dpkg-query` too.
    fn install_through_helper(
        &self,
        helper: &Path,
        version: ReleaseVersion,
        on_progress: &dyn Fn(InstallProgress),
        cancel: &Cancellation,
    ) -> Result<(), UpdateError> {
        if cancel.is_cancelled() {
            return Err(UpdateError::Cancelled);
        }
        let packages = self.package_manager.as_ref();
        let command = PackageCommand::InstallRelease {
            helper: helper.to_path_buf(),
            version,
        };
        let mut outcome = None;
        let mut on_line = |line: &str| match HelperReport::decode(line) {
            Some(HelperReport::Installing) => on_progress(InstallProgress::AwaitingApproval),
            Some(report @ (HelperReport::Installed(_) | HelperReport::Failed(_))) => outcome = Some(report),
            // "Downloading" is already shown, and other lines are pkexec's.
            Some(HelperReport::Downloading) | None => {}
        };
        let output = packages.run_reporting(&command, &mut on_line, cancel)?;
        match outcome {
            Some(HelperReport::Failed(failure)) => return Err(failure.into_error()),
            Some(HelperReport::Installed(installed)) if installed == version && output.is_success() => {}
            // pkexec refused (126, 127) or the helper ended without saying
            // why: the same failure as a refused `pkexec apt-get`.
            _ => {
                return Err(installation_failure(&without_reports(output))
                    .unwrap_or(UpdateError::InstallNotConfirmed));
            }
        }
        let query = packages.run(&PackageCommand::QueryInstalled)?;
        check_installed_version(&query, version)
    }

    /// Checks the verified download with `dpkg-deb`, installs it behind
    /// the administrator prompt and checks with `dpkg-query` that
    /// `version` is installed.
    fn install_download(
        &self,
        installer: &Path,
        version: ReleaseVersion,
        on_progress: &dyn Fn(InstallProgress),
        cancel: &Cancellation,
    ) -> Result<(), UpdateError> {
        let packages = self.package_manager.as_ref();
        let inspection = packages.run(&PackageCommand::InspectInstaller(installer.to_path_buf()))?;
        check_installer_fields(&inspection, version)?;
        // The last moment cancelling can stop the installation.
        if cancel.is_cancelled() {
            return Err(UpdateError::Cancelled);
        }
        on_progress(InstallProgress::AwaitingApproval);
        let installation = packages.run(&PackageCommand::Install(installer.to_path_buf()))?;
        if let Some(failure) = installation_failure(&installation) {
            return Err(failure);
        }
        let query = packages.run(&PackageCommand::QueryInstalled)?;
        check_installed_version(&query, version)
    }

    /// Starts a task: the checked release, if no other task holds it.
    ///
    /// Safety rule "one update task at a time" (`self.lock` in
    /// `Updater`): a second check or installation is refused, not queued.
    fn start_task(&self) -> Result<MutexGuard<'_, Option<Release>>, UpdateError> {
        match self.checked_release.try_lock() {
            Ok(checked_release) => Ok(checked_release),
            // A task that panicked left a whole value behind.
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => Err(UpdateError::TaskRunning),
        }
    }
}

/// `output` without the helper's report lines, so a failure shows only
/// what pkexec or the helper printed for people.
fn without_reports(output: CommandOutput) -> CommandOutput {
    let stdout: Vec<&str> = output
        .stdout
        .lines()
        .filter(|line| HelperReport::decode(line).is_none())
        .collect();
    CommandOutput {
        stdout: stdout.join("\n"),
        ..output
    }
}

/// Locks `mutex`, using its value even if a panicking thread held it: each
/// value here is replaced whole, so it is never half-written.
fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl fmt::Debug for UpdaterParts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdaterParts")
            .field("current_version", &self.current_version)
            .field("installation", &self.installation)
            .field("updates_folder", &self.updates_folder)
            .field("install_helper", &self.install_helper)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Updater {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Updater")
            .field("current_version", &self.current_version)
            .field("installation", &self.installation)
            .field("updates_folder", &self.updates_folder)
            .field("install_helper", &self.install_helper)
            .finish_non_exhaustive()
    }
}

/// `~/.cache/winspace/updates`, under `XDG_CACHE_HOME` when it is set.
fn default_updates_folder() -> PathBuf {
    glib::user_cache_dir().join("winspace").join("updates")
}
