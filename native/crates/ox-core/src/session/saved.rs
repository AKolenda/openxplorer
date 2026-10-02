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
use crate::location::without_user;
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
    /// The session in its persistent JSON form. Account names stay in the
    /// current session only, just as they do for saved places.
    pub fn to_json(&self) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|tab| {
                let panes: Vec<Value> = tab
                    .panes
                    .iter()
                    .cloned()
                    .map(without_accounts)
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
        .map(|pane| TabSnapshot::from_json(pane).map(without_accounts))
        .collect::<Result<Vec<_>, _>>()?;
    let active_pane = index_in(value.get("activePane"), panes.len());
    Ok(SavedTab { panes, active_pane })
}

/// Removes account names only at the persistent-session boundary.
/// In-process tab transfers keep their original addresses so they reach
/// the mounted account; reopening a saved tab asks for that account again.
fn without_accounts(mut pane: TabSnapshot) -> TabSnapshot {
    pane.uri = without_user(&pane.uri);
    for uri in pane.history.iter_mut().chain(pane.selection.iter_mut()) {
        *uri = without_user(uri);
    }
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

    /// Saving and reading session files strips account names from every
    /// address, without changing a tab's in-memory handoff state.
    ///
    /// parity: SAFE-010, TAB-053
    #[test]
    fn saved_sessions_do_not_keep_remote_accounts() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let raw = json!({
            "uri": "sftp://demo@server/two",
            "history": ["sftp://demo@server/one", "sftp://demo@server/two"],
            "index": 1,
            "selection": ["sftp://demo@server/two/notes.txt"]
        });
        let pane = TabSnapshot::from_json(&raw).expect("a session account is valid");
        let session = SavedSession {
            tabs: vec![SavedTab {
                panes: vec![pane.clone()],
                active_pane: 0,
            }],
            active_tab: 0,
        };

        session.save(directory.path()).expect("the session is saved");
        let text = std::fs::read_to_string(session_file(directory.path())).expect("the saved session");
        assert!(!text.contains("demo@"), "no location field persists an account");
        assert_eq!(session.tabs[0].panes[0], pane, "the live tab is unchanged");
        assert_eq!(pane.to_json()["uri"], raw["uri"], "tab handoffs keep the account");

        // An earlier version may already have saved account-bearing URIs.
        let earlier = json!({"tabs": [{"panes": [raw], "activePane": 0}], "activeTab": 0});
        std::fs::write(session_file(directory.path()), earlier.to_string()).expect("an older session");
        let restored = SavedSession::load(directory.path()).expect("the older session is valid");
        let restored = &restored.tabs[0].panes[0];
        assert_eq!(restored.uri, "sftp://server/two");
        assert_eq!(restored.history, ["sftp://server/one", "sftp://server/two"]);
        assert_eq!(restored.selection, ["sftp://server/two/notes.txt"]);
        assert_eq!(restored.index, 1);
    }
}
