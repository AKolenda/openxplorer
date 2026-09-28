// SPDX-License-Identifier: AGPL-3.0-only
//! Syncing the download folder into Brave profiles, and restoring it.
//!
//! Ports `BraveIntegration.sync` and `BraveIntegration.restore` in
//! `desktop/brave_integration.py` (INT-020, INT-021).

use std::collections::HashSet;
use std::fs::{self, DirBuilder, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustix::fs::Access;

use super::preferences::{
    compact_json, has_unsupported_structure, previous_folder, read_preferences, read_undo_record,
    restore_download_folders, set_download_folders, write_private, write_undo_record, DownloadPreference,
    Preferences, UndoRecord,
};
use super::{
    profile_hash, BraveActivity, BraveError, BraveIntegration, Confirmation, ProfileFailure, SyncOutcome,
};

/// The most profiles one sync may update, as in the Python app.
const MAX_PROFILES: usize = 40;

/// Folders whose contents do not survive a reboot or are not files.
const VOLATILE_PREFIXES: [&str; 4] = ["/run/", "/proc/", "/sys/", "/dev/"];

/// The mode of the backup folder.
const BACKUP_FOLDER_MODE: u32 = 0o700;

/// A profile that was checked and read before anything is written.
struct PlannedSync {
    profile_id: String,
    path: PathBuf,
    preferences: Preferences,
}

impl<A: BraveActivity> BraveIntegration<A> {
    /// Sets the download and "Save as" folders of `profile_ids` to
    /// `destination` (INT-020). Failures after the checks are reported per
    /// profile in the outcome.
    ///
    /// Safety rule "check every profile before writing any" (`sync` in
    /// `brave_integration.py`): the consent, the selection, the folder, that
    /// Brave is closed and every profile's preferences are checked before
    /// the first backup is written.
    ///
    /// # Errors
    ///
    /// A [`BraveError`] for a refused request, a running Brave, or a
    /// profile that is unknown, unreadable or malformed; nothing has been
    /// written then.
    pub fn sync(
        &self,
        profile_ids: &[String],
        destination: &str,
        confirmation: Confirmation,
    ) -> Result<SyncOutcome, BraveError> {
        self.refuse_in_sandbox()?;
        if confirmation != Confirmation::Confirmed {
            return Err(BraveError::SyncNotConfirmed);
        }
        check_selection(profile_ids)?;
        let folder = self.checked_destination(destination)?;
        if self.activity.is_running() {
            return Err(BraveError::BraveRunningBeforeSync);
        }
        let plans = profile_ids
            .iter()
            .map(|profile_id| self.plan_sync(profile_id))
            .collect::<Result<Vec<_>, _>>()?;
        self.prepare_backup_folder()?;
        let mut outcome = SyncOutcome {
            updated: Vec::new(),
            failures: Vec::new(),
            path: PathBuf::from(&folder),
            backups: self.backups.clone(),
        };
        for plan in plans {
            let profile = plan.profile_id.clone();
            match self.apply_sync(plan, &folder) {
                Ok(()) => outcome.updated.push(profile),
                Err(error) => outcome.failures.push(ProfileFailure { profile, error }),
            }
        }
        Ok(outcome)
    }

    /// Puts back the folders the last sync of `profile_id` replaced, where
    /// they still hold what it set (INT-021), and returns the groups put
    /// back. The undo record is then removed.
    ///
    /// # Errors
    ///
    /// A [`BraveError`] for a refused request, a running Brave, a missing
    /// record, folders changed since the sync, or a failed read or write.
    pub fn restore(
        &self,
        profile_id: &str,
        confirmation: Confirmation,
    ) -> Result<Vec<DownloadPreference>, BraveError> {
        self.refuse_in_sandbox()?;
        if confirmation != Confirmation::Confirmed {
            return Err(BraveError::RestoreNotConfirmed);
        }
        if self.activity.is_running() {
            return Err(BraveError::BraveRunningBeforeRestore);
        }
        let record_path = self.record_path(profile_id);
        if !record_path.exists() {
            return Err(BraveError::NoRecord);
        }
        let record = read_undo_record(&record_path)?;
        let path = self.preferences_path(profile_id)?;
        let Preferences { raw, mut data } = read_preferences(&path)?;
        let restored = restore_download_folders(&mut data, &record);
        if restored.is_empty() {
            return Err(BraveError::ChangedSinceSync);
        }
        if !self.is_unchanged(&path, &raw)? {
            return Err(BraveError::ChangedDuringRestore);
        }
        // Written without a trailing newline, as the Python app's restore
        // writes it.
        write_private(&path, &compact_json(&data))?;
        fs::remove_file(&record_path).map_err(|error| BraveError::io(&record_path, error))?;
        Ok(restored)
    }

    /// Checks one profile and reads its preferences.
    fn plan_sync(&self, profile_id: &str) -> Result<PlannedSync, BraveError> {
        let path = self.preferences_path(profile_id)?;
        let preferences = read_preferences(&path)?;
        if has_unsupported_structure(&preferences.data) {
            return Err(BraveError::UnsupportedStructure);
        }
        Ok(PlannedSync {
            profile_id: profile_id.to_owned(),
            path,
            preferences,
        })
    }

    /// Backs up one profile, records how to undo the change, and writes
    /// the new folders.
    fn apply_sync(&self, plan: PlannedSync, folder: &str) -> Result<(), BraveError> {
        let PlannedSync {
            profile_id,
            path,
            preferences: Preferences { raw, mut data },
        } = plan;
        let previous = DownloadPreference::ALL
            .iter()
            .map(|group| (group.as_str().to_owned(), previous_folder(&data, *group)))
            .collect();
        let backup = self.backup_path(&profile_id);
        write_private(&backup, &raw)?;
        set_download_folders(&mut data, folder);
        let record = UndoRecord {
            profile: profile_id.clone(),
            previous,
            applied: folder.to_owned(),
            backup: backup.to_string_lossy().into_owned(),
        };
        // Safety rule "stop if Brave started or the file changed": checked
        // again right before writing, since Brave may start at any time.
        if !self.is_unchanged(&path, &raw)? {
            return Err(BraveError::ChangedDuringSync);
        }
        // The undo record is written first: if replacing the preferences
        // then fails, restore finds the applied folder absent and changes
        // nothing.
        write_undo_record(&self.record_path(&profile_id), &record)?;
        let mut json = compact_json(&data);
        json.push(b'\n');
        write_private(&path, &json)
    }

    /// True if Brave is closed and `path` is still the file read as `raw`.
    fn is_unchanged(&self, path: &Path, raw: &[u8]) -> Result<bool, BraveError> {
        if self.activity.is_running() || path.is_symlink() {
            return Ok(false);
        }
        let current = fs::read(path).map_err(|error| BraveError::io(path, error))?;
        Ok(current == raw)
    }

    /// The canonical download folder, if it can be Brave's.
    ///
    /// Safety rule "a persistent, dedicated download folder" (`sync` in
    /// `brave_integration.py`): the folder must exist and be writable, and
    /// must not be under `/run`, `/proc`, `/sys` or `/dev`, the home folder
    /// or `/`. An SMB URI is not an absolute path and is refused.
    fn checked_destination(&self, destination: &str) -> Result<String, BraveError> {
        if destination.contains('\0') {
            return Err(BraveError::InvalidDirectory);
        }
        if !Path::new(destination).is_absolute() {
            return Err(BraveError::RelativeDirectory);
        }
        let folder = fs::canonicalize(destination).map_err(|_| BraveError::UnusableDirectory)?;
        let is_writable = rustix::fs::access(&folder, Access::WRITE_OK | Access::EXEC_OK).is_ok();
        if !folder.is_dir() || !is_writable {
            return Err(BraveError::UnusableDirectory);
        }
        // Brave stores the folder as JSON text, which must be UTF-8.
        let text = folder.to_str().ok_or(BraveError::InvalidDirectory)?;
        let is_volatile = VOLATILE_PREFIXES.iter().any(|prefix| text.starts_with(prefix));
        if is_volatile || folder == self.home || folder == Path::new("/") {
            return Err(BraveError::NotDedicated);
        }
        Ok(text.to_owned())
    }

    /// Creates the backup folder with mode 0700, refusing a symlink.
    fn prepare_backup_folder(&self) -> Result<(), BraveError> {
        if self.backups.is_symlink() {
            return Err(BraveError::SymlinkedBackupFolder);
        }
        let io_error = |error| BraveError::io(&self.backups, error);
        create_private_folder(&self.backups).map_err(io_error)?;
        fs::set_permissions(&self.backups, Permissions::from_mode(BACKUP_FOLDER_MODE)).map_err(io_error)
    }

    /// A new backup file for `profile_id`, named by the ID's hash and the
    /// time in nanoseconds.
    fn backup_path(&self, profile_id: &str) -> PathBuf {
        let nanoseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!("{}-{nanoseconds}.preferences.bak", profile_hash(profile_id));
        self.backups.join(name)
    }
}

/// Checks that 1 to 40 distinct profiles are selected.
fn check_selection(profile_ids: &[String]) -> Result<(), BraveError> {
    let distinct: HashSet<&String> = profile_ids.iter().collect();
    let is_valid = (1..=MAX_PROFILES).contains(&profile_ids.len()) && distinct.len() == profile_ids.len();
    if is_valid {
        Ok(())
    } else {
        Err(BraveError::InvalidSelection)
    }
}

/// Creates `folder` with mode 0700 and any missing parents with the
/// default mode, like Python's `mkdir(parents=True, exist_ok=True,
/// mode=0o700)`.
fn create_private_folder(folder: &Path) -> io::Result<()> {
    if let Some(parent) = folder.parent() {
        fs::create_dir_all(parent)?;
    }
    match DirBuilder::new().mode(BACKUP_FOLDER_MODE).create(folder) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && folder.is_dir() => Ok(()),
        created => created,
    }
}
