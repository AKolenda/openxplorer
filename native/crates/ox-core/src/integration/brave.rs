// SPDX-License-Identifier: AGPL-3.0-only
//! Pointing Brave's download folder at the user's Downloads folder, and
//! putting the previous folder back.
//!
//! Ports `desktop/brave_integration.py` (INT-019 to INT-021). The change
//! is explicit and offline, and every rule of the Python module holds:
//!
//! - Only detected native profiles are changed; Flatpak and Snap installs
//!   of Brave are reported for manual setup (`brave://settings/downloads`).
//! - Nothing changes while Brave runs, and the app never stops it.
//! - Every profile is checked before any is written; each write is backed
//!   up first, private (0600) and atomic, and changes only the two
//!   download folders.
//! - Restore puts back only a folder that still holds what the sync set.
//! - No browser policy, credential, extension or `sudo` is involved.
//!
//! Inside a Flatpak sandbox the host's Brave profiles and processes are
//! out of reach: the status says so ([`BraveReach::Sandboxed`]) and
//! syncing and restoring are refused.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `profiles` | Detecting profiles and sandboxed installs |
//! | `preferences` | Reading and writing `Preferences` and the undo record |
//! | `process` | Whether Brave is running |
//! | `changes` | Sync and restore |
//! | `error` | [`BraveError`] |

mod changes;
mod error;
mod preferences;
mod process;
mod profiles;

use std::future::Future;
use std::path::{Path, PathBuf};

pub use error::BraveError;
pub use preferences::DownloadPreference;
pub use process::{BraveActivity, ProcessTable};
pub use profiles::{BraveChannel, BraveProfile, SandboxedBrave};

use super::sandbox::Sandbox;
use super::worker::on_worker;

/// Where the user changes the download folder of a sandboxed Brave.
pub const MANUAL_SETTINGS_URL: &str = "brave://settings/downloads";

/// The backup folder's name in the settings folder.
const BACKUP_FOLDER_NAME: &str = "brave-backups";

/// Whether the user ticked the consent checkbox of the Brave dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// The user confirmed the change.
    Confirmed,
    /// The user did not confirm; nothing is changed.
    NotConfirmed,
}

/// The folders the Brave integration uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BravePaths {
    /// The settings folder (`~/.config/winspace`), which keeps the backups.
    pub settings: PathBuf,
    /// The user's home folder.
    pub home: PathBuf,
    /// The user's configuration folder, which holds `BraveSoftware`.
    pub config_home: PathBuf,
}

impl BravePaths {
    /// The current user's folders, with the backups in `settings`.
    pub fn for_user(settings: &Path) -> Self {
        Self {
            settings: settings.to_owned(),
            home: glib::home_dir(),
            config_home: glib::user_config_dir(),
        }
    }
}

/// Whether the app can see and change the host's Brave profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraveReach {
    /// The app runs on the host: its native profiles are listed and can
    /// be changed.
    Native,
    /// The app runs inside Flatpak, which has its own processes and
    /// configuration folder: the host's profiles, processes and installs
    /// are out of sight, so the dialog shows the message of
    /// [`BraveError::Sandboxed`] instead of profiles.
    Sandboxed,
}

impl BraveReach {
    /// What the app can reach from `sandbox`.
    pub fn for_sandbox(sandbox: Sandbox) -> Self {
        match sandbox {
            Sandbox::Host => Self::Native,
            Sandbox::Flatpak => Self::Sandboxed,
        }
    }
}

/// What the Brave dialog shows (INT-019).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraveStatus {
    /// Whether the host's profiles can be seen and changed at all.
    pub reach: BraveReach,
    /// The native profiles that can be updated.
    pub profiles: Vec<BraveProfile>,
    /// Brave is running, or that cannot be told.
    pub is_running: bool,
    /// Sandboxed installs that need manual setup.
    pub sandboxed_installs: Vec<SandboxedBrave>,
    /// The folder the backups and undo records are kept in.
    pub backups: PathBuf,
}

/// One profile a sync could not update.
#[derive(Debug)]
pub struct ProfileFailure {
    /// The profile's ID.
    pub profile: String,
    /// Why it was not updated.
    pub error: BraveError,
}

/// The result of a sync: each profile is reported on its own.
#[derive(Debug)]
pub struct SyncOutcome {
    /// The IDs of the profiles that now use the folder.
    pub updated: Vec<String>,
    /// The profiles that could not be updated.
    pub failures: Vec<ProfileFailure>,
    /// The download folder that was applied.
    pub path: PathBuf,
    /// The folder the backups are kept in.
    pub backups: PathBuf,
}

/// The Brave download-folder integration of one user.
#[derive(Debug, Clone)]
pub struct BraveIntegration<A = ProcessTable> {
    home: PathBuf,
    config_home: PathBuf,
    backups: PathBuf,
    activity: A,
    sandbox: Sandbox,
}

impl BraveIntegration<ProcessTable> {
    /// The integration in `paths`, which looks at `/proc` for Brave.
    pub fn new(paths: &BravePaths, sandbox: Sandbox) -> Self {
        Self::with_activity(paths, sandbox, ProcessTable::system())
    }
}

impl<A: BraveActivity> BraveIntegration<A> {
    /// The integration in `paths`, asking `activity` whether Brave runs.
    pub fn with_activity(paths: &BravePaths, sandbox: Sandbox, activity: A) -> Self {
        Self {
            home: paths.home.clone(),
            config_home: paths.config_home.clone(),
            backups: paths.settings.join(BACKUP_FOLDER_NAME),
            activity,
            sandbox,
        }
    }

    /// The folder the backups and undo records are kept in.
    pub fn backup_folder(&self) -> &Path {
        &self.backups
    }

    /// The detected native profiles; none inside Flatpak, whose
    /// configuration folder is the sandbox's, not the host's.
    pub fn profiles(&self) -> Vec<BraveProfile> {
        match BraveReach::for_sandbox(self.sandbox) {
            BraveReach::Native => profiles::detect_profiles(&self.config_home),
            BraveReach::Sandboxed => Vec::new(),
        }
    }

    /// Everything the Brave dialog shows. Only reads.
    pub fn status(&self) -> BraveStatus {
        match BraveReach::for_sandbox(self.sandbox) {
            BraveReach::Native => self.status_on_host(),
            BraveReach::Sandboxed => self.status_out_of_reach(),
        }
    }

    /// The status on the host: the detected profiles and installs, and
    /// whether Brave runs.
    fn status_on_host(&self) -> BraveStatus {
        BraveStatus {
            reach: BraveReach::Native,
            profiles: self.profiles(),
            is_running: self.activity.is_running(),
            sandboxed_installs: profiles::sandboxed_installs(&self.home),
            backups: self.backups.clone(),
        }
    }

    /// The status inside Flatpak.
    ///
    /// Safety rule "fail closed" (`browser_running` in
    /// `brave_integration.py`): the sandbox's process table holds none of
    /// the host's processes, so Brave counts as running because that
    /// cannot be told. No profile or install is listed, since the
    /// sandbox's configuration folder is not the host's and `~/.var/app`
    /// is hidden from it.
    fn status_out_of_reach(&self) -> BraveStatus {
        BraveStatus {
            reach: BraveReach::Sandboxed,
            profiles: Vec::new(),
            is_running: true,
            sandboxed_installs: Vec::new(),
            backups: self.backups.clone(),
        }
    }

    /// The preference file of the detected profile `profile_id`.
    ///
    /// # Errors
    ///
    /// [`BraveError::UnknownProfile`] if no detected profile has that ID.
    fn preferences_path(&self, profile_id: &str) -> Result<PathBuf, BraveError> {
        self.profiles()
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.preferences_path())
            .ok_or(BraveError::UnknownProfile)
    }

    /// The undo record of `profile_id`: named by a hash of the ID, so the
    /// ID never becomes part of a path.
    fn record_path(&self, profile_id: &str) -> PathBuf {
        self.backups.join(format!("{}.json", profile_hash(profile_id)))
    }

    /// Refuses Brave changes inside a Flatpak sandbox.
    ///
    /// Safety rule "never write host files from the sandbox": the
    /// sandbox's configuration folder is not the host's, and host
    /// processes are invisible there, so Brave could be running unseen.
    fn refuse_in_sandbox(&self) -> Result<(), BraveError> {
        if self.sandbox.is_flatpak() {
            return Err(BraveError::Sandboxed);
        }
        Ok(())
    }
}

impl<A: BraveActivity + Clone + Send + 'static> BraveIntegration<A> {
    /// Runs `operation` on a worker thread, for example
    /// `brave.run_in_background(|brave| brave.status())`. Each operation
    /// reads and writes a few files per profile and needs no cancellation.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl Future<Output = T> + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let brave = self.clone();
        on_worker(move || operation(&brave))
    }
}

/// The first 24 hexadecimal digits of the SHA-256 of `profile_id`, which
/// name its backups and undo record, as in the Python app.
fn profile_hash(profile_id: &str) -> String {
    let digest = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, profile_id)
        .expect("GLib always supports SHA-256");
    digest[..24].to_owned()
}
