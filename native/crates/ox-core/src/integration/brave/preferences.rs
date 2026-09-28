// SPDX-License-Identifier: AGPL-3.0-only
//! Reading and writing a Brave profile's `Preferences` file, and the
//! undo record of a sync.
//!
//! Ports `read_object` and `atomic_bytes` of
//! `desktop/brave_integration.py` and the record format of
//! `BraveIntegration.sync`. Only `download.default_directory` and
//! `savefile.default_directory` are ever changed; every other preference
//! is written back with the value it was read with. Numbers are parsed
//! exactly (`serde_json`'s `float_roundtrip`, as Python's `json` parses
//! them), so a 17-digit zoom level survives a sync and a restore. Keys
//! come back in sorted order, which Brave reads the same, since JSON
//! objects are unordered.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rustix::fs::OFlags;
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::process::current_user_id;
use super::BraveError;
use crate::integration::private_file::write_private_file;
use crate::private_storage::KernelOpenFlags;

/// The largest preference file that is read, in bytes.
const MAX_PREFERENCES_BYTES: u64 = 32_000_000;

/// The preference inside each [`DownloadPreference`] that holds a folder.
const DEFAULT_DIRECTORY: &str = "default_directory";

/// A preference group whose `default_directory` a sync changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DownloadPreference {
    /// `download`: where downloads are saved.
    Download,
    /// `savefile`: where "Save page as" saves.
    SaveFile,
}

impl DownloadPreference {
    /// Both groups, in the order the record lists them.
    pub const ALL: [Self; 2] = [Self::Download, Self::SaveFile];

    /// The group's key in `Preferences`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::SaveFile => "savefile",
        }
    }

    /// The group with key `name`, if it is one of the two.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.as_str() == name)
    }
}

/// A preference file as read: its exact bytes, to detect a later change,
/// and its parsed contents.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Preferences {
    pub(super) raw: Vec<u8>,
    pub(super) data: Map<String, Value>,
}

/// One group's folder before a sync: whether it was set, and to what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct PreviousFolder {
    present: bool,
    value: Value,
}

/// What a sync changed in one profile, so that restore can undo exactly
/// that: `{"profile", "previous", "applied", "backup"}` as in Python.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct UndoRecord {
    pub(super) profile: String,
    pub(super) previous: BTreeMap<String, PreviousFolder>,
    pub(super) applied: String,
    pub(super) backup: String,
}

/// Reads a preference file or undo record.
///
/// Safety rule "only a private regular file is read" (`read_object` in
/// `brave_integration.py`): the file is opened without following a symlink
/// and without blocking on a FIFO, and must be a regular file of this
/// user of at most 32 MB, checked on the opened file.
///
/// # Errors
///
/// [`BraveError::SymlinkedPreferences`], [`BraveError::NotPrivateRegularFile`],
/// [`BraveError::InvalidJson`] or [`BraveError::NotAnObject`] for a file
/// that is refused, and [`BraveError::Io`] when it cannot be read.
pub(super) fn read_preferences(path: &Path) -> Result<Preferences, BraveError> {
    let file = open_private_regular_file(path)?;
    let mut raw = Vec::new();
    file.take(MAX_PREFERENCES_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| BraveError::io(path, error))?;
    if raw.len() as u64 > MAX_PREFERENCES_BYTES {
        return Err(BraveError::NotPrivateRegularFile);
    }
    let Value::Object(data) = serde_json::from_slice(&raw)? else {
        return Err(BraveError::NotAnObject);
    };
    Ok(Preferences { raw, data })
}

/// Reads the undo record at `path`.
///
/// # Errors
///
/// As [`read_preferences`], and [`BraveError::InvalidJson`] for a record
/// without the expected fields.
pub(super) fn read_undo_record(path: &Path) -> Result<UndoRecord, BraveError> {
    let record = read_preferences(path)?;
    Ok(serde_json::from_value(Value::Object(record.data))?)
}

/// Writes `record` to `path` as a private file.
///
/// # Errors
///
/// [`BraveError::Io`] when the file cannot be written.
pub(super) fn write_undo_record(path: &Path, record: &UndoRecord) -> Result<(), BraveError> {
    let json = serde_json::to_vec(record)?;
    write_private(path, &json)
}

/// Writes `bytes` to `path` as a private file, atomically.
///
/// # Errors
///
/// [`BraveError::Io`] when the file cannot be written.
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), BraveError> {
    write_private_file(path, ".winspace-", bytes).map_err(|error| BraveError::io(path, error))
}

/// `data` as compact JSON, the form Brave writes.
pub(super) fn compact_json(data: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec(data).expect("a JSON map always serialises")
}

/// The folder a group holds now, for the undo record.
pub(super) fn previous_folder(data: &Map<String, Value>, group: DownloadPreference) -> PreviousFolder {
    let folder = data
        .get(group.as_str())
        .and_then(Value::as_object)
        .and_then(|preferences| preferences.get(DEFAULT_DIRECTORY));
    PreviousFolder {
        present: folder.is_some(),
        value: folder.cloned().unwrap_or(Value::Null),
    }
}

/// True if a group that exists is not an object, which no sync touches.
pub(super) fn has_unsupported_structure(data: &Map<String, Value>) -> bool {
    DownloadPreference::ALL
        .iter()
        .filter_map(|group| data.get(group.as_str()))
        .any(|preferences| !preferences.is_object())
}

/// Sets both groups' folder to `folder`, creating a missing group.
pub(super) fn set_download_folders(data: &mut Map<String, Value>, folder: &str) {
    for group in DownloadPreference::ALL {
        let preferences = data
            .entry(group.as_str())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(preferences) = preferences {
            preferences.insert(DEFAULT_DIRECTORY.to_owned(), Value::String(folder.to_owned()));
        }
    }
}

/// Puts back each group of `record` whose folder still is the one the
/// sync applied, and returns the groups put back.
///
/// Safety rule "never overwrite a later choice" (`restore` in
/// `brave_integration.py`): a group whose folder changed since the sync is
/// left alone.
pub(super) fn restore_download_folders(
    data: &mut Map<String, Value>,
    record: &UndoRecord,
) -> Vec<DownloadPreference> {
    let mut restored = Vec::new();
    for (name, previous) in &record.previous {
        let Some(group) = DownloadPreference::from_name(name) else {
            continue;
        };
        let Some(preferences) = data.get_mut(name).and_then(Value::as_object_mut) else {
            continue;
        };
        let applied = Value::String(record.applied.clone());
        if preferences.get(DEFAULT_DIRECTORY) != Some(&applied) {
            continue;
        }
        if previous.present {
            preferences.insert(DEFAULT_DIRECTORY.to_owned(), previous.value.clone());
        } else {
            preferences.remove(DEFAULT_DIRECTORY);
        }
        restored.push(group);
    }
    restored
}

/// Opens `path` for reading if it is a regular file of this user of at
/// most 32 MB, without following a symlink or blocking on a FIFO.
fn open_private_regular_file(path: &Path) -> Result<File, BraveError> {
    let opened = OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::NOFOLLOW | OFlags::NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(Errno::LOOP.raw_os_error()) => {
            return Err(BraveError::SymlinkedPreferences);
        }
        Err(error) => return Err(BraveError::io(path, error)),
    };
    let metadata = file.metadata().map_err(|error| BraveError::io(path, error))?;
    let is_private_regular_file =
        metadata.is_file() && metadata.uid() == current_user_id() && metadata.len() <= MAX_PREFERENCES_BYTES;
    if !is_private_regular_file {
        return Err(BraveError::NotPrivateRegularFile);
    }
    Ok(file)
}
