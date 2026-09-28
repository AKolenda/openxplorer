// SPDX-License-Identifier: AGPL-3.0-only
//! The errors of updating and of the running-instance guard.
//!
//! Every message is the Python app's, word for word, where Python has one:
//! `desktop/updater.py`, `desktop/runtime_guard.py` and the update rules in
//! `desktop/winspace.py`. Where Python let a library exception through (an
//! HTTP error while downloading, invalid JSON, a failed `dpkg-deb`), the
//! message is in the app's wording instead; those variants say so.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::Installation;
use crate::private_storage::{StorageError, StorageRefusal};

/// Why an update check, installation or restart was refused or failed.
/// `Display` is the user-facing message.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The answer is not a release, or a draft or pre-release.
    #[error("No stable release is available.")]
    NoStableRelease,
    /// The release tag does not start with `v`.
    #[error("The release tag is invalid.")]
    InvalidTag,
    /// The version is not `MAJOR.MINOR.PATCH`.
    #[error("The release does not have a supported stable version.")]
    UnsupportedVersion,
    /// The release's `assets` is not a list.
    #[error("The release asset list is invalid.")]
    InvalidAssetList,
    /// No asset has the expected installer name and download URL.
    #[error("The release is missing its expected Debian installer.")]
    MissingInstaller,
    /// The installer has no SHA-256 digest yet.
    #[error("The release installer has no verified SHA-256 digest yet. Try again later.")]
    MissingDigest,
    /// The installer size is missing, not an integer or out of range.
    #[error("The release installer size is invalid.")]
    InvalidInstallerSize,
    /// An address or redirect outside the trusted hosts.
    #[error("The update server returned an untrusted download location.")]
    UntrustedLocation,
    /// The release answer is over 2 MiB.
    #[error("The update response is too large.")]
    ResponseTooLarge,
    /// The release answer is not JSON. Python showed the JSON parser's own
    /// message.
    #[error("The update response is not valid JSON.")]
    InvalidResponse,
    /// GitHub answered the check with an HTTP error.
    #[error("GitHub could not check for updates (HTTP {status}). Try again later.")]
    CheckRefused {
        /// The HTTP status code.
        status: u32,
    },
    /// GitHub answered the download with an HTTP error. Python showed
    /// urllib's own message.
    #[error("GitHub could not provide the installer (HTTP {status}). Nothing was installed.")]
    DownloadRefused {
        /// The HTTP status code.
        status: u32,
    },
    /// No connection, or it timed out.
    #[error("Could not reach GitHub. Check your connection and try again.")]
    Unreachable,
    /// Another check or installation holds the updater.
    #[error("An update task is already running.")]
    TaskRunning,
    /// The user cancelled before the package manager started. Python had
    /// no cancellation.
    #[error("The update was cancelled. Nothing was installed.")]
    Cancelled,
    /// Installation was asked for without the user's confirmation.
    #[error("Confirm installation before updating.")]
    NotConfirmed,
    /// No check found this newer version.
    #[error("Check for updates again before installing.")]
    NotChecked,
    /// This build cannot install updates itself.
    #[error("{}", installation.install_refusal())]
    InstallUnavailable {
        /// How this build was installed.
        installation: Installation,
    },
    /// The download is larger than the release said.
    #[error("The downloaded installer is larger than expected.")]
    InstallerTooLarge,
    /// The download's size or SHA-256 digest is not the release's.
    #[error("The installer checksum or size did not match. Nothing was installed.")]
    ChecksumMismatch,
    /// `dpkg-deb` reports another package, version or architecture.
    #[error("The installer metadata did not match this release. Nothing was installed.")]
    MetadataMismatch,
    /// The administrator prompt was refused, or APT failed.
    #[error("Installation was cancelled or failed. {details}")]
    InstallFailed {
        /// The end of APT's error output.
        details: String,
    },
    /// `dpkg-query` does not report the new version as installed.
    #[error("The package manager did not confirm the expected installed version.")]
    InstallNotConfirmed,
    /// `dpkg-deb` or `dpkg-query` failed. Python showed `subprocess`'s own
    /// message.
    ///
    /// The message does not say whether anything was installed:
    /// `dpkg-query` runs after APT, which may have changed the system.
    #[error("The package tool {program} failed (exit status {status}).")]
    PackageToolFailed {
        /// The tool's path.
        program: &'static str,
        /// Its exit status, or minus the signal number if a signal ended
        /// it.
        status: i32,
    },
    /// `dpkg-deb` or `dpkg-query` did not finish in time and was stopped.
    /// Python showed `subprocess`'s own message.
    #[error("The package tool {program} did not finish within {} seconds.", limit.as_secs())]
    PackageToolTimedOut {
        /// The tool's path.
        program: &'static str,
        /// Its time limit.
        limit: Duration,
    },
    /// The private download folder broke a private-storage rule.
    #[error("{reason} ({})", path.display())]
    Refused {
        /// The refused folder.
        path: PathBuf,
        /// Which rule it broke.
        reason: StorageRefusal,
    },
    /// A file-system or process error on `path`.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file, folder or program.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
    /// An update is installing: everything but window chrome waits.
    #[error("An application update is running. Wait for it to finish before using files.")]
    UpdateRunning,
    /// An installation changed the application's files: restart first.
    #[error("Restart OpenXplorer to finish the application update before using files.")]
    RestartRequired,
    /// File operations, folder loading, mount prompts or tab moves are
    /// running somewhere.
    #[error("Wait for file operations, folder loading and tab moves to finish, then try again.")]
    WorkInProgress,
    /// Restart was asked for without an installed update.
    #[error("No installed update is waiting for restart.")]
    NoRestartPending,
    /// A file operation is still writing somewhere.
    #[error("Wait for file operations to finish before restarting.")]
    WritesRunning,
    /// A new window was asked for during an update or before its restart.
    #[error("Finish the application update and restart before opening another window.")]
    WindowsBlocked,
    /// A window was closed while an update installs.
    #[error("Wait for the application update to finish before closing.")]
    CloseRefused,
    /// The app was quit while an update installs.
    #[error("Wait for the application update to finish before quitting.")]
    QuitRefused,
}

impl UpdateError {
    /// A file-system or process error on `path`.
    pub(crate) fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            error,
        }
    }
}

/// A private-storage error keeps its path and reason.
impl From<StorageError> for UpdateError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Refused { path, reason } => Self::Refused { path, reason },
            StorageError::Io { path, error } => Self::Io { path, error },
        }
    }
}

/// Why the running-instance guard stopped a launch. `Display` is the
/// message `openxplorer` prints before exiting with status 3.
#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    /// A different or older instance runs and the user did not agree to
    /// restart it.
    #[error(
        "A different or older OpenXplorer process is running. Finish file operations and run \
         openxplorer --restart."
    )]
    OutdatedInstance,
    /// The instance asked to quit still runs, for example because a file
    /// operation is writing.
    #[error(
        "OpenXplorer is still running. Finish or cancel active file operations, then run \
         openxplorer --restart again. No process was killed."
    )]
    StillRunning,
    /// Another instance took the application name while waiting.
    #[error(
        "Another OpenXplorer process started during restart. No process was killed. Retry after closing it."
    )]
    AnotherInstance,
    /// The session bus refused a call.
    #[error("{0}")]
    Bus(#[from] glib::Error),
}
