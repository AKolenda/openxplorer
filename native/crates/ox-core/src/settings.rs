// SPDX-License-Identifier: AGPL-3.0-only
//! Shared settings in `$XDG_CONFIG_HOME/winspace/settings.json`.
//!
//! Ports `Settings` in `desktop/core.py`, on the private storage of
//! `crate::private_storage` (`desktop/private_storage.py`). The Python
//! application and this one use the same file, so the protocol matches
//! exactly:
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
//! file that could not be read completely. It is first renamed to
//! `settings.json.unreadable-…` beside the new file, and the
//! [`warning`](Settings::warning) says where it went. A file private storage
//! refuses is never renamed: the change fails instead.
//!
//! The submodules split the work: `read` reads the file, `mutate` holds
//! the changes, and `save` locks and writes. Reading and saving rely on
//! the private-storage rules of `crate::private_storage`.
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
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};

pub use crate::private_storage::{StorageError, StorageRefusal};
pub use choices::{Appearance, ContextMenu, Theme, View};
pub use error::SettingsError;
pub use model::{Bookmark, RecentEntry, SettingsData};
pub use mutate::{BookmarkAction, BookmarkKind, BookmarkRequest};
pub use preferences::{
    Column, ColumnWidth, ColumnWidths, Preferences, PreferencesUpdate, WindowSize, DEFAULT_TEXT_SIZE,
    NETWORK_INTERVALS, SIDEBAR_ICON_SIZES, SIDEBAR_WIDTHS, TEXT_SIZES, WINDOW_HEIGHTS, WINDOW_WIDTHS,
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
    /// Not read completely: private storage refused the file or its
    /// directory (a symlink, hard link, FIFO, other owner, or an open that
    /// failed), reading it failed, or its contents are too large, not UTF-8,
    /// not JSON or of the wrong shape. The message says why. What the next
    /// change does with the file is decided by [`FileState::old_file`].
    Unreadable(String),
    /// A change replaced an unreadable file after keeping it as a backup;
    /// the message names the backup.
    BackedUp(String),
}

impl FileState {
    /// What the next change does with the file this state describes.
    ///
    /// Safety rule "never erase unreadable settings" (a gain over
    /// `Settings.save` in core.py): a file that was not read completely is
    /// kept as a backup, whatever stopped the read. Keeping it never moves a
    /// file private storage refuses, because [`replace_private_file`] runs
    /// the same checks and fails before it writes or renames anything. If a
    /// refusal has gone away since the read (an open that failed only once),
    /// the file is still kept rather than replaced by settings that never
    /// came from it. The match names every state, so a new one cannot fall
    /// into [`OldFile::Discard`] unnoticed.
    fn old_file(&self) -> OldFile {
        match self {
            // Safety rule "never erase unreadable settings".
            FileState::Unreadable(_) => OldFile::KeepAsBackup,
            // Read completely, or written by the last change: nothing is lost.
            FileState::Sound | FileState::BackedUp(_) => OldFile::Discard,
        }
    }
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
            FileState::Unreadable(message) | FileState::BackedUp(message) => Some(message),
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
    /// [`SettingsError::Storage`] if private storage refuses the settings
    /// directory, lock or file, or if one of them
    /// cannot be opened or written.
    pub fn update_preferences(&mut self, update: &PreferencesUpdate) -> Result<Preferences, SettingsError> {
        self.mutate(|data| {
            data.preferences.apply(update);
            Ok(data.preferences.clone())
        })
    }

    /// Adds or removes a Quick access pin or a mapped share. Removing a pin
    /// hides it from Quick access, which also works for known folders;
    /// adding it shows it again. Removing ignores the requested label.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Location`] for a location or label the Python app
    /// would reject (for example one with credentials), and every error of
    /// [`update_preferences`](Self::update_preferences).
    pub fn bookmark(
        &mut self,
        action: BookmarkAction,
        kind: BookmarkKind,
        request: &BookmarkRequest,
    ) -> Result<(), SettingsError> {
        self.mutate(|data| mutate::apply_bookmark(data, action, kind, request))
    }

    /// Adds or reorders up to 200 Quick access pins in one change and
    /// returns the cleaned pins. The batch is validated as a whole, so an
    /// invalid entry changes nothing. See [`BookmarkRequest`].
    ///
    /// `before` is the entry the folders were dropped on; `quick_order` is
    /// the order the sidebar showed (at most 400 entries).
    ///
    /// # Errors
    ///
    /// [`SettingsError::Invalid`] for an empty or oversized batch, an order
    /// longer than 400 entries, or more than 200 pins in total;
    /// [`SettingsError::Location`] for an invalid location or label in the
    /// batch, `before` or the order; and every error of
    /// [`update_preferences`](Self::update_preferences).
    pub fn pin_many(
        &mut self,
        items: &[BookmarkRequest],
        before: Option<&str>,
        quick_order: Option<&[String]>,
    ) -> Result<Vec<Bookmark>, SettingsError> {
        self.mutate(|data| mutate::pin_many(data, items, before, quick_order))
    }

    /// Records an opened file at the top of the recent files.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Location`] for an invalid file location, and every
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
        let lock = SettingsLock::acquire(&self.directory)?;
        let mut updated = self.clone();
        updated.reload();
        let result = change(&mut updated.data)?;
        updated.save_while_locked(&lock)?;
        *self = updated;
        Ok(result)
    }

    /// Writes the data, keeping an unreadable file as a backup (see
    /// [`FileState::old_file`]).
    ///
    /// Safety rule "changes never interleave" (the `flock` of
    /// `settings_mutation` in core.py): the borrowed [`SettingsLock`] proves
    /// that the caller has held the lock since it re-read the file, so no
    /// other window or Python process can change the file in between.
    fn save_while_locked(&mut self, _lock: &SettingsLock) -> Result<(), SettingsError> {
        let contents = self.data.to_file_text();
        let backup = replace_private_file(
            &self.path(),
            Self::TEMPORARY_PREFIX,
            contents.as_bytes(),
            self.file_state.old_file(),
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
