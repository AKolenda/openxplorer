// SPDX-License-Identifier: AGPL-3.0-only
//! The private backup of `user-dirs.dirs` and the history of changes.
//!
//! Ports the backup and `folder-location-history.json` parts of
//! `FolderLocations.apply` and `_history` in `desktop/folder_locations.py`.
//! Both apps share the history file, so it keeps the Python format:
//! `{"DOWNLOAD": {"previous": …, "path": …, "backup": …, "changedAt": …}}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use super::{CheckedLocation, RelocationError};
use crate::places::KnownFolder;
use crate::private_storage::{
    private_directory, private_file_if_present, read_limited_text, replace_file_atomically,
    PrivateFileOptions, StorageError, WithPath,
};
use crate::random::{random_hex, NAME_BYTES};

/// The history file in the state directory.
const HISTORY_FILE: &str = "folder-location-history.json";

/// The folder of backups in the state directory.
const BACKUP_FOLDER: &str = "location-backups";

/// The largest history read; it holds at most eight small records.
const HISTORY_LIMIT: u64 = 256 * 1024;

/// Where `folder` was before the last change recorded in `state_directory`;
/// `None` without a history or a record. An unreadable history counts as
/// empty, as `_history` does.
pub(super) fn previous_path(state_directory: &Path, folder: KnownFolder) -> Option<PathBuf> {
    let history = read(state_directory);
    let record = history.get(folder.xdg_key())?;
    let previous = record.get("previous")?.as_str()?;
    Some(PathBuf::from(previous))
}

/// Copies `user_dirs_file` (empty when missing) to a new private file in
/// `<state_directory>/location-backups` and returns its path.
///
/// Safety rule "a private backup first" (`tempfile.mkstemp` and
/// `os.fchmod(0o600)` in `apply`): the folder is 0700 and the copy 0600,
/// flushed to disk before the configuration changes.
///
/// # Errors
///
/// [`RelocationError::Storage`] when the file cannot be read or the backup
/// cannot be written.
pub(super) fn back_up(state_directory: &Path, user_dirs_file: &Path) -> Result<PathBuf, RelocationError> {
    let backups = state_directory.join(BACKUP_FOLDER);
    private_directory(state_directory)?;
    private_directory(&backups)?;
    let contents = match fs::read(user_dirs_file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(StorageError::io(user_dirs_file, error).into()),
    };
    let random = random_hex(NAME_BYTES).with_path(&backups)?;
    let backup = backups.join(format!("user-dirs-{random}.dirs"));
    replace_file_atomically(&backup, ".user-dirs-", &contents)?;
    Ok(backup)
}

/// Records that `location.folder` moved from `location.previous` to
/// `location.path`, with its `backup`, keeping every other record.
///
/// # Errors
///
/// [`RelocationError::Storage`] when the history cannot be written.
pub(super) fn record(
    state_directory: &Path,
    location: &CheckedLocation,
    backup: &Path,
) -> Result<(), RelocationError> {
    let mut history = read(state_directory);
    let changed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    let entry = json!({
        "previous": location.previous.to_string_lossy(),
        "path": location.path.to_string_lossy(),
        "backup": backup.to_string_lossy(),
        "changedAt": changed_at,
    });
    history.insert(location.folder.xdg_key().to_owned(), entry);
    let text = serde_json::to_string_pretty(&Value::Object(history))
        .expect("a JSON object of strings and numbers always serialises");
    replace_file_atomically(
        &state_directory.join(HISTORY_FILE),
        ".locations-",
        text.as_bytes(),
    )?;
    Ok(())
}

/// The history as a JSON object; empty when missing, unreadable or not an
/// object.
fn read(state_directory: &Path) -> Map<String, Value> {
    let path = state_directory.join(HISTORY_FILE);
    let Ok(Some(file)) = private_file_if_present(&path, PrivateFileOptions::default()) else {
        return Map::new();
    };
    let Ok(text) = read_limited_text(file, &path, HISTORY_LIMIT) else {
        return Map::new();
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(history)) => history,
        _ => Map::new(),
    }
}
