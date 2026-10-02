// SPDX-License-Identifier: AGPL-3.0-only
//! The tabs of the last window closed, which a start without locations
//! reopens when the settings ask for it (TAB-053, Dolphin's
//! `RememberOpenedTabs`).
//!
//! The file is `session.json` in the settings directory, written as a
//! private file and replaced atomically. It is read back as untrusted
//! data: every pane is a [`TabSnapshot`], with its rules, and the lists
//! are bounded, so a damaged or hostile file restores nothing rather than
//! something unexpected.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{TabSnapshot, WindowStateError};
use crate::private_storage::{
    private_file_if_present, read_limited_text, replace_file_atomically, PrivateFileOptions, StorageError,
};

/// The file's name in the settings directory.
const SESSION_FILE: &str = "session.json";

/// The prefix of the temporary file a save writes first.
const TEMPORARY_PREFIX: &str = ".session.json.";

/// The most tabs a saved session restores.
pub const MAX_SAVED_TABS: usize = 100;

/// The largest session file read, in bytes.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// One tab of a saved window: its pane, or the two panes of a split tab,
/// left first.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedTab {
    /// One or two panes, left first.
    pub panes: Vec<TabSnapshot>,
    /// The index of the active pane in `panes`.
    pub active_pane: usize,
}

/// The tabs of a window, left to right, and which one was in front.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSession {
    /// The tabs, 1 to [`MAX_SAVED_TABS`].
    pub tabs: Vec<SavedTab>,
    /// The index of the tab in front.
    pub active_tab: usize,
}

impl SavedSession {
    /// The session in its JSON form.
    pub fn to_json(&self) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|tab| {
                let panes: Vec<Value> = tab
                    .panes
                    .iter()
                    .cloned()
                    .map(without_administrator_access)
                    .map(|pane| pane.to_json())
                    .collect();
                json!({ "panes": panes, "activePane": tab.active_pane })
            })
            .collect();
        json!({ "tabs": tabs, "activeTab": self.active_tab })
    }

    /// Validates a session in its JSON form.
    ///
    /// # Errors
    ///
    /// [`WindowStateError::InvalidTab`] for anything but 1 to
    /// [`MAX_SAVED_TABS`] tabs of one or two panes, or the first rule a
    /// pane breaks.
    pub fn from_json(value: &Value) -> Result<Self, WindowStateError> {
        let tabs = value
            .get("tabs")
            .and_then(Value::as_array)
            .filter(|tabs| (1..=MAX_SAVED_TABS).contains(&tabs.len()))
            .ok_or(WindowStateError::InvalidTab)?;
        let tabs = tabs.iter().map(saved_tab).collect::<Result<Vec<_>, _>>()?;
        let active_tab = index_in(value.get("activeTab"), tabs.len());
        Ok(Self { tabs, active_tab })
    }

    /// Saves the session in `directory`, replacing the last one.
    ///
    /// # Errors
    ///
    /// [`StorageError`] when the file cannot be written.
    pub fn save(&self, directory: &Path) -> Result<(), StorageError> {
        let contents = self.to_json().to_string();
        replace_file_atomically(&session_file(directory), TEMPORARY_PREFIX, contents.as_bytes())
    }

    /// The session saved in `directory`; `None` when there is none, or it
    /// cannot be read or is not valid.
    pub fn load(directory: &Path) -> Option<Self> {
        let path = session_file(directory);
        let file = private_file_if_present(&path, PrivateFileOptions::default()).ok()??;
        let text = read_limited_text(file, &path, MAX_FILE_BYTES).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Self::from_json(&value).ok()
    }
}

/// Where the session is saved in `directory`.
fn session_file(directory: &Path) -> PathBuf {
    directory.join(SESSION_FILE)
}

/// One tab of the JSON form.
fn saved_tab(value: &Value) -> Result<SavedTab, WindowStateError> {
    let panes = value
        .get("panes")
        .and_then(Value::as_array)
        .filter(|panes| (1..=2).contains(&panes.len()))
        .ok_or(WindowStateError::InvalidTab)?;
    let panes = panes
        .iter()
        .map(|value| TabSnapshot::from_json(value).map(without_administrator_access))
        .collect::<Result<Vec<_>, _>>()?;
    let active_pane = index_in(value.get("activePane"), panes.len());
    Ok(SavedTab { panes, active_pane })
}

/// Restoring a session must not request fresh administrator authentication.
fn without_administrator_access(mut pane: TabSnapshot) -> TabSnapshot {
    let ordinary = |uri: &str| {
        uri.strip_prefix("admin:")
            .map_or_else(|| uri.to_owned(), |path| format!("file:{path}"))
    };
    pane.uri = ordinary(&pane.uri);
    pane.history = pane.history.iter().map(|uri| ordinary(uri)).collect();
    pane.selection = pane.selection.iter().map(|uri| ordinary(uri)).collect();
    pane
}

/// An index into a list of `length` items, the first when it is missing
/// or out of range.
fn index_in(value: Option<&Value>, length: usize) -> usize {
    value
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < length)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved split tab comes back with both panes and its active pane;
    /// a damaged file restores nothing.
    ///
    /// parity: TAB-053
    #[test]
    fn a_saved_session_comes_back_as_it_was_saved() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let pane = |uri: &str| {
            TabSnapshot::from_json(&json!({ "uri": uri, "history": [uri], "index": 0 })).expect("a valid tab")
        };
        let session = SavedSession {
            tabs: vec![
                SavedTab {
                    panes: vec![pane("file:///srv/one")],
                    active_pane: 0,
                },
                SavedTab {
                    panes: vec![pane("file:///srv/two"), pane("file:///srv/three")],
                    active_pane: 1,
                },
            ],
            active_tab: 1,
        };

        session.save(directory.path()).expect("the session is saved");
        let loaded = SavedSession::load(directory.path());
        std::fs::write(directory.path().join(SESSION_FILE), r#"{"tabs": []}"#).expect("written");

        assert_eq!(loaded, Some(session));
        assert_eq!(SavedSession::load(directory.path()), None);
    }
    /// parity: OPS-039
    #[test]
    fn restored_sessions_reopen_administrator_locations_without_elevation() {
        let saved = SavedSession::from_json(&json!({"tabs": [{"panes": [{"uri": "admin:///etc", "history": ["admin:///etc"], "index": 0}], "activePane": 0}], "activeTab": 0})).unwrap();
        assert_eq!(saved.tabs[0].panes[0].uri, "file:///etc");
        assert!(!saved.to_json().to_string().contains("admin:"));
    }
}
