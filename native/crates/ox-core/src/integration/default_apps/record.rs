// SPDX-License-Identifier: AGPL-3.0-only
//! `previous-defaults.json`: the handlers the app replaced, so that
//! Restore previous can put them back.
//!
//! Ports `VALID_DESKTOP`, `DesktopIntegration.previous` and
//! `DesktopIntegration._save` in `desktop/desktop_integration.py`. The
//! file lives in the settings folder shared with the Python app, so both
//! apps read what the other wrote: a JSON object from MIME type name to a
//! desktop ID, or to `""` when the type had no handler.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use crate::integration::mime_type::MimeType;
use crate::integration::private_file::write_private_file;
use crate::private_storage::{
    private_directory, private_file, read_limited_text, PrivateFileOptions, StorageError,
};

use super::{DefaultAppsError, APP_ID};

/// The largest record that is read, in bytes, as in the Python app.
const MAX_RECORD_BYTES: u64 = 65_536;

/// A launcher's desktop file ID such as `org.gnome.Nautilus.desktop`.
///
/// Safety rule "only a plain desktop ID is recorded or passed to
/// `xdg-mime`" (`VALID_DESKTOP` in `desktop_integration.py`): the ID is
/// letters, digits and `_ . @ + -` followed by `.desktop`, so a recorded
/// value can never carry a path, spaces or shell syntax.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DesktopId(String);

impl DesktopId {
    /// `id` if it is a plain desktop ID, otherwise `None`.
    pub fn new(id: &str) -> Option<Self> {
        let stem = id.strip_suffix(".desktop")?;
        let is_plain = !stem.is_empty() && stem.chars().all(is_desktop_id_character);
        is_plain.then(|| Self(id.to_owned()))
    }

    /// The app's own desktop ID, [`APP_ID`].
    pub fn openxplorer() -> Self {
        Self(APP_ID.to_owned())
    }

    /// The ID as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The handlers recorded before the app took a type over. `None`
/// records that the type had no handler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PreviousDefaults {
    handlers: BTreeMap<MimeType, Option<DesktopId>>,
}

impl PreviousDefaults {
    /// Reads the record at `path`. A missing, unsafe or unreadable record
    /// counts as empty, and entries that are not a known type with a
    /// plain desktop ID are dropped.
    pub(super) fn read(path: &Path) -> Self {
        let Ok(file) = private_file(path, PrivateFileOptions::default()) else {
            return Self::default();
        };
        let Ok(text) = read_limited_text(file, path, MAX_RECORD_BYTES) else {
            return Self::default();
        };
        let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(&text) else {
            return Self::default();
        };
        let handlers = entries
            .iter()
            .filter_map(|(name, value)| recorded_entry(name, value))
            .collect();
        Self { handlers }
    }

    /// Writes the record to `path` as a private file in a private folder.
    ///
    /// # Errors
    ///
    /// [`DefaultAppsError::RecordNotSaved`] if the folder or file cannot
    /// be written.
    pub(super) fn save(&self, path: &Path) -> Result<(), DefaultAppsError> {
        if let Some(directory) = path.parent() {
            private_directory(directory).map_err(|error| DefaultAppsError::RecordNotSaved {
                path: directory.to_owned(),
                error: into_io_error(error),
            })?;
        }
        write_private_file(path, ".defaults-", self.to_json().as_bytes()).map_err(|error| {
            DefaultAppsError::RecordNotSaved {
                path: path.to_owned(),
                error,
            }
        })
    }

    /// Records `handler` as the previous handler of `mime_type`.
    pub(super) fn record(&mut self, mime_type: MimeType, handler: Option<DesktopId>) {
        self.handlers.insert(mime_type, handler);
    }

    /// Only the entries for ZIP types.
    pub(super) fn zip_only(mut self) -> Self {
        self.handlers.retain(|mime_type, _| mime_type.is_zip());
        self
    }

    /// True if a handler is recorded for any of `mime_types`.
    pub(super) fn has_handler_for_any(&self, mime_types: &[MimeType]) -> bool {
        self.recorded_handlers()
            .any(|(mime_type, _)| mime_types.contains(&mime_type))
    }

    /// The types with a recorded handler, and that handler.
    pub(super) fn recorded_handlers(&self) -> impl Iterator<Item = (MimeType, &DesktopId)> + '_ {
        self.handlers
            .iter()
            .filter_map(|(mime_type, handler)| Some((*mime_type, handler.as_ref()?)))
    }

    /// The record as the JSON object both apps read.
    fn to_json(&self) -> String {
        let entries: serde_json::Map<String, serde_json::Value> = self
            .handlers
            .iter()
            .map(|(mime_type, handler)| (mime_type.as_str().to_owned(), handler_json(handler.as_ref())))
            .collect();
        serde_json::Value::Object(entries).to_string()
    }
}

/// The type and handler of one record entry, or `None` if the entry is
/// not a known type with `""` or a plain desktop ID.
fn recorded_entry(name: &str, value: &serde_json::Value) -> Option<(MimeType, Option<DesktopId>)> {
    let mime_type = MimeType::from_name(name)?;
    let handler = value.as_str()?;
    if handler.is_empty() {
        return Some((mime_type, None));
    }
    Some((mime_type, Some(DesktopId::new(handler)?)))
}

/// How a recorded handler is written: its ID, or `""` for none.
fn handler_json(handler: Option<&DesktopId>) -> serde_json::Value {
    let text = handler.map_or("", DesktopId::as_str);
    serde_json::Value::String(text.to_owned())
}

/// The operating system's error inside a private-storage error, or the
/// refused rule as an error of its own.
fn into_io_error(error: StorageError) -> io::Error {
    match error {
        StorageError::Io { error, .. } => error,
        StorageError::Refused { reason, .. } => io::Error::other(reason),
    }
}

/// The characters `VALID_DESKTOP` allows before `.desktop`.
fn is_desktop_id_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || "_.@+-".contains(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SAFE-020
    #[test]
    fn plain_desktop_ids_are_accepted_and_anything_else_refused() {
        let accepted = [
            "org.kde.dolphin.desktop",
            "a.desktop",
            "io.winspace.Development.desktop",
        ];
        let refused = [
            "",
            ".desktop",
            "evil;command.desktop",
            "with space.desktop",
            "../outside.desktop",
            "nautilus",
            "org.gnome.Nautilus.desktop\n",
        ];

        for id in accepted {
            assert_eq!(DesktopId::new(id).map(|id| id.0), Some(id.to_owned()));
        }
        for id in refused {
            assert_eq!(DesktopId::new(id), None, "{id:?}");
        }
    }

    #[test]
    fn a_saved_record_reads_back_and_ignores_unknown_entries() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join("winspace").join("previous-defaults.json");
        let mut record = PreviousDefaults::default();
        record.record(MimeType::Directory, DesktopId::new("org.kde.dolphin.desktop"));
        record.record(MimeType::Zip, None);

        record.save(&path).expect("save");

        assert_eq!(PreviousDefaults::read(&path), record);
        let text = std::fs::read_to_string(&path).expect("read");
        let with_unknown = text.replace(
            '}',
            r#","text/plain":"gedit.desktop","x-scheme-handler/smb":"a b"}"#,
        );
        std::fs::write(&path, with_unknown).expect("write");
        assert_eq!(PreviousDefaults::read(&path), record);
    }

    #[test]
    fn a_missing_or_malformed_record_is_empty() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join("previous-defaults.json");
        assert_eq!(PreviousDefaults::read(&path), PreviousDefaults::default());

        std::fs::write(&path, "[1, 2]").expect("write");

        assert_eq!(PreviousDefaults::read(&path), PreviousDefaults::default());
    }

    /// The record is state like the settings: a symlinked record is not
    /// read, saving replaces the link rather than writing through it, and
    /// the saved file and its folder are private.
    ///
    /// parity: SAFE-009
    #[test]
    fn a_symlinked_record_is_never_followed_and_the_saved_one_is_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = folder.path().join("winspace");
        std::fs::create_dir(&directory).expect("fixture folder");
        let target = folder.path().join("target.json");
        std::fs::write(&target, r#"{"inode/directory":"org.kde.dolphin.desktop"}"#).expect("fixture");
        let path = directory.join("previous-defaults.json");
        symlink(&target, &path).expect("fixture link");

        assert_eq!(PreviousDefaults::read(&path), PreviousDefaults::default());
        let mut record = PreviousDefaults::default();
        record.record(MimeType::Zip, None);
        record.save(&path).expect("save");

        let target_text = std::fs::read_to_string(&target).expect("read the target");
        assert_eq!(target_text, r#"{"inode/directory":"org.kde.dolphin.desktop"}"#);
        let saved = std::fs::symlink_metadata(&path).expect("stat");
        assert!(saved.is_file(), "the link was replaced by a file");
        assert_eq!(saved.permissions().mode() & 0o777, 0o600);
        let folder_mode = std::fs::metadata(&directory).expect("stat").permissions().mode();
        assert_eq!(folder_mode & 0o777, 0o700);
    }
}
