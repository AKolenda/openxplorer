// SPDX-License-Identifier: AGPL-3.0-only
//! Whitelist validation of an untrusted `settings.json`.
//!
//! Ports the reading half of `Settings.__init__` in `desktop/core.py`.
//! Reading never fails: invalid entries are skipped, lists are capped, and
//! only known preferences within their bounds are kept, so credentials or
//! arbitrary values in the file never reach the application. Like the
//! Python reader, a section of the wrong type (for example `"pins": 5`)
//! stops reading with a warning; sections read before it are kept and the
//! rest keep their defaults.

use serde_json::{Map, Number, Value};

use super::labels::bookmark_fallback_label;
use super::model::{Bookmark, RecentEntry, SettingsData, MAX_BOOKMARKS, MAX_HIDDEN, MAX_ORDER, MAX_RECENT};
use super::preferences::PreferencesUpdate;
use super::SettingsError;
use crate::location::{normalise, python_strip, require_share, safe_label, LocationError};

/// Validates a parsed file into `settings`, which starts as the defaults.
/// Sections read before a problem are kept, as in Python.
pub(super) fn read_settings(source: &Value, settings: &mut SettingsData) -> Result<(), SettingsError> {
    let Some(sections) = source.as_object() else {
        return Err(SettingsError::invalid("Settings must be a JSON object."));
    };
    settings.pins = entry_section(sections, "pins", MAX_BOOKMARKS)?
        .iter()
        .filter_map(|item| read_bookmark(item, normalise))
        .collect();
    settings.shares = entry_section(sections, "shares", MAX_BOOKMARKS)?
        .iter()
        .filter_map(|item| read_bookmark(item, require_share))
        .collect();
    settings.recent = entry_section(sections, "recent", MAX_RECENT)?
        .iter()
        .filter_map(read_recent)
        .collect();
    // Hidden entries are not deduplicated, matching the Python reader.
    settings.hidden_quick = location_section(sections, "hiddenQuick", MAX_HIDDEN)?
        .iter()
        .filter_map(|location| normalise(location).ok())
        .collect();
    for location in location_section(sections, "quickOrder", MAX_ORDER)? {
        let Ok(uri) = normalise(&location) else {
            continue;
        };
        if !settings.quick_order.contains(&uri) {
            settings.quick_order.push(uri);
        }
    }
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
        Some(Value::String(text)) => Ok(text.chars().take(limit).map(String::from).collect()),
        Some(Value::Array(items)) => {
            let as_text = |item: &Value| item.as_str().unwrap_or_default().to_owned();
            Ok(items.iter().take(limit).map(as_text).collect())
        }
        Some(_) => Err(wrong_type(key)),
    }
}

/// The warning for a section that is neither a list nor a string.
fn wrong_type(key: &str) -> SettingsError {
    SettingsError::invalid(format!("“{key}” must be a list."))
}

/// A `{uri, label}` entry whose location passes `check`, or `None` if it
/// is unusable.
fn read_bookmark(item: &Value, check: fn(&str) -> Result<String, LocationError>) -> Option<Bookmark> {
    let uri = check(item.get("uri")?.as_str()?).ok()?;
    let label = item.get("label").and_then(Value::as_str).unwrap_or_default();
    let label = safe_label(label, &bookmark_fallback_label(&uri)).ok()?;
    Some(Bookmark { uri, label })
}

/// A recent-file entry, or `None` if it is unusable. The fields convert
/// with Python's `str()` and `int()`, as `Settings.__init__` does.
fn read_recent(item: &Value) -> Option<RecentEntry> {
    let fields = item.as_object()?;
    let uri = normalise(fields.get("uri")?.as_str()?).ok()?;
    let name = python_str(fields.get("name")?);
    let type_name = fields.get("type").map_or_else(|| "File".to_owned(), python_str);
    let entry = RecentEntry {
        uri,
        name,
        type_name,
        is_dir: false,
        size: python_count(fields.get("size"))?,
        modified: python_count(fields.get("modified"))?,
    };
    Some(entry.into_stored())
}

/// Python's `str(value)` for JSON scalars; lists, objects and fractional
/// numbers use their JSON text instead of Python's `repr`.
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        other => other.to_string(),
    }
}

/// Python's `max(0, int(value or 0))` for a field that may be missing;
/// `None` where `int()` would raise, which makes the reader skip the entry.
fn python_count(value: Option<&Value>) -> Option<u64> {
    let Some(value) = value else {
        return Some(0);
    };
    match value {
        Value::Null | Value::Bool(false) => Some(0),
        Value::Bool(true) => Some(1),
        Value::Number(number) => Some(number_count(number)),
        Value::String(text) if text.is_empty() => Some(0),
        Value::String(text) => parse_python_int(text),
        Value::Array(items) => items.is_empty().then_some(0),
        Value::Object(fields) => fields.is_empty().then_some(0),
    }
}

/// Python's `max(0, int(number))`: a fraction is truncated toward zero, a
/// negative number becomes 0, and a huge one saturates at `u64::MAX`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "`as` truncates toward zero like int(), and max(0.0) plus saturation stand in for max(0, ...)"
)]
fn number_count(number: &Number) -> u64 {
    if let Some(count) = number.as_u64() {
        return count;
    }
    number.as_f64().map_or(0, |float| float.max(0.0) as u64)
}

/// Parses a decimal integer the way Python's `int(str)` does (surrounding
/// white space, a sign, and single underscores between digits), clamped to
/// `0..=u64::MAX`.
fn parse_python_int(text: &str) -> Option<u64> {
    let trimmed = python_strip(text);
    let (is_negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if !is_python_digit_string(digits) {
        return None;
    }
    if is_negative {
        return Some(0);
    }
    let value = digits
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0_u64, |total, digit| {
            total.saturating_mul(10).saturating_add(u64::from(digit - b'0'))
        });
    Some(value)
}

/// Whether `digits` is ASCII digits in groups separated by single
/// underscores, as Python's `int()` accepts (`1_000`, not `1__0` or `_1`).
fn is_python_digit_string(digits: &str) -> bool {
    let is_digit_group = |group: &str| !group.is_empty() && group.bytes().all(|byte| byte.is_ascii_digit());
    !digits.is_empty() && digits.split('_').all(is_digit_group)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::settings::model::MAX_NAME_CHARS;
    use crate::settings::{Column, Theme};

    fn read(value: &Value) -> (SettingsData, Option<SettingsError>) {
        let mut settings = SettingsData::default();
        let problem = read_settings(value, &mut settings).err();
        (settings, problem)
    }

    /// parity: SET-012, SAFE-010, SAFE-018
    #[test]
    fn invalid_entries_are_skipped_without_a_warning() {
        let (data, problem) = read(&json!({
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
        let labels: Vec<&str> = data.pins.iter().map(|pin| pin.label.as_str()).collect();
        assert_eq!(labels, ["Work", "Plain"]);
        assert!(data.shares.is_empty());
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
        let (data, _) = read(&json!({"pins": pins, "quickOrder": order, "recent": recent}));
        assert_eq!(data.pins.len(), MAX_BOOKMARKS);
        assert_eq!(data.quick_order.len(), MAX_ORDER);
        assert_eq!(data.recent.len(), MAX_RECENT);
    }

    /// parity: SET-012
    #[test]
    fn quick_order_is_deduplicated_but_hidden_entries_are_not() {
        let (data, _) = read(&json!({
            "hiddenQuick": ["file:///tmp/a", "file:///tmp/a", 3],
            "quickOrder": ["file:///tmp/a", "file:///tmp/b", "file:///tmp/a"]
        }));
        assert_eq!(data.hidden_quick, ["file:///tmp/a", "file:///tmp/a"]);
        assert_eq!(data.quick_order, ["file:///tmp/a", "file:///tmp/b"]);
    }

    /// parity: SET-013
    #[test]
    fn a_string_location_section_reads_each_character_like_python() {
        let (data, problem) = read(&json!({"hiddenQuick": "/ ", "pins": "text"}));
        assert!(problem.is_none());
        assert_eq!(data.hidden_quick, ["file:///"]);
        assert!(data.pins.is_empty());
    }

    /// parity: SET-013
    #[test]
    fn a_section_of_the_wrong_type_stops_reading_with_a_warning() {
        let (data, problem) = read(&json!({
            "pins": [{"uri": "file:///tmp/kept"}],
            "shares": null,
            "quickOrder": ["file:///tmp/lost"],
            "preferences": {"theme": "dark"}
        }));
        assert!(problem.is_some());
        assert_eq!(data.pins.len(), 1);
        assert!(data.quick_order.is_empty());
        assert_eq!(data.preferences.theme, Theme::System);
    }

    /// parity: SET-013
    #[test]
    fn non_object_files_and_preferences_warn() {
        assert!(read(&json!([1, 2])).1.is_some());
        assert!(read(&json!({"preferences": []})).1.is_some());
        assert!(read(&json!({"preferences": null})).1.is_some());
        assert!(read(&json!({"pins": "text"})).1.is_none());
    }

    /// parity: SET-016
    #[test]
    fn preference_types_follow_python() {
        let (data, _) = read(&json!({"preferences": {
            "textSize": 150.0, "networkInterval": 30.0, "sidebarWidth": true,
            "details": "no", "showHidden": true, "columnWidths": {"name": 300, "type": true, "css": 1}
        }}));
        let prefs = data.preferences;
        assert_eq!(prefs.text_size, 100);
        assert_eq!(prefs.network_interval, 30);
        assert_eq!(prefs.sidebar_width, None);
        assert!(prefs.details && prefs.show_hidden);
        let columns = prefs.column_widths.unwrap();
        assert_eq!(columns.get(Column::Name), Some(300));
        assert_eq!(columns.get(Column::Type), None);
    }

    /// parity: HOME-011, SAFE-018
    #[test]
    fn recent_entries_convert_like_python() {
        let (data, _) = read(&json!({"recent": [
            {"uri": "file:///tmp/a.txt", "name": "a.txt", "size": "1_024", "modified": -5, "isDir": true},
            {"uri": "file:///tmp/b.txt", "name": 12, "type": null, "size": 2.9, "modified": []},
            {"uri": "file:///tmp/c.txt", "name": "c", "size": "12.5"},
            {"uri": "file:///tmp/d.txt", "size": 1},
            {"uri": "file:///tmp/e.txt", "name": "é".repeat(600)}
        ]}));
        let names: Vec<&str> = data.recent.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert_eq!(names[..2], ["a.txt", "12"]);
        assert_eq!(names[2].chars().count(), MAX_NAME_CHARS);
        let first = &data.recent[0];
        assert_eq!((first.size, first.modified, first.is_dir), (1024, 0, false));
        assert_eq!(first.type_name, "File");
        let second = &data.recent[1];
        assert_eq!((second.size, second.modified), (2, 0));
        assert_eq!(second.type_name, "None");
    }

    /// parity: HOME-011
    #[test]
    fn text_counts_parse_like_python_int() {
        assert_eq!(parse_python_int(" +42 "), Some(42));
        assert_eq!(parse_python_int("-7"), Some(0));
        assert_eq!(parse_python_int("1__0"), None);
        assert_eq!(parse_python_int("_1"), None);
        assert_eq!(parse_python_int("99999999999999999999999"), Some(u64::MAX));
        assert_eq!(parse_python_int("abc"), None);
    }
}
