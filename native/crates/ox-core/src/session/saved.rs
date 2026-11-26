// SPDX-License-Identifier: AGPL-3.0-only
//! The tabs of the windows open last, which a start without locations
//! reopens when the settings ask for it (TAB-053, Dolphin's
//! `RememberOpenedTabs`, and Windows 11's "Restore previous folder windows
//! at logon", which reopens every folder window with its tabs).
//!
//! The window in front keeps the one-window form at the top of the file
//! (`tabs` and `activeTab`), so a version that knows only one window still
//! reopens it; the other windows follow in `otherWindows`.
//!
//! The file is `session.json` in the settings directory, written as a
//! private file and replaced atomically. It is read back as untrusted
//! data: every pane is a [`TabSnapshot`], with its rules, and the lists
//! are bounded, so a damaged or hostile file restores nothing rather than
//! something unexpected.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{TabSnapshot, WindowStateError};
use crate::location::{without_user, ArchiveLocation};
use crate::private_storage::{
    private_file_if_present, read_limited_text, replace_file_atomically, PrivateFileOptions, StorageError,
};

/// The file's name in the settings directory.
const SESSION_FILE: &str = "session.json";

/// The prefix of the temporary file a save writes first.
const TEMPORARY_PREFIX: &str = ".session.json.";

/// The most tabs a saved window restores.
pub const MAX_SAVED_TABS: usize = 100;

/// The most windows a saved session restores, the one in front included.
pub const MAX_SAVED_WINDOWS: usize = 20;

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
                    .map(for_saved_session)
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

    /// The window in front of the session saved in `directory`; `None`
    /// when there is none, or it cannot be read or is not valid.
    pub fn load(directory: &Path) -> Option<Self> {
        Self::from_json(&read_session(directory)?).ok()
    }
}

/// Every window of a saved session, the one in front first.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedWindows {
    /// 1 to [`MAX_SAVED_WINDOWS`] windows, the one in front first.
    pub windows: Vec<SavedSession>,
}

impl SavedWindows {
    /// The windows in their persistent JSON form: the front window's tabs
    /// at the top, as [`SavedSession::to_json`] writes them, and the
    /// others in `otherWindows`.
    pub fn to_json(&self) -> Value {
        windows_json(&self.windows)
    }

    /// Validates saved windows in their JSON form: the front window must
    /// be valid; another window that is not is left out, and windows past
    /// [`MAX_SAVED_WINDOWS`] are too.
    pub fn from_json(value: &Value) -> Option<Self> {
        let front = SavedSession::from_json(value).ok()?;
        let others = value
            .get("otherWindows")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let windows = std::iter::once(front)
            .chain(
                others
                    .iter()
                    .filter_map(|window| SavedSession::from_json(window).ok()),
            )
            .take(MAX_SAVED_WINDOWS)
            .collect();
        Some(Self { windows })
    }

    /// Saves the windows in `directory`, replacing the last session.
    ///
    /// Every window may hold up to [`MAX_SAVED_TABS`] tabs, each with a
    /// long history and many selected items, so twenty of them can take
    /// more than the session file read back ([`MAX_FILE_BYTES`]), which
    /// would then reopen nothing. Windows are left out from the last, the
    /// one used longest ago, until the rest fit; the window in front is
    /// always kept, as one window alone always was.
    ///
    /// # Errors
    ///
    /// [`StorageError`] when the file cannot be written.
    pub fn save(&self, directory: &Path) -> Result<(), StorageError> {
        let mut kept = self.windows.len().min(MAX_SAVED_WINDOWS);
        let contents = loop {
            let contents = windows_json(&self.windows[..kept]).to_string();
            let fits = u64::try_from(contents.len()).unwrap_or(u64::MAX) <= MAX_FILE_BYTES;
            if fits || kept <= 1 {
                break contents;
            }
            kept -= 1;
        };
        replace_file_atomically(&session_file(directory), TEMPORARY_PREFIX, contents.as_bytes())
    }

    /// The windows saved in `directory`; `None` when there are none, or
    /// the front window cannot be read or is not valid.
    pub fn load(directory: &Path) -> Option<Self> {
        Self::from_json(&read_session(directory)?)
    }
}

/// The first [`MAX_SAVED_WINDOWS`] of `windows` in the JSON form
/// [`SavedWindows::to_json`] writes.
fn windows_json(windows: &[SavedSession]) -> Value {
    let mut windows = windows.iter().take(MAX_SAVED_WINDOWS);
    let mut value = windows.next().map_or_else(|| json!({}), SavedSession::to_json);
    let others: Vec<Value> = windows.map(SavedSession::to_json).collect();
    if !others.is_empty() {
        value["otherWindows"] = Value::Array(others);
    }
    value
}

/// The session file in `directory` as JSON, when it can be read.
fn read_session(directory: &Path) -> Option<Value> {
    let path = session_file(directory);
    let file = private_file_if_present(&path, PrivateFileOptions::default()).ok()??;
    let text = read_limited_text(file, &path, MAX_FILE_BYTES).ok()?;
    serde_json::from_str(&text).ok()
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
        .map(|pane| TabSnapshot::from_json(pane).map(for_saved_session))
        .collect::<Result<Vec<_>, _>>()?;
    let active_pane = index_in(value.get("activePane"), panes.len());
    Ok(SavedTab { panes, active_pane })
}

/// Removes account names and administrator access at the persistent-session boundary.
/// In-process tab transfers keep their original addresses so they reach
/// the mounted account; reopening a saved tab asks for that account again.
fn for_saved_session(mut pane: TabSnapshot) -> TabSnapshot {
    pane.uri = saved_uri(&pane.uri);
    for uri in pane.history.iter_mut().chain(pane.selection.iter_mut()) {
        *uri = saved_uri(uri);
    }
    pane
}

/// ZIP locations wrap their source URI, which needs the same privacy and
/// elevation rules as a directly browsed folder.
fn saved_uri(uri: &str) -> String {
    if let Some(mut archive) = ArchiveLocation::from_uri(uri) {
        archive.archive_uri = saved_uri(&archive.archive_uri);
        return archive.uri();
    }
    let uri = without_user(uri);
    uri.strip_prefix("admin:")
        .map_or_else(|| uri.clone(), |path| format!("file:{path}"))
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

    /// Every window of a saved session comes back, the one in front first,
    /// as Windows 11 reopens every folder window with its tabs. The window
    /// in front keeps the one-window form, so a file saved before several
    /// windows were kept, or read by such a version, still opens it; a
    /// damaged other window is left out, and only so many windows return.
    ///
    /// parity: TAB-053
    #[test]
    fn every_saved_window_comes_back_with_the_front_one_first() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let pane = |uri: &str| {
            TabSnapshot::from_json(&json!({ "uri": uri, "history": [uri], "index": 0 })).expect("a valid tab")
        };
        let window = |uris: &[&str], active_tab: usize| SavedSession {
            tabs: uris
                .iter()
                .map(|uri| SavedTab {
                    panes: vec![pane(uri)],
                    active_pane: 0,
                })
                .collect(),
            active_tab,
        };
        let windows = SavedWindows {
            windows: vec![
                window(&["file:///srv/front", "file:///srv/front/Plans"], 1),
                window(&["file:///srv/second"], 0),
                window(
                    &["file:///srv/third", "file:///srv/third/A", "file:///srv/third/B"],
                    2,
                ),
            ],
        };

        windows.save(directory.path()).expect("the windows are saved");
        let loaded = SavedWindows::load(directory.path());
        let front_only = SavedSession::load(directory.path());

        assert_eq!(loaded.as_ref(), Some(&windows));
        assert_eq!(
            front_only.as_ref(),
            Some(&windows.windows[0]),
            "an older version reads the front window"
        );

        // A file of one window, as earlier versions saved it.
        windows.windows[1]
            .save(directory.path())
            .expect("one window is saved");
        assert_eq!(
            SavedWindows::load(directory.path()),
            Some(SavedWindows {
                windows: vec![windows.windows[1].clone()]
            })
        );

        // A damaged other window is left out; the others come back.
        let mut value = windows.to_json();
        value["otherWindows"][0] = json!({ "tabs": [] });
        let read = SavedWindows::from_json(&value).expect("the front window is valid");
        assert_eq!(
            read.windows,
            [windows.windows[0].clone(), windows.windows[2].clone()]
        );

        // At most MAX_SAVED_WINDOWS windows return.
        let many = json!({
            "tabs": [{ "panes": [pane("file:///srv/front").to_json()], "activePane": 0 }],
            "activeTab": 0,
            "otherWindows": vec![window(&["file:///srv/more"], 0).to_json(); MAX_SAVED_WINDOWS + 5],
        });
        let read = SavedWindows::from_json(&many).expect("the front window is valid");
        assert_eq!(read.windows.len(), MAX_SAVED_WINDOWS);

        // Without a valid front window nothing is restored.
        assert_eq!(
            SavedWindows::from_json(
                &json!({ "tabs": [], "otherWindows": [window(&["file:///srv/x"], 0).to_json()] })
            ),
            None
        );
    }

    /// Twenty windows of a hundred tabs with long histories take more than
    /// the session file read back: the windows used longest ago are left
    /// out until the rest fit, so the next start still reopens the window
    /// in front and as many others as fit, rather than nothing.
    ///
    /// parity: TAB-053
    #[test]
    fn a_session_too_large_to_read_back_keeps_the_windows_that_fit() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let long = "x".repeat(90);
        let pane = |window: usize, tab: usize| {
            let history: Vec<String> = (0..20)
                .map(|step| format!("file:///srv/example/{long}/{window}/{tab}/{step}"))
                .collect();
            TabSnapshot::from_json(&json!({ "uri": history[19], "history": history, "index": 19 }))
                .expect("a valid tab")
        };
        let windows = SavedWindows {
            windows: (0..MAX_SAVED_WINDOWS)
                .map(|window| SavedSession {
                    tabs: (0..MAX_SAVED_TABS)
                        .map(|tab| SavedTab {
                            panes: vec![pane(window, tab)],
                            active_pane: 0,
                        })
                        .collect(),
                    active_tab: 0,
                })
                .collect(),
        };
        let whole = u64::try_from(windows.to_json().to_string().len()).expect("a length");
        assert!(whole > MAX_FILE_BYTES, "{whole} bytes whole");

        windows.save(directory.path()).expect("the windows are saved");
        let loaded = SavedWindows::load(directory.path()).expect("the session is read back");

        let count = loaded.windows.len();
        assert!(count > 1 && count < MAX_SAVED_WINDOWS, "{count} windows");
        assert_eq!(
            loaded.windows[..],
            windows.windows[..count],
            "the front ones, in order"
        );
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
    /// parity: OPS-039
    #[test]
    fn restored_sessions_reopen_administrator_locations_without_elevation() {
        let saved = SavedSession::from_json(&json!({"tabs": [{"panes": [{"uri": "admin:///etc", "history": ["admin:///etc"], "index": 0}], "activePane": 0}], "activeTab": 0})).unwrap();
        assert_eq!(saved.tabs[0].panes[0].uri, "file:///etc");
        assert!(!saved.to_json().to_string().contains("admin:"));
    }

    /// parity: SAFE-010, OPS-039, TAB-053
    #[test]
    fn archive_sessions_strip_accounts_and_elevation_from_every_location() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        for (original, expected) in [
            ("sftp://demo@server/srv/plans.zip", "sftp://server/srv/plans.zip"),
            ("admin:///etc/plans.zip", "file:///etc/plans.zip"),
        ] {
            let archive = ArchiveLocation::root(original);
            let raw = json!({
                "uri": archive.member("Docs/").uri(),
                "history": [archive.uri(), archive.member("Docs/").uri()],
                "index": 1,
                "selection": [archive.member("Docs/notes.txt").uri()]
            });
            let pane = TabSnapshot::from_json(&raw).expect("a valid live tab");
            let session = SavedSession {
                tabs: vec![SavedTab {
                    panes: vec![pane.clone()],
                    active_pane: 0,
                }],
                active_tab: 0,
            };

            session.save(directory.path()).unwrap();

            let check = |restored: SavedSession| {
                let restored = &restored.tabs[0].panes[0];
                for uri in std::iter::once(&restored.uri)
                    .chain(&restored.history)
                    .chain(&restored.selection)
                {
                    assert_eq!(ArchiveLocation::from_uri(uri).unwrap().archive_uri, expected);
                }
                assert_eq!(
                    ArchiveLocation::from_uri(&restored.selection[0]).unwrap().member,
                    "Docs/notes.txt"
                );
                assert_eq!(restored.index, 1);
            };
            // Check the saved bytes directly, before the load-time sanitizer.
            let written: Value =
                serde_json::from_str(&std::fs::read_to_string(session_file(directory.path())).unwrap())
                    .unwrap();
            let saved_pane = TabSnapshot::from_json(&written["tabs"][0]["panes"][0]).unwrap();
            check(SavedSession {
                tabs: vec![SavedTab {
                    panes: vec![saved_pane],
                    active_pane: 0,
                }],
                active_tab: 0,
            });
            check(SavedSession::load(directory.path()).unwrap());
            assert_eq!(session.tabs[0].panes[0], pane, "the live tab keeps its source");

            // Files produced by earlier versions are sanitized on read too.
            let earlier = json!({"tabs": [{"panes": [raw], "activePane": 0}], "activeTab": 0});
            std::fs::write(session_file(directory.path()), earlier.to_string()).unwrap();
            check(SavedSession::load(directory.path()).unwrap());
        }
    }
}
