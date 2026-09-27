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
//! The `winspace` directory name is a compatibility contract; do not rename
//! it.

mod model;
mod mutate;
mod read;
pub mod storage;
mod validate;

use std::io;
use std::path::{Path, PathBuf};

pub use model::{
    Bookmark, Column, ColumnWidths, Preferences, PreferencesUpdate, RecentEntry, SettingsData, CONTEXT_MENUS,
    NETWORK_INTERVALS, SETTINGS_VERSION, SIDEBAR_WIDTHS, TEXT_SIZES, THEMES, VIEWS,
};
pub use mutate::{BookmarkAction, BookmarkKind, PinRequest};
pub use validate::{safe_label, MAX_LABEL_CHARS};

use storage::{private_directory, private_text, replace_private_file, SettingsLock, SETTINGS_SIZE_LIMIT};

/// Why a settings change was refused.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The request or the stored data failed validation (Python's
    /// `ValueError`). The message is user-facing.
    #[error("{0}")]
    Invalid(String),
    /// The file system refused an operation (Python's `OSError`), for
    /// example because `settings.lock` is a symlink.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl SettingsError {
    /// A validation error with a user-facing message.
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// True if this is a missing file or directory.
    fn is_not_found(&self) -> bool {
        matches!(self, Self::Io(error) if error.kind() == io::ErrorKind::NotFound)
    }
}

impl From<crate::location::LocationError> for SettingsError {
    fn from(error: crate::location::LocationError) -> Self {
        Self::Invalid(error.0)
    }
}

impl From<serde_json::Error> for SettingsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

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
    warning: Option<String>,
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
    /// Does not create the directory.
    pub fn open(directory: &Path) -> Self {
        let (data, warning) = load(directory);
        Self {
            directory: directory.to_path_buf(),
            data,
            warning,
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

    /// Why the last read fell back to defaults, if it did.
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// The settings directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The path of `settings.json`.
    pub fn path(&self) -> PathBuf {
        self.directory.join(Self::FILE_NAME)
    }

    /// Writes the current snapshot under the shared lock. This deliberately
    /// replaces the snapshot; use change methods to merge concurrent edits.
    pub fn save(&self) -> Result<(), SettingsError> {
        let _lock = SettingsLock::acquire(&self.directory)?;
        replace_private_file(
            &self.path(),
            Self::TEMPORARY_PREFIX,
            self.data.to_file_text().as_bytes(),
        )
    }

    /// Applies every valid value in `update`, ignores the rest, saves, and
    /// returns the resulting preferences.
    pub fn update_preferences(&mut self, update: &PreferencesUpdate) -> Result<Preferences, SettingsError> {
        self.mutate(|data| {
            data.preferences.apply(update);
            Ok(data.preferences.clone())
        })
    }

    /// Adds or removes a Quick access pin or a mapped share. Removing a pin
    /// hides it from Quick access, which also works for known folders;
    /// adding it shows it again.
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
    pub fn pin_many(
        &mut self,
        items: &[PinRequest],
        before: Option<&str>,
        quick_order: Option<&[String]>,
    ) -> Result<Vec<Bookmark>, SettingsError> {
        self.mutate(|data| mutate::pin_many(data, items, before, quick_order))
    }

    /// Records an opened file at the top of the recent files.
    pub fn remember_open(&mut self, entry: &RecentEntry) -> Result<(), SettingsError> {
        self.mutate(|data| mutate::remember_open(data, entry))
    }

    /// Locks, re-reads, changes a copy of the data, saves it, and only then
    /// keeps it. The Python app's `settings_mutation` protocol.
    fn mutate<T>(
        &mut self,
        change: impl FnOnce(&mut SettingsData) -> Result<T, SettingsError>,
    ) -> Result<T, SettingsError> {
        let _lock = SettingsLock::acquire(&self.directory)?;
        let mut updated = self.clone();
        updated.reload();
        let result = change(&mut updated.data)?;
        replace_private_file(
            &self.path(),
            Self::TEMPORARY_PREFIX,
            updated.data.to_file_text().as_bytes(),
        )?;
        *self = updated;
        Ok(result)
    }
}

/// Reads and validates the settings in `directory`.
fn load(directory: &Path) -> (SettingsData, Option<String>) {
    let mut data = SettingsData::default();
    match read_file(directory, &mut data) {
        Ok(()) => (data, None),
        Err(error) if error.is_not_found() => (data, None),
        Err(error) => {
            let warning = format!("Could not fully read settings; using safe defaults. {error}");
            (data, Some(warning))
        }
    }
}

/// Checks the directory and file, then reads what is valid into `data`.
fn read_file(directory: &Path, data: &mut SettingsData) -> Result<(), SettingsError> {
    if directory.exists() || directory.is_symlink() {
        private_directory(directory)?;
    }
    let text = private_text(&directory.join(Settings::FILE_NAME), SETTINGS_SIZE_LIMIT)?;
    let source: serde_json::Value = serde_json::from_str(&text)?;
    read::read_settings(&source, data)
}

#[cfg(test)]
mod tests;
