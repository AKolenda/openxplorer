// SPDX-License-Identifier: AGPL-3.0-only
//! Reading `settings.json`: the private-storage checks, then a whitelist
//! validation of the untrusted contents.
//!
//! Ports the reading half of `Settings.__init__` in `v2.0.0:desktop/core.py`.
//! Reading never fails: invalid entries are skipped, lists are capped, and
//! only known preferences within their bounds are kept, so credentials or
//! arbitrary values in the file never reach the application. Like the
//! Python reader, a section of the wrong type (for example `"pins": 5`)
//! stops reading with a warning; sections read before it are kept and the
//! rest keep their defaults.

use std::fmt::Display;
use std::path::Path;

use serde_json::{Map, Value};

use super::labels::bookmark_fallback_label;
use super::model::{Bookmark, RecentEntry, SettingsData, MAX_BOOKMARKS, MAX_HIDDEN, MAX_ORDER, MAX_RECENT};
use super::preferences::PreferencesUpdate;
use super::python_conversions::{python_count, python_str};
use super::{FileState, Settings, SettingsError};
use crate::location::{normalise, require_share, safe_label, LocationError};
use crate::private_storage::{private_directory, private_file, read_limited_text, PrivateFileOptions};

/// Largest settings file read, in bytes: the default limit of
/// `private_text` in `v2.0.0:desktop/private_storage.py`.
const SETTINGS_SIZE_LIMIT: u64 = 4 * 1024 * 1024;

/// How a location read from the file is checked: [`normalise`] for a pin,
/// [`require_share`] for a mapped share.
type LocationCheck = fn(&str) -> Result<String, LocationError>;

/// Checks the settings directory and file, then reads what is valid into
/// `settings`, which starts as the defaults. Sections read before a problem
/// are kept, as in Python.
pub(super) fn read_file(directory: &Path, settings: &mut SettingsData) -> FileState {
    let outcome = try_read_file(directory, settings);
    file_state_after(outcome)
}

/// The state a read left the file in.
fn file_state_after(outcome: Result<(), SettingsError>) -> FileState {
    match outcome {
        Ok(()) => FileState::Sound,
        // No file yet is a first start, not a problem (Python's
        // `except FileNotFoundError: pass`).
        Err(error) if error.is_not_found() => FileState::Sound,
        // Safety rule "never erase unreadable settings": whatever else
        // stopped the read (a refusal, an I/O error such as `EIO` after the
        // file was opened, or unusable contents), the file is unreadable, so
        // `FileState::old_file` keeps it when the next change is saved.
        Err(error) => FileState::Unreadable(read_warning(&error)),
    }
}

/// [`read_file`], stopping at the first problem: the private-storage checks
/// of the directory and the file come first, then the contents.
fn try_read_file(directory: &Path, settings: &mut SettingsData) -> Result<(), SettingsError> {
    // Reading never creates the directory, but one that exists (or a
    // symlink in its place) must pass the private-directory check.
    if directory.exists() || directory.is_symlink() {
        private_directory(directory)?;
    }
    let path = directory.join(Settings::FILE_NAME);
    let file = private_file(&path, PrivateFileOptions::default())?;
    let text = read_limited_text(file, &path, SETTINGS_SIZE_LIMIT)?;
    let source = parse_json(&text)?;
    read_sections(&source, settings)
}

/// The file's text as JSON; the error keeps serde's line and column.
fn parse_json(text: &str) -> Result<Value, SettingsError> {
    serde_json::from_str(text)
        .map_err(|error| SettingsError::invalid(format!("The settings file is not valid JSON ({error}).")))
}

/// The warning shown when reading fell back to defaults, in the Python
/// app's words.
fn read_warning(error: &impl Display) -> String {
    format!("Could not fully read settings; using safe defaults. {error}")
}

/// Validates a parsed file into `settings`, section by section in the
/// order Python reads them.
fn read_sections(source: &Value, settings: &mut SettingsData) -> Result<(), SettingsError> {
    let Some(sections) = source.as_object() else {
        return Err(SettingsError::invalid("Settings must be a JSON object."));
    };
    settings.pins = read_bookmarks(entry_section(sections, "pins", MAX_BOOKMARKS)?, normalise);
    settings.shares = read_bookmarks(entry_section(sections, "shares", MAX_BOOKMARKS)?, require_share);
    settings.recent = entry_section(sections, "recent", MAX_RECENT)?
        .iter()
        .filter_map(read_recent)
        .collect();
    let hidden = location_section(sections, "hiddenQuick", MAX_HIDDEN)?;
    // Hidden entries are not deduplicated, matching the Python reader.
    settings.hidden_quick = normalised(&hidden);
    let order = location_section(sections, "quickOrder", MAX_ORDER)?;
    settings.quick_order = normalised_without_duplicates(&order);
    if let Some(preferences) = sections.get("preferences") {
        let update = PreferencesUpdate::from_json(preferences)?;
        settings.preferences.apply(&update);
    }
    Ok(())
}

/// The first `limit` items of a section of objects; a missing section is
/// empty. Python iterates a string section character by character, and a
/// character is never an object, so a string also reads as empty. Any
/// other type stops reading.
fn entry_section<'a>(
    sections: &'a Map<String, Value>,
    key: &str,
    limit: usize,
) -> Result<&'a [Value], SettingsError> {
    match sections.get(key) {
        None | Some(Value::String(_)) => Ok(&[]),
        Some(Value::Array(items)) => Ok(&items[..items.len().min(limit)]),
        Some(_) => Err(wrong_type(key)),
    }
}

/// The first `limit` location strings of a section; a non-string item
/// becomes an empty string, which never normalises.
///
/// Python iterates a string section character by character and normalises
/// each character as a path relative to the home folder. That is
/// reproduced so both applications read a damaged file the same way.
fn location_section(
    sections: &Map<String, Value>,
    key: &str,
    limit: usize,
) -> Result<Vec<String>, SettingsError> {
    match sections.get(key) {
        None => Ok(Vec::new()),
        Some(Value::String(text)) => {
            let characters = text.chars().take(limit);
            Ok(characters.map(String::from).collect())
        }
        Some(Value::Array(items)) => {
            let first_items = items.iter().take(limit);
            Ok(first_items.map(location_text).collect())
        }
        Some(_) => Err(wrong_type(key)),
    }
}

/// A location item as text; an item that is not a string becomes an empty
/// string, which never normalises.
fn location_text(item: &Value) -> String {
    item.as_str().unwrap_or_default().to_owned()
}

/// The warning for a section that is neither a list nor a string.
fn wrong_type(key: &str) -> SettingsError {
    SettingsError::invalid(format!("“{key}” must be a list."))
}

/// The usable `{uri, label}` entries of a pins or shares section.
fn read_bookmarks(items: &[Value], check: LocationCheck) -> Vec<Bookmark> {
    items
        .iter()
        .filter_map(|item| read_bookmark(item, check))
        .collect()
}

/// A `{uri, label}` entry whose location passes `check`, or `None` if it
/// is unusable.
fn read_bookmark(item: &Value, check: LocationCheck) -> Option<Bookmark> {
    let stored_uri = item.get("uri")?.as_str()?;
    let uri = check(stored_uri).ok()?;
    let label = item.get("label").and_then(Value::as_str).unwrap_or_default();
    let label = safe_label(label, &bookmark_fallback_label(&uri)).ok()?;
    Some(Bookmark { uri, label })
}

/// A recent-file entry, or `None` if it is unusable. The fields convert
/// with Python's `str()` and `int()`, as `Settings.__init__` does.
fn read_recent(item: &Value) -> Option<RecentEntry> {
    let fields = item.as_object()?;
    let stored_uri = fields.get("uri")?.as_str()?;
    let uri = normalise(stored_uri).ok()?;
    let name = python_str(fields.get("name")?);
    let type_label = fields.get("type").map_or_else(|| "File".to_owned(), python_str);
    let entry = RecentEntry {
        uri,
        name,
        type_label,
        is_dir: false,
        size: python_count(fields.get("size"))?,
        modified: python_count(fields.get("modified"))?,
        opened: fields.get("opened").and_then(Value::as_u64),
    };
    Some(entry.into_stored())
}

/// The canonical form of every location that normalises, in order.
fn normalised(locations: &[String]) -> Vec<String> {
    locations
        .iter()
        .filter_map(|location| normalise(location).ok())
        .collect()
}

/// [`normalised`], keeping only the first of equal locations.
fn normalised_without_duplicates(locations: &[String]) -> Vec<String> {
    let mut unique = Vec::with_capacity(locations.len());
    for uri in normalised(locations) {
        if !unique.contains(&uri) {
            unique.push(uri);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use rustix::io::Errno;
    use std::io;

    use serde_json::json;

    use super::*;
    use crate::settings::model::MAX_NAME_CHARS;
    use crate::settings::save::OldFile;
    use crate::settings::{Column, StorageError, Theme};

    /// What reading one parsed file produced.
    struct Outcome {
        settings: SettingsData,
        /// Why reading stopped early; it becomes the warning.
        problem: Option<SettingsError>,
    }

    fn read(source: &Value) -> Outcome {
        let mut settings = SettingsData::default();
        let problem = read_sections(source, &mut settings).err();
        Outcome { settings, problem }
    }

    /// A read of `settings.json` that failed with the operating-system
    /// error `error`.
    fn failed_read(error: io::Error) -> Result<(), SettingsError> {
        let path = Path::new("/state/winspace/settings.json");
        Err(SettingsError::from(StorageError::io(path, error)))
    }

    /// Safety rule "never erase unreadable settings": an I/O error after
    /// the file was opened (`EIO` from a failing disk) says nothing about
    /// its contents, so the next change keeps the file as a backup instead
    /// of replacing it.
    /// parity: SET-013
    #[test]
    fn a_read_error_after_opening_leaves_the_file_unreadable() {
        let eio = io::Error::from(Errno::IO);

        let state = file_state_after(failed_read(eio));

        assert!(matches!(state, FileState::Unreadable(_)), "{state:?}");
        assert_eq!(state.old_file(), OldFile::KeepAsBackup);
    }

    /// A missing file is a first start, which Python reads without a
    /// warning (`except FileNotFoundError: pass`).
    #[test]
    fn a_missing_file_is_read_without_a_warning() {
        let missing = io::Error::from(io::ErrorKind::NotFound);

        let state = file_state_after(failed_read(missing));

        assert_eq!(state, FileState::Sound);
    }

    /// parity: SET-012, SAFE-010, SAFE-018
    #[test]
    fn invalid_entries_are_skipped_without_a_warning() {
        let Outcome { settings, problem } = read(&json!({
            "pins": [
                {"uri": "file:///tmp/Work", "label": "Work"},
                {"uri": "https://example.invalid/", "label": "Web"},
                {"uri": "smb://user:secret@nas/share"},
                {"uri": 5},
                {"label": "no uri"},
                "file:///tmp/string",
                {"uri": "file:///tmp/Bad", "label": "bad\nlabel"},
                {"uri": "file:///tmp/Plain", "label": 7}
            ],
            "shares": [{"uri": "smb://nas/"}, {"uri": "file:///tmp"}],
            "password": "not-stored"
        }));
        assert!(problem.is_none());
        let labels: Vec<&str> = settings.pins.iter().map(|pin| pin.label.as_str()).collect();
        assert_eq!(labels, ["Work", "Plain"]);
        assert!(settings.shares.is_empty());
    }

    /// parity: SET-012
    #[test]
    fn lists_are_capped() {
        let pins: Vec<Value> = (0..250)
            .map(|i| json!({"uri": format!("file:///tmp/p{i}")}))
            .collect();
        let order: Vec<Value> = (0..450).map(|i| json!(format!("file:///tmp/o{i}"))).collect();
        let recent: Vec<Value> = (0..40)
            .map(|i| json!({"uri": format!("file:///tmp/r{i}"), "name": "r"}))
            .collect();
        let Outcome { settings, .. } = read(&json!({"pins": pins, "quickOrder": order, "recent": recent}));
        assert_eq!(settings.pins.len(), MAX_BOOKMARKS);
        assert_eq!(settings.quick_order.len(), MAX_ORDER);
        assert_eq!(settings.recent.len(), MAX_RECENT);
    }

    /// parity: SET-012
    #[test]
    fn quick_order_is_deduplicated_but_hidden_entries_are_not() {
        let Outcome { settings, .. } = read(&json!({
            "hiddenQuick": ["file:///tmp/a", "file:///tmp/a", 3],
            "quickOrder": ["file:///tmp/a", "file:///tmp/b", "file:///tmp/a"]
        }));
        assert_eq!(settings.hidden_quick, ["file:///tmp/a", "file:///tmp/a"]);
        assert_eq!(settings.quick_order, ["file:///tmp/a", "file:///tmp/b"]);
    }

    /// parity: SET-013
    #[test]
    fn a_string_location_section_reads_each_character_like_python() {
        let Outcome { settings, problem } = read(&json!({"hiddenQuick": "/ ", "pins": "text"}));
        assert!(problem.is_none());
        assert_eq!(settings.hidden_quick, ["file:///"]);
        assert!(settings.pins.is_empty());
    }

    /// parity: SET-013
    #[test]
    fn a_section_of_the_wrong_type_stops_reading_with_a_warning() {
        let Outcome { settings, problem } = read(&json!({
            "pins": [{"uri": "file:///tmp/kept"}],
            "shares": null,
            "quickOrder": ["file:///tmp/lost"],
            "preferences": {"theme": "dark"}
        }));
        assert!(problem.is_some());
        assert_eq!(settings.pins.len(), 1);
        assert!(settings.quick_order.is_empty());
        assert_eq!(settings.preferences.theme, Theme::System);
    }

    /// parity: SET-013
    #[test]
    fn non_object_files_and_preferences_warn() {
        assert!(read(&json!([1, 2])).problem.is_some());
        assert!(read(&json!({"preferences": []})).problem.is_some());
        assert!(read(&json!({"preferences": null})).problem.is_some());
        assert!(read(&json!({"pins": "text"})).problem.is_none());
    }

    /// parity: SET-016
    #[test]
    fn preference_types_follow_python() {
        let Outcome { settings, .. } = read(&json!({"preferences": {
            "textSize": 150.0, "networkInterval": 30.0, "sidebarWidth": true,
            "details": "no", "showHidden": true, "columnWidths": {"name": 300, "type": true, "css": 1}
        }}));
        let preferences = settings.preferences;
        assert_eq!(preferences.text_size, 100);
        assert_eq!(preferences.network_interval, 30);
        assert_eq!(preferences.sidebar_width, None);
        assert!(preferences.show_details_pane);
        assert!(preferences.show_hidden);
        let columns = preferences.column_widths.unwrap();
        assert_eq!(columns.get(Column::Name), Some(300));
        assert_eq!(columns.get(Column::Type), None);
    }

    /// parity: HOME-011, SAFE-018
    #[test]
    fn recent_entries_convert_like_python() {
        let Outcome { settings, .. } = read(&json!({"recent": [
            {"uri": "file:///tmp/a.txt", "name": "a.txt", "size": "1_024", "modified": -5, "isDir": true},
            {"uri": "file:///tmp/b.txt", "name": 12, "type": null, "size": 2.9, "modified": []},
            {"uri": "file:///tmp/c.txt", "name": "c", "size": "12.5"},
            {"uri": "file:///tmp/d.txt", "size": 1},
            {"uri": "file:///tmp/e.txt", "name": "é".repeat(600)}
        ]}));
        let names: Vec<&str> = settings.recent.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert_eq!(names[..2], ["a.txt", "12"]);
        assert_eq!(names[2].chars().count(), MAX_NAME_CHARS);
        let first = &settings.recent[0];
        assert_eq!((first.size, first.modified, first.is_dir), (1024, 0, false));
        assert_eq!(first.type_label, "File");
        let second = &settings.recent[1];
        assert_eq!((second.size, second.modified), (2, 0));
        assert_eq!(second.type_label, "None");
    }
}
