// SPDX-License-Identifier: AGPL-3.0-only
//! Reading and writing a Brave profile's `Preferences` file, and the
//! undo record of a sync.
//!
//! Ports `read_object` and `atomic_bytes` of
//! `v2.0.0:desktop/brave_integration.py` and the record format of
//! `BraveIntegration.sync`. Only `download.default_directory` and
//! `savefile.default_directory` are changed, and
//! `download.prompt_for_download` when the user asks for it (INT-034);
//! every other preference is written back with the value it was read
//! with. Numbers are parsed exactly (`serde_json`'s `float_roundtrip`, as
//! Python's `json` parses them), so a 17-digit zoom level survives a sync
//! and a restore. Keys come back in sorted order, which Brave reads the
//! same, since JSON objects are unordered.

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

/// The preference inside `download` that makes Brave ask where to save
/// each file ("Ask where to save each file before downloading").
const PROMPT_FOR_DOWNLOAD: &str = "prompt_for_download";

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

/// One preference before a sync: whether it was set, and to what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct PreviousValue {
    present: bool,
    value: Value,
}

impl PreviousValue {
    /// The value `preference` holds now, if any.
    fn of(preference: Option<&Value>) -> Self {
        Self {
            present: preference.is_some(),
            value: preference.cloned().unwrap_or(Value::Null),
        }
    }

    /// Puts the value back as `key` of `preferences`, or removes `key` if
    /// it was not set.
    fn put_back(&self, preferences: &mut Map<String, Value>, key: &str) {
        if self.present {
            preferences.insert(key.to_owned(), self.value.clone());
        } else {
            preferences.remove(key);
        }
    }
}

/// What a sync changed in one profile, so that restore can undo exactly
/// that: `{"profile", "previous", "applied", "backup"}` as in Python, and
/// `prompt_for_download` when the sync also set that (INT-034). A record
/// without it, as the Python app and earlier versions wrote, restores the
/// folders only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct UndoRecord {
    pub(super) profile: String,
    pub(super) previous: BTreeMap<String, PreviousValue>,
    pub(super) applied: String,
    pub(super) backup: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt_for_download: Option<PreviousValue>,
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
pub(super) fn previous_folder(data: &Map<String, Value>, group: DownloadPreference) -> PreviousValue {
    let folder = data
        .get(group.as_str())
        .and_then(Value::as_object)
        .and_then(|preferences| preferences.get(DEFAULT_DIRECTORY));
    PreviousValue::of(folder)
}

/// Whether Brave asks where to save each download now, for the undo
/// record.
pub(super) fn previous_download_prompt(data: &Map<String, Value>) -> PreviousValue {
    let prompt = data
        .get(DownloadPreference::Download.as_str())
        .and_then(Value::as_object)
        .and_then(|preferences| preferences.get(PROMPT_FOR_DOWNLOAD));
    PreviousValue::of(prompt)
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

/// Makes Brave ask where to save each download, creating a missing
/// `download` group.
pub(super) fn set_download_prompt(data: &mut Map<String, Value>) {
    let preferences = data
        .entry(DownloadPreference::Download.as_str())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(preferences) = preferences {
        preferences.insert(PROMPT_FOR_DOWNLOAD.to_owned(), Value::Bool(true));
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
        previous.put_back(preferences, DEFAULT_DIRECTORY);
        restored.push(group);
    }
    restored
}

/// Puts back whether Brave asked where to save each download, if the
/// sync of `record` set it and it is still `true`, and returns whether it
/// was put back.
///
/// Safety rule "never overwrite a later choice", as for the folders: a
/// prompt the user turned off since the sync is left alone.
pub(super) fn restore_download_prompt(data: &mut Map<String, Value>, record: &UndoRecord) -> bool {
    let Some(previous) = &record.prompt_for_download else {
        return false;
    };
    let Some(preferences) = data
        .get_mut(DownloadPreference::Download.as_str())
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    if preferences.get(PROMPT_FOR_DOWNLOAD) != Some(&Value::Bool(true)) {
        return false;
    }
    previous.put_back(preferences, PROMPT_FOR_DOWNLOAD);
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A preference file over 32 MB is refused before it is read; one at
    /// the limit is read.
    ///
    /// parity: SAFE-020
    #[test]
    fn a_preference_file_over_32_mb_is_refused() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join("Preferences");
        let file = File::create(&path).expect("fixture");
        // Sparse, so the test writes nothing to disk.
        file.set_len(MAX_PREFERENCES_BYTES + 1).expect("fixture size");

        let refused = read_preferences(&path);

        assert!(
            matches!(refused, Err(BraveError::NotPrivateRegularFile)),
            "{refused:?}"
        );
        std::fs::write(&path, "{}").expect("fixture");
        assert!(read_preferences(&path).is_ok());
    }
}
