// SPDX-License-Identifier: AGPL-3.0-only
//! Snapshot source mappings (PROP-023): saving, replacing, removing and
//! reading them, and sharing `snapshot-sources.json` with
//! `v2.0.0:desktop/previous_versions.py`.
//!
//! `v2.0.0:desktop/tests` has no backend test of `configure` or `sources`, so the
//! rules are checked here, and the file is checked against the Python
//! app in both directions. Every file is inside a temporary directory.

mod python_support;

use std::fs;
use std::path::PathBuf;

use ox_core::versions::{PreviousVersions, SnapshotLayout, SnapshotSource, VersionsError, MAX_SOURCES};
use python_support::run_python;
use serde_json::{json, Value};
use tempfile::TempDir;

/// Prints the sources the Python app reads from the settings directory
/// `sys.argv[1]`, as JSON.
const PYTHON_PRINTS_SOURCES: &str = r"
import json, sys
from pathlib import Path
from previous_versions import PreviousVersions
print(json.dumps(PreviousVersions(Path(sys.argv[1])).sources()))
";

/// Maps a live folder to a Snapper collection with the Python app.
const PYTHON_CONFIGURES_A_SOURCE: &str = r"
import sys
from pathlib import Path
from previous_versions import PreviousVersions
PreviousVersions(Path(sys.argv[1])).configure('smb://nas/home', 'smb://nas/backup/home', 'snapper')
";

/// A settings directory inside a temporary folder the caller keeps.
struct Settings {
    _temporary: TempDir,
    directory: PathBuf,
}

impl Settings {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("winspace");
        Self {
            _temporary: temporary,
            directory,
        }
    }

    fn versions(&self) -> PreviousVersions {
        PreviousVersions::new(&self.directory)
    }

    /// The sources as the Python app reads them.
    fn read_by_python(&self) -> Value {
        let printed = run_python(PYTHON_PRINTS_SOURCES, &[&self.directory]);
        serde_json::from_str(&printed).expect("Python printed JSON")
    }

    /// Writes `contents` as the sources file.
    fn write_sources_file(&self, contents: &Value) {
        fs::create_dir_all(&self.directory).unwrap();
        fs::write(self.sources_file(), contents.to_string()).unwrap();
    }

    fn sources_file(&self) -> PathBuf {
        self.directory.join("snapshot-sources.json")
    }
}

/// The live folders of `sources`, in order.
fn live_folders(sources: &[SnapshotSource]) -> Vec<&str> {
    sources.iter().map(SnapshotSource::live).collect()
}

/// parity: PROP-023
#[test]
fn mapping_a_mapped_live_folder_again_replaces_its_mapping() {
    let settings = Settings::new();
    let versions = settings.versions();
    versions
        .configure("/srv/data", "/srv/old-history", SnapshotLayout::Direct)
        .unwrap();
    versions
        .configure("/srv/media", "/srv/media-history", SnapshotLayout::Direct)
        .unwrap();

    let saved = versions
        .configure("/srv/data/", "/srv/history", SnapshotLayout::Snapper)
        .unwrap();

    assert_eq!(live_folders(&saved), ["file:///srv/media", "file:///srv/data"]);
    assert_eq!(saved[1].collection(), "file:///srv/history");
    assert_eq!(saved[1].layout(), SnapshotLayout::Snapper);
    assert_eq!(versions.sources(), saved);
}

/// parity: PROP-023
#[test]
fn at_most_sixty_four_live_folders_can_be_mapped() {
    let settings = Settings::new();
    let versions = settings.versions();
    for number in 0..MAX_SOURCES {
        let live = format!("/srv/{number}");
        versions
            .configure(&live, &format!("{live}/.history"), SnapshotLayout::Direct)
            .unwrap();
    }

    let refused = versions.configure("/srv/more", "/srv/more/.history", SnapshotLayout::Direct);
    let replaced = versions.configure("/srv/0", "/srv/0/.older", SnapshotLayout::Direct);

    let refusal = refused.unwrap_err();
    assert_eq!(refusal.to_string(), "At most 64 snapshot sources are supported.");
    assert!(matches!(refusal, VersionsError::TooManySources));
    assert_eq!(replaced.map(|sources| sources.len()).ok(), Some(MAX_SOURCES));
}

/// parity: PROP-023
#[test]
fn a_refused_mapping_leaves_the_saved_sources_unchanged() {
    let settings = Settings::new();
    let versions = settings.versions();
    versions
        .configure("/srv/data", "/srv/history", SnapshotLayout::Direct)
        .unwrap();

    let containing = versions.configure("/srv/data/a", "/srv", SnapshotLayout::Direct);
    let credentials = versions.configure("smb://user:secret@nas/share", "/srv/x", SnapshotLayout::Direct);

    assert!(matches!(
        containing,
        Err(VersionsError::SnapshotFolderContainsLiveFolder)
    ));
    assert!(matches!(credentials, Err(VersionsError::Location(_))));
    assert_eq!(live_folders(&versions.sources()), ["file:///srv/data"]);
}

/// Python's "Remove mapping" also validated the form's snapshot folder;
/// here removing needs only the live folder.
///
/// parity: PROP-023
#[test]
fn removing_a_mapping_keeps_the_others() {
    let settings = Settings::new();
    let versions = settings.versions();
    versions
        .configure("/srv/data", "/srv/history", SnapshotLayout::Direct)
        .unwrap();
    versions
        .configure("/srv/media", "/srv/media-history", SnapshotLayout::Direct)
        .unwrap();

    let saved = versions.remove_source("/srv/data").unwrap();

    assert_eq!(live_folders(&saved), ["file:///srv/media"]);
    assert_eq!(versions.sources(), saved);
    assert_eq!(versions.check_writable("/srv/history/a").ok(), Some(()));
}

/// parity: PROP-023
#[test]
fn saved_sources_are_read_like_the_python_app_reads_them() {
    let settings = Settings::new();
    settings.write_sources_file(&json!([
        {"live": "\\\\NAS\\Home", "snapshots": "smb://nas/Home/.snapshots", "layout": "snapper"},
        {"live": "/srv/data", "snapshots": "/srv/history", "layout": "zfs"},
        {"live": "/srv/data/a", "snapshots": "/srv"},
        {"live": "C:\\Work", "snapshots": "/srv/x"},
        {"snapshots": "/srv/x"},
        "smb://nas/share",
        {"live": "~/Documents", "snapshots": "~/Backups"},
    ]));

    let sources = settings.versions().sources();

    let from_rust = serde_json::to_value(&sources).unwrap();
    let from_python = settings.read_by_python();
    let without_home_folder = from_rust.as_array().map(|sources| sources[..2].to_vec());
    assert_eq!(from_rust, from_python);
    assert_eq!(
        without_home_folder,
        Some(vec![
            json!({"live": "smb://nas/Home", "snapshots": "smb://nas/Home/.snapshots", "layout": "snapper"}),
            json!({"live": "file:///srv/data", "snapshots": "file:///srv/history", "layout": "direct"}),
        ])
    );
    assert_eq!(sources.len(), 3);
}

/// parity: PROP-023
#[test]
fn an_unreadable_sources_file_holds_no_sources() {
    let settings = Settings::new();
    for contents in [json!({"live": "/srv"}), json!("[]"), json!(null)] {
        settings.write_sources_file(&contents);

        assert!(settings.versions().sources().is_empty(), "{contents}");
        assert_eq!(settings.read_by_python(), json!([]), "{contents}");
    }
}

/// parity: PROP-023, PROP-024
#[test]
fn sources_saved_by_either_app_are_used_by_the_other() {
    let settings = Settings::new();
    run_python(PYTHON_CONFIGURES_A_SOURCE, &[&settings.directory]);
    let versions = settings.versions();

    let read = versions.sources();
    let saved = versions
        .configure("/srv/data", "/srv/history", SnapshotLayout::Direct)
        .unwrap();

    assert_eq!(live_folders(&read), ["smb://nas/home"]);
    assert_eq!(read[0].layout(), SnapshotLayout::Snapper);
    assert_eq!(settings.read_by_python(), serde_json::to_value(&saved).unwrap());
    assert!(matches!(
        versions.check_writable("smb://nas/backup/home/12/snapshot"),
        Err(VersionsError::ReadOnly)
    ));
}
