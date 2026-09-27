// SPDX-License-Identifier: AGPL-3.0-only
//! Shared settings in `$XDG_CONFIG_HOME/winspace/settings.json`.
//!
//! Ports `Settings` in `desktop/core.py` and `desktop/private_storage.py`.
//! The Python application and this one use the same file, so the protocol
//! matches exactly:
//!
//! * Reading validates against a whitelist and never fails: unreadable
//!   input yields safe defaults plus a [`warning`](Settings::warning).
//! * Every change takes an exclusive `flock` on `settings.lock`, re-reads
//!   the file, applies the change and atomically replaces the file with a
//!   private (0600) copy in a private (0700) directory. Symlinked or
//!   hard-linked settings files are refused, never followed.
//! * Everything the Python app keeps (including `contextMenu`,
//!   `networkInterval`, `autoIndex` and `columnWidths`) is read and written
//!   back, so a change made here never erases a Python setting.
//!
//! One rule goes beyond the Python app: a change never erases a settings
//! file whose contents could not be read. It is first renamed to
//! `settings.json.unreadable-…` beside the new file, and the
//! [`warning`](Settings::warning) says where it went.
//!
//! The submodules split the work: `read` reads the file, `mutate` holds
//! the changes, `save` locks and writes, and `storage` enforces the
//! private-storage rules all of them rely on.
//!
//! The `winspace` directory name is a compatibility contract; do not rename
//! it.

mod choices;
mod error;
mod labels;
mod model;
mod mutate;
mod preferences;
mod python_conversions;
mod read;
mod save;
mod storage;
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};

pub use choices::{Appearance, ContextMenu, Theme, View};
pub use error::{SettingsError, StorageRefusal};
pub use model::{Bookmark, RecentEntry, SettingsData, SETTINGS_VERSION};
pub use mutate::{BookmarkAction, BookmarkKind, PinRequest};
pub use preferences::{
    Column, ColumnWidths, Preferences, PreferencesUpdate, DEFAULT_TEXT_SIZE, NETWORK_INTERVALS,
    SIDEBAR_WIDTHS, TEXT_SIZES,
};

use save::{replace_private_file, OldFile, SettingsLock};

/// Settings shared by every window of both applications.
///
/// [`data`](Self::data) is the state after the last read or change;
/// [`snapshot`](Self::snapshot) re-reads the file first. Each change method
/// locks, re-reads, validates, applies and saves; on any error the data
/// stays as last read.
#[derive(Debug, Clone)]
pub struct Settings {
    directory: PathBuf,
    data: SettingsData,
    file_state: FileState,
}

/// What the last read or change found out about `settings.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileState {
    /// Read completely, or not created yet.
    Sound,
    /// A private-storage check refused the file or its directory (a
    /// symlink, hard link, other owner or I/O error). Changes are refused
    /// too, and the file is never moved or replaced.
    Refused(String),
    /// The file is private, but its contents are too large, not UTF-8, not
    /// JSON or of the wrong shape. The next change keeps it as a backup.
    Damaged(String),
    /// A change replaced a damaged file after keeping it as a backup; the
    /// message names the backup.
    BackedUp(String),
}

impl Settings {
    /// Name of the settings file inside the settings directory.
    pub const FILE_NAME: &'static str = "settings.json";

    /// Prefix of the temporary files used for atomic saves.
    const TEMPORARY_PREFIX: &'static str = ".settings-";

    /// The default directory: `$XDG_CONFIG_HOME/winspace` or
    /// `~/.config/winspace`.
    pub fn default_directory() -> PathBuf {
        glib::user_config_dir().join("winspace")
    }

    /// Loads settings from `directory`; never fails. A missing file gives
    /// the defaults; anything unreadable gives the defaults plus a warning.
    /// Does not create the directory, but an existing directory and
    /// `settings.json` are made private (0700 / 0600) while they are read,
    /// as `Settings.__init__` in core.py does.
    pub fn open(directory: &Path) -> Self {
        let mut data = SettingsData::default();
        let file_state = read::read_file(directory, &mut data);
        Self {
            directory: directory.to_path_buf(),
            data,
            file_state,
        }
    }

    /// Loads settings from [`default_directory`](Self::default_directory).
    pub fn open_default() -> Self {
        Self::open(&Self::default_directory())
    }

    /// Re-reads the file if it exists; the data and warning are replaced
    /// together. A deleted file keeps the data last read.
    pub fn reload(&mut self) {
        if self.path().exists() {
            *self = Self::open(&self.directory);
        }
    }

    /// The data as last read or changed.
    pub fn data(&self) -> &SettingsData {
        &self.data
    }

    /// Re-reads the file and returns a copy of the current data.
    pub fn snapshot(&mut self) -> SettingsData {
        self.reload();
        self.data.clone()
    }

    /// What the user should be told about the settings file: why the last
    /// read fell back to defaults, or, right after a change replaced an
    /// unreadable file, where that file was kept.
    pub fn warning(&self) -> Option<&str> {
        match &self.file_state {
            FileState::Sound => None,
            FileState::Refused(message) | FileState::Damaged(message) | FileState::BackedUp(message) => {
                Some(message)
            }
        }
    }

    /// The settings directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The path of `settings.json`.
    pub fn path(&self) -> PathBuf {
        self.directory.join(Self::FILE_NAME)
    }

    /// Applies every valid value in `update`, ignores the rest, saves, and
    /// returns the resulting preferences.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Refused`] if private storage refuses the settings
    /// directory, lock or file, and [`SettingsError::Io`] if one of them
    /// cannot be opened or written.
    pub fn update_preferences(&mut self, update: &PreferencesUpdate) -> Result<Preferences, SettingsError> {
        self.mutate(|data| {
            data.preferences.apply(update);
            Ok(data.preferences.clone())
        })
    }

    /// Adds or removes a Quick access pin or a mapped share. Removing a pin
    /// hides it from Quick access, which also works for known folders;
    /// adding it shows it again.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Invalid`] for a location or label the Python app
    /// would reject (for example one with credentials), and every error of
    /// [`update_preferences`](Self::update_preferences).
    pub fn bookmark(
        &mut self,
        action: BookmarkAction,
        kind: BookmarkKind,
        uri: &str,
        label: &str,
    ) -> Result<(), SettingsError> {
        self.mutate(|data| mutate::apply_bookmark(data, action, kind, uri, label))
    }

    /// Adds or reorders up to 200 Quick access pins in one change and
    /// returns the cleaned pins. The batch is validated as a whole, so an
    /// invalid entry changes nothing. See [`PinRequest`].
    ///
    /// `before` is the entry the folders were dropped on; `quick_order` is
    /// the order the sidebar showed (at most 400 entries).
    ///
    /// # Errors
    ///
    /// [`SettingsError::Invalid`] for an empty or oversized batch, an
    /// invalid location, label or order, or more than 200 pins in total;
    /// and every error of [`update_preferences`](Self::update_preferences).
    pub fn pin_many(
        &mut self,
        items: &[PinRequest],
        before: Option<&str>,
        quick_order: Option<&[String]>,
    ) -> Result<Vec<Bookmark>, SettingsError> {
        self.mutate(|data| mutate::pin_many(data, items, before, quick_order))
    }

    /// Records an opened file at the top of the recent files.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Invalid`] for an invalid file location, and every
    /// error of [`update_preferences`](Self::update_preferences).
    pub fn remember_open(&mut self, entry: RecentEntry) -> Result<(), SettingsError> {
        self.mutate(move |data| mutate::remember_open(data, entry))
    }

    /// Locks, re-reads, changes a copy of the data, saves it, and only then
    /// keeps it. The Python app's `settings_mutation` protocol.
    ///
    /// Safety rule "a failed change changes nothing" (the `self.data = old`
    /// rollback of `pin_many` in core.py, applied here to every change):
    /// the change is made on a copy, so an invalid request or a failed save
    /// leaves [`data`](Self::data) as it was.
    fn mutate<T>(
        &mut self,
        change: impl FnOnce(&mut SettingsData) -> Result<T, SettingsError>,
    ) -> Result<T, SettingsError> {
        let _lock = SettingsLock::acquire(&self.directory)?;
        let mut updated = self.clone();
        updated.reload();
        let result = change(&mut updated.data)?;
        updated.save_while_locked()?;
        *self = updated;
        Ok(result)
    }

    /// Writes the data; the caller holds the [`SettingsLock`].
    fn save_while_locked(&mut self) -> Result<(), SettingsError> {
        // Safety rule "never erase unreadable settings" (a gain over
        // `Settings.save` in core.py): damaged contents are kept as a backup.
        // The match names every state, so a new one cannot fall into
        // `Discard` unnoticed.
        let old_file = match self.file_state {
            FileState::Damaged(_) => OldFile::KeepAsBackup,
            FileState::Sound | FileState::Refused(_) | FileState::BackedUp(_) => OldFile::Discard,
        };
        let contents = self.data.to_file_text();
        let backup = replace_private_file(
            &self.path(),
            Self::TEMPORARY_PREFIX,
            contents.as_bytes(),
            old_file,
        )?;
        if let Some(backup) = backup {
            let message = format!(
                "Your previous settings could not be read and were kept as “{}”.",
                backup.display()
            );
            self.file_state = FileState::BackedUp(message);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
