// SPDX-License-Identifier: AGPL-3.0-only
//! The contents of `settings.json` and the limits each list obeys.
//!
//! Ports the `Settings.data` layout of `desktop/core.py`. Field order
//! matches the Python dictionaries, so both applications write the same
//! file layout. The preferences themselves are in the `preferences` module.

use serde::Serialize;

use super::preferences::Preferences;

/// Format version written to `settings.json`.
const SETTINGS_VERSION: u32 = 2;

/// Most Quick access pins, and most mapped shares, kept.
pub(super) const MAX_BOOKMARKS: usize = 200;

/// Most recent files kept.
pub(super) const MAX_RECENT: usize = 30;

/// Most hidden Quick access locations kept.
pub(super) const MAX_HIDDEN: usize = 200;

/// Longest Quick access order kept.
pub(super) const MAX_ORDER: usize = 400;

/// Longest recent-file name kept, in characters.
pub(super) const MAX_NAME_CHARS: usize = 512;

/// Longest recent-file type kept, in characters.
const MAX_TYPE_CHARS: usize = 200;

/// A saved location: a Quick access pin or a mapped network share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bookmark {
    /// Canonical location URI.
    pub uri: String,
    /// Sidebar label; at most 120 characters, no control characters.
    pub label: String,
}

/// A recently opened file, shown on the Home page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    /// Canonical file URI.
    pub uri: String,
    /// Display name, at most 512 characters when read back.
    pub name: String,
    /// Human-readable type, for example "PDF document".
    #[serde(rename = "type")]
    pub type_name: String,
    /// Always `false` when read back: only files are remembered.
    pub is_dir: bool,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub modified: u64,
}

impl RecentEntry {
    /// This entry as `settings.json` keeps it.
    ///
    /// Safety rule "recent entries are bounded" (SAFE-018; the slicing in
    /// `Settings.__init__`, core.py): the name and type are cut to 512 and
    /// 200 characters, and the entry is never a folder, because only opened
    /// files are remembered.
    pub(super) fn into_stored(self) -> Self {
        Self {
            name: first_chars(&self.name, MAX_NAME_CHARS),
            type_name: first_chars(&self.type_name, MAX_TYPE_CHARS),
            is_dir: false,
            ..self
        }
    }
}

/// The validated contents of `settings.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsData {
    /// Quick access pins, in the order they were added.
    pub pins: Vec<Bookmark>,
    /// Mapped network shares.
    pub shares: Vec<Bookmark>,
    /// Quick access locations the user unpinned, including known folders.
    pub hidden_quick: Vec<String>,
    /// The Quick access order the user dragged into place.
    pub quick_order: Vec<String>,
    /// Recently opened files, newest first.
    pub recent: Vec<RecentEntry>,
    /// Preferences.
    pub preferences: Preferences,
}

impl SettingsData {
    /// The file contents as a JSON value, in the layout both apps write.
    ///
    /// # Panics
    ///
    /// Never: the data holds only strings, numbers, flags and lists.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.file_layout()).expect("settings data holds only strings, numbers and lists")
    }

    /// The file contents as pretty-printed JSON with a final newline,
    /// matching Python's `json.dump(..., indent=2)` key order.
    ///
    /// # Panics
    ///
    /// Never: the data holds only strings, numbers, flags and lists.
    pub(super) fn to_file_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.file_layout())
            .expect("settings data holds only strings, numbers and lists");
        text.push('\n');
        text
    }

    /// The data with the format version, in the order the file lists it.
    fn file_layout(&self) -> FileLayout<'_> {
        FileLayout {
            version: SETTINGS_VERSION,
            pins: &self.pins,
            shares: &self.shares,
            hidden_quick: &self.hidden_quick,
            quick_order: &self.quick_order,
            recent: &self.recent,
            preferences: &self.preferences,
        }
    }
}

/// `settings.json` field order as the Python app writes it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileLayout<'a> {
    version: u32,
    pins: &'a [Bookmark],
    shares: &'a [Bookmark],
    hidden_quick: &'a [String],
    quick_order: &'a [String],
    recent: &'a [RecentEntry],
    preferences: &'a Preferences,
}

/// The first `limit` characters of `text`, like Python's `text[:limit]`.
fn first_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SET-012
    #[test]
    fn file_text_uses_the_python_layout() {
        let text = SettingsData::default().to_file_text();

        let top_level_keys: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("  \""))
            .map(|line| line.trim().split('"').nth(1).unwrap())
            .collect();

        assert_eq!(
            top_level_keys,
            [
                "version",
                "pins",
                "shares",
                "hiddenQuick",
                "quickOrder",
                "recent",
                "preferences"
            ]
        );
        assert!(text.ends_with("}\n"));
    }

    /// parity: SAFE-018, HOME-011
    #[test]
    fn a_stored_recent_entry_is_a_bounded_file() {
        let opened = RecentEntry {
            uri: "file:///tmp/long".into(),
            name: "n".repeat(600),
            type_name: "t".repeat(300),
            is_dir: true,
            size: 7,
            modified: 9,
        };

        let stored = opened.clone().into_stored();

        assert_eq!(stored.name.chars().count(), MAX_NAME_CHARS);
        assert_eq!(stored.type_name.chars().count(), MAX_TYPE_CHARS);
        assert!(!stored.is_dir);
        assert_eq!((stored.uri, stored.size, stored.modified), (opened.uri, 7, 9));
    }
}
