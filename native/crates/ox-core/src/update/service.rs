// SPDX-License-Identifier: AGPL-3.0-only
//! The application's update service: the rules every window follows while
//! an update installs and until the restart, and checks and installations
//! run on worker threads. Ports the update branches of `dispatch`, and
//! `on_delete`, `create_window` and `quit_safely`, in `desktop/winspace.py`.

use std::cell::Cell;
use std::fmt;
use std::io;
use std::panic;
use std::sync::Arc;

use futures_channel::mpsc;
use futures_util::StreamExt;

use super::{
    Confirmation, InstallProgress, InstalledBuild, ReleaseVersion, RestartLauncher, RuntimeIdentity,
    UpdateError, UpdateStatus, Updater, RESTART_COMMAND,
};
use crate::transfer::Cancellation;

/// What the application is doing about an update, in every window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UpdatePhase {
    /// No update is installing or waiting.
    #[default]
    Idle,
    /// An installation runs.
    Installing,
    /// An installation changed the application's files; the running
    /// process must restart before it touches files again.
    RestartRequired,
}

/// What a window asks the application to do, for [`UpdateService::check_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRequest {
    /// Anything that reads or changes files: listing, search, file
    /// operations and the clipboard.
    Files,
    /// Check for updates.
    UpdateCheck,
    /// Install an update.
    UpdateInstall,
    /// Restart into an installed update.
    UpdateRestart,
    /// Read the application's environment for a window.
    Environment,
    /// Quit the application.
    Quit,
    /// The window's own chrome: title, metadata and readiness.
    WindowChrome,
}

/// Whether work is running that an update must not interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Nothing is running.
    Idle,
    /// Something is running.
    Busy,
}

/// The answer to [`UpdateService::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// GitHub was asked.
    Checked(UpdateStatus),
    /// An installation is waiting for a restart; GitHub was not asked.
    RestartPending {
        /// The version installed, unless the installation failed after
        /// changing files.
        installed_version: Option<ReleaseVersion>,
    },
}

/// What [`UpdateService::install`] installs, and how.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// The version the last check found.
    pub version: ReleaseVersion,
    /// Whether the user confirmed.
    pub confirmation: Confirmation,
    /// Whether any open window has running jobs (folder loading
    /// included), writes, a mount prompt or a pending tab handoff, or a
    /// tab move is pending.
    pub activity: Activity,
    /// Stops the installation before the administrator prompt.
    pub cancel: Cancellation,
}

/// The parts an [`UpdateService`] is made of.
pub struct UpdateServiceParts {
    /// The updater.
    pub updater: Updater,
    /// This process's identity, read when it started.
    pub running: RuntimeIdentity,
    /// Where the installed build is, to see whether an installation
    /// changed it.
    pub installed_build: InstalledBuild,
    /// What starts the restart launcher.
    pub launcher: Box<dyn RestartLauncher>,
}

/// The update service. It lives on the main thread; checks and
/// installations run on GIO worker threads.
pub struct UpdateService {
    updater: Arc<Updater>,
    running: RuntimeIdentity,
    installed_build: InstalledBuild,
    launcher: Box<dyn RestartLauncher>,
    phase: Cell<UpdatePhase>,
}

impl UpdateService {
    /// A service made of `parts`, with no update in progress.
    pub fn new(parts: UpdateServiceParts) -> Self {
        Self {
            updater: Arc::new(parts.updater),
            running: parts.running,
            installed_build: parts.installed_build,
            launcher: parts.launcher,
            phase: Cell::new(UpdatePhase::Idle),
        }
    }

    /// What the application is doing about an update.
    pub fn phase(&self) -> UpdatePhase {
        self.phase.get()
    }

    /// The updater.
    pub fn updater(&self) -> &Updater {
        &self.updater
    }

    /// Whether a window may do `request` now.
    ///
    /// Safety rule "an update locks the application" (`dispatch` in
    /// `desktop/winspace.py`): while an update installs, only a window's
    /// own chrome is served, in every window; until the restart, files
    /// stay untouched and only update status, restart, quitting, chrome
    /// and the environment are served.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UpdateRunning`] or [`UpdateError::RestartRequired`].
    pub fn check_request(&self, request: AppRequest) -> Result<(), UpdateError> {
        match self.phase.get() {
            UpdatePhase::Idle => Ok(()),
            UpdatePhase::Installing if request == AppRequest::WindowChrome => Ok(()),
            UpdatePhase::Installing => Err(UpdateError::UpdateRunning),
            UpdatePhase::RestartRequired => match request {
                AppRequest::Files | AppRequest::UpdateInstall => Err(UpdateError::RestartRequired),
                _ => Ok(()),
            },
        }
    }

    /// Whether another window may open (`create_window`).
    ///
    /// # Errors
    ///
    /// [`UpdateError::WindowsBlocked`] during an update and until the
    /// restart.
    pub fn check_new_window(&self) -> Result<(), UpdateError> {
        match self.phase.get() {
            UpdatePhase::Idle => Ok(()),
            UpdatePhase::Installing | UpdatePhase::RestartRequired => Err(UpdateError::WindowsBlocked),
        }
    }

    /// Whether a window may close (`on_delete`). A pending restart does
    /// not keep windows open.
    ///
    /// # Errors
    ///
    /// [`UpdateError::CloseRefused`] while an update installs.
    pub fn check_close_window(&self) -> Result<(), UpdateError> {
        self.refuse_while_installing(UpdateError::CloseRefused)
    }

    /// Whether the application may quit (`quit_safely`). A pending
    /// restart allows it.
    ///
    /// # Errors
    ///
    /// [`UpdateError::QuitRefused`] while an update installs.
    pub fn check_quit(&self) -> Result<(), UpdateError> {
        self.refuse_while_installing(UpdateError::QuitRefused)
    }

    /// Checks for updates on a worker thread. While a restart is pending
    /// GitHub is not asked.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UpdateRunning`] during an installation, and every
    /// error of [`Updater::check`].
    ///
    /// # Panics
    ///
    /// Resumes a panic of the worker: a panicking check is a bug.
    pub async fn check(&self, cancel: Cancellation) -> Result<UpdateCheck, UpdateError> {
        self.check_request(AppRequest::UpdateCheck)?;
        if self.phase.get() == UpdatePhase::RestartRequired {
            let installed_version = self.updater.installed_version();
            return Ok(UpdateCheck::RestartPending { installed_version });
        }
        let updater = Arc::clone(&self.updater);
        let checked = gio::spawn_blocking(move || updater.check(&cancel)).await;
        let status = checked.unwrap_or_else(|panic| panic::resume_unwind(panic))?;
        Ok(UpdateCheck::Checked(status))
    }

    /// Installs an update on a worker thread, locking the application
    /// until it ends. `on_progress` hears each step on this thread, before
    /// the installation resolves.
    ///
    /// Safety rule "nothing else runs while the package manager does"
    /// (`updateInstall` in `dispatch`): the user must have confirmed and
    /// no window may have work running before the application locks.
    ///
    /// Safety rule "a changed installation needs a restart": even a
    /// failed installation may have replaced files, so afterwards the
    /// installed build is compared with this process's, and the
    /// application waits for a restart if they differ or cannot be
    /// compared.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UpdateRunning`], [`UpdateError::RestartRequired`],
    /// [`UpdateError::NotConfirmed`], [`UpdateError::WorkInProgress`], and
    /// every error of [`Updater::install`].
    ///
    /// Await it to the end: if the future is dropped before it resolves,
    /// the application waits for a restart, because the package manager
    /// may still be running. Cancel with [`InstallRequest::cancel`]
    /// instead.
    ///
    /// # Panics
    ///
    /// Resumes a panic of the worker, after the application waits for a
    /// restart: a panicking installation is a bug, and it may have changed
    /// files.
    pub async fn install(
        &self,
        request: InstallRequest,
        on_progress: impl Fn(InstallProgress),
    ) -> Result<(), UpdateError> {
        self.check_request(AppRequest::UpdateInstall)?;
        if request.confirmation != Confirmation::Confirmed {
            return Err(UpdateError::NotConfirmed);
        }
        if request.activity == Activity::Busy {
            return Err(UpdateError::WorkInProgress);
        }
        let lock = InstallationLock::lock(&self.phase);
        let (progress_sender, progress) = mpsc::unbounded();
        let installation = self.spawn_installation(request, progress_sender);
        let delivery = progress.for_each(|step| {
            on_progress(step);
            std::future::ready(())
        });
        let (outcome, ()) = futures_util::future::join(installation, delivery).await;
        let (result, installed) = outcome.unwrap_or_else(|panic| panic::resume_unwind(panic));
        lock.release(phase_after_installation(installed, &self.running));
        result
    }

    /// Restarts into the installed update.
    ///
    /// Safety rule "only the fixed launcher" (`updateRestart` in
    /// `dispatch`): the launcher is [`RESTART_COMMAND`], never a path or
    /// command from a window, and it asks this exact instance to quit
    /// safely.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UpdateRunning`], [`UpdateError::NoRestartPending`],
    /// [`UpdateError::WritesRunning`] if any open window writes, and a
    /// launcher that cannot start.
    pub fn restart(&self, writes: Activity) -> Result<(), UpdateError> {
        self.check_request(AppRequest::UpdateRestart)?;
        if self.phase.get() != UpdatePhase::RestartRequired {
            return Err(UpdateError::NoRestartPending);
        }
        if writes == Activity::Busy {
            return Err(UpdateError::WritesRunning);
        }
        self.launcher.launch(&RESTART_COMMAND)
    }

    /// Runs the installation and then reads the installed build's
    /// identity, both on one worker thread.
    fn spawn_installation(
        &self,
        request: InstallRequest,
        progress: mpsc::UnboundedSender<InstallProgress>,
    ) -> gio::JoinHandle<(Result<(), UpdateError>, io::Result<RuntimeIdentity>)> {
        let updater = Arc::clone(&self.updater);
        let installed_build = self.installed_build.clone();
        gio::spawn_blocking(move || {
            let report = |step| {
                // Nobody listens once the caller stopped waiting.
                let _ = progress.unbounded_send(step);
            };
            let result = updater.install(request.version, request.confirmation, &report, &request.cancel);
            (result, installed_build.read_identity())
        })
    }

    fn refuse_while_installing(&self, refusal: UpdateError) -> Result<(), UpdateError> {
        if self.phase.get() == UpdatePhase::Installing {
            Err(refusal)
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for UpdateServiceParts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateServiceParts")
            .field("updater", &self.updater)
            .field("running", &self.running)
            .field("installed_build", &self.installed_build)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UpdateService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateService")
            .field("updater", &self.updater)
            .field("running", &self.running)
            .field("installed_build", &self.installed_build)
            .field("phase", &self.phase.get())
            .finish_non_exhaustive()
    }
}

/// The application's lock while an installation runs.
///
/// Safety rule "an unfinished installation needs a restart": if the lock
/// is dropped before it is released (the installation's future was dropped
/// or its worker panicked), the package manager may still be running or
/// may have changed files, so the application waits for a restart.
struct InstallationLock<'a> {
    phase: &'a Cell<UpdatePhase>,
}

impl<'a> InstallationLock<'a> {
    /// Locks the application for an installation.
    fn lock(phase: &'a Cell<UpdatePhase>) -> Self {
        phase.set(UpdatePhase::Installing);
        Self { phase }
    }

    /// Unlocks it into `phase` once the installation has finished.
    fn release(self, phase: UpdatePhase) {
        self.phase.set(phase);
    }
}

impl Drop for InstallationLock<'_> {
    fn drop(&mut self) {
        if self.phase.get() == UpdatePhase::Installing {
            self.phase.set(UpdatePhase::RestartRequired);
        }
    }
}

/// The phase after an installation: a restart is required when the
/// installed build is not this process's, or could not be read.
fn phase_after_installation(
    installed: io::Result<RuntimeIdentity>,
    running: &RuntimeIdentity,
) -> UpdatePhase {
    match installed {
        Ok(identity) if identity == *running => UpdatePhase::Idle,
        _ => UpdatePhase::RestartRequired,
    }
}
