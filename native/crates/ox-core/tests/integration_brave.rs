// SPDX-License-Identifier: AGPL-3.0-only
//! Brave's download folder in disposable profiles.
//!
//! Ports `BraveTests` of `desktop/tests/test_v07.py`. The profiles, backups
//! and process tables are real files in a temporary folder; whether Brave
//! runs is answered by the test, as the Python fixture's `is_running`
//! double did. This file holds the shared fixture.
//!
//! | Case file | What it covers |
//! |---|---|
//! | `status` | Detected profiles, sandboxed installs, running Brave |
//! | `sync` | Pointing profiles at the download folder |
//! | `restore` | Putting the previous folders back |

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ox_core::integration::{
    BraveActivity, BraveError, BraveIntegration, BravePaths, Confirmation, Sandbox, SyncOutcome,
};
use serde_json::{json, Value};
use tempfile::TempDir;

/// The stable channel's default profile.
const PROFILE_ID: &str = "Brave-Browser:Default";

/// A zoom level as Chromium writes it: a 17-digit double that a float
/// parser which is not correctly rounded reads as a neighbouring value.
const ZOOM_LEVEL: &str = "39.430133835633676";

/// Preferences holding [`ZOOM_LEVEL`], written as Brave writes them.
const PREFERENCES_WITH_ZOOM_LEVEL: &str = r#"{"download":{"default_directory":"/old/downloads"},"partition":{"default_zoom_level":{"x":39.430133835633676}},"profile":{"name":"Test person"}}"#;

/// Whether Brave runs, switchable by the test.
#[derive(Debug, Clone, Default)]
struct BraveSwitch(Arc<AtomicBool>);

impl BraveSwitch {
    fn set_running(&self, is_running: bool) {
        self.0.store(is_running, Ordering::SeqCst);
    }
}

impl BraveActivity for BraveSwitch {
    fn is_running(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The Python fixture: a home folder with one stable Brave profile named
/// "Test person" and an empty Downloads folder.
struct Fixture {
    root: TempDir,
    preferences: PathBuf,
    destination: PathBuf,
    brave_switch: BraveSwitch,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary folder");
        let home = root.path().join("home");
        let preferences = home.join(".config/BraveSoftware/Brave-Browser/Default/Preferences");
        fs::create_dir_all(preferences.parent().expect("profile folder")).expect("profile folder");
        write_json(&preferences, &original_preferences());
        let destination = home.join("Downloads");
        fs::create_dir(&destination).expect("Downloads");
        Self {
            root,
            preferences,
            destination,
            brave_switch: BraveSwitch::default(),
        }
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn paths(&self) -> BravePaths {
        BravePaths {
            settings: self.root.path().join("winspace"),
            home: self.home(),
            config_home: self.home().join(".config"),
        }
    }

    fn brave(&self) -> BraveIntegration<BraveSwitch> {
        self.brave_with(self.brave_switch.clone())
    }

    fn brave_with<A: BraveActivity>(&self, activity: A) -> BraveIntegration<A> {
        BraveIntegration::with_activity(&self.paths(), Sandbox::Host, activity)
    }

    fn sync(&self, confirmation: Confirmation) -> Result<SyncOutcome, BraveError> {
        self.brave()
            .sync(&[PROFILE_ID.to_owned()], &self.destination_text(), confirmation)
    }

    fn sync_to(&self, destination: &str) -> Result<SyncOutcome, BraveError> {
        self.brave()
            .sync(&[PROFILE_ID.to_owned()], destination, Confirmation::Confirmed)
    }

    fn destination_text(&self) -> String {
        self.destination
            .to_str()
            .expect("a UTF-8 temporary path")
            .to_owned()
    }

    fn preferences_json(&self) -> Value {
        read_json(&self.preferences)
    }

    /// The files in the backup folder whose names end in `suffix`.
    fn backups_ending_in(&self, suffix: &str) -> Vec<PathBuf> {
        let folder = self.brave().backup_folder().to_owned();
        fs::read_dir(folder)
            .expect("backup folder")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect()
    }
}

fn original_preferences() -> Value {
    json!({
        "profile": {"name": "Test person"},
        "download": {"default_directory": "/old/downloads", "prompt_for_download": true},
        "savefile": {"default_directory": "/old/save"},
        "unrelated": {"keep": [1, 2, 3]},
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, value.to_string()).expect("write JSON");
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read JSON")).expect("JSON")
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("stat").mode() & 0o777
}

#[path = "integration_brave/restore.rs"]
mod restore;
#[path = "integration_brave/status.rs"]
mod status;
#[path = "integration_brave/sync.rs"]
mod sync;
