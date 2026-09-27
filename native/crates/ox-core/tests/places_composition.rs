// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar regression cases from `desktop/tests/test_v07.py`, `environment()`
//! in `desktop/winspace.py`, and `read_user_dirs` in
//! `desktop/folder_locations.py`, which runs on the same files as the Rust
//! parser. Every file is inside a temporary directory.

mod python_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ox_core::places::{
    compose_quick_access, merge_network_locations, network_key, FolderLocations, KnownFolder, NetworkKind,
    NetworkMount, Place, SavedShare, StableMount,
};
use ox_core::settings::{Bookmark, SettingsData};
use python_support::run_python;
use serde_json::Value;

fn bookmark(uri: &str, label: &str) -> Bookmark {
    Bookmark {
        uri: uri.into(),
        label: label.into(),
    }
}

/// A saved share that is not connected.
fn saved(uri: &str, label: &str) -> SavedShare {
    SavedShare {
        bookmark: bookmark(uri, label),
        connected: false,
    }
}

/// A known-folder row of the Documents folder.
fn place(uri: &str, label: &str) -> Place {
    Place {
        uri: uri.into(),
        label: label.into(),
        known_folder: Some(KnownFolder::Documents),
        is_shared: false,
    }
}

/// A mounted GIO network mount.
fn mounted(uri: &str, label: &str) -> NetworkMount {
    NetworkMount {
        uri: uri.into(),
        label: label.into(),
        mounted: true,
    }
}

/// parity: SIDE-005, SIDE-009
#[test]
fn quick_access_hides_builtins_preserves_labels_and_keeps_unranked_order() {
    let known = [
        place("file:///home/demo/Desktop", "Desktop"),
        place("file:///home/demo/Documents", "Documents"),
    ];
    let settings = SettingsData {
        hidden_quick: vec![known[0].uri.clone()],
        pins: vec![
            bookmark(&known[1].uri, "Custom"),
            bookmark("smb://nas/work", "Work"),
            bookmark("file:///tmp/Other", "Other"),
        ],
        quick_order: vec!["smb://nas/work".into(), "file:///missing".into()],
        ..SettingsData::default()
    };
    let rows = compose_quick_access(&settings, &known, &[]);
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, ["Work", "Documents", "Other"]);
    assert!(rows[0].is_shared);
    assert_eq!(rows[1].known_folder, Some(KnownFolder::Documents));
}

/// parity: NET-006
#[test]
fn mount_badges_respect_path_boundaries_and_escaping() {
    let known = [
        place("file:///mnt/Team%20%281%29/Docs", "Shared"),
        place("file:///mnt/Team%20%281%29-other", "Local"),
    ];
    let rows = compose_quick_access(
        &SettingsData::default(),
        &known,
        &[PathBuf::from("/mnt/Team (1)")],
    );
    assert!(rows[0].is_shared);
    assert!(!rows[1].is_shared);
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_default_port_and_case`,
/// `test_custom_port_distinct` and `test_host_aliases_not_merged`.
/// parity: NET-018
#[test]
fn network_identity_preserves_custom_ports_and_host_aliases() {
    assert_eq!(
        network_key("smb://NAS:445/Share/"),
        network_key("smb://nas/share")
    );
    assert_eq!(network_key("smb://nas/STRAẞE"), network_key("smb://nas/strasse"));
    assert_ne!(
        network_key("smb://nas:1445/share"),
        network_key("smb://nas/share")
    );
    assert_ne!(
        network_key("smb://nas/share"),
        network_key("smb://10.0.0.1/share")
    );
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_saved_label_preserved_and_mount_deduplicated`
/// parity: NET-018
#[test]
fn saved_labels_win_and_connected_state_merges() {
    let shares = [saved("smb://nas/Work", "My Work")];
    let mounts = [mounted("smb://NAS:445/work/", "work")];
    let rows = merge_network_locations(&shares, &mounts, &[], &[]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uri, "smb://nas/Work");
    assert_eq!(rows[0].label, "My Work");
    assert!(rows[0].saved && rows[0].connected);
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_connected_unsaved_share`
/// parity: NET-018
#[test]
fn a_mounted_share_is_connected_but_not_saved() {
    let rows = merge_network_locations(&[], &[mounted("smb://nas/work", "Work")], &[], &[]);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].connected);
    assert!(!rows[0].saved);
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_uri_not_label_is_unique_key`
/// parity: NET-018
#[test]
fn shares_with_the_same_label_on_different_hosts_stay_apart() {
    let shares = [saved("smb://a/work", "Work"), saved("smb://b/work", "Work")];
    assert_eq!(merge_network_locations(&shares, &[], &[], &[]).len(), 2);
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_unsaved_host_session_entry`
/// and `test_stable_cifs_mount`.
/// parity: NET-006, NET-018
#[test]
fn visited_servers_and_stable_mounts_need_no_saved_bookmark() {
    let stable = [StableMount {
        path: "/mnt/Work".into(),
        label: String::new(),
        filesystem: "cifs".into(),
    }];
    let rows = merge_network_locations(&[], &[], &stable, &[bookmark("smb://nas/", "")]);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].uri.as_str(), rows[0].label.as_str(), rows[0].kind),
        ("file:///mnt/Work", "Work", NetworkKind::Mount)
    );
    assert!(rows[0].connected);
    assert_eq!(
        (rows[1].label.as_str(), rows[1].kind),
        ("nas", NetworkKind::Server)
    );
    assert!(!rows[1].connected);
    assert!(rows.iter().all(|row| !row.saved));
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_ignore_local_and_unmounted`
/// and `test_invalid_saved_ignored`.
/// parity: NET-018
#[test]
fn malformed_and_non_network_contributors_are_ignored() {
    let shares = [
        saved("https://example.invalid", "Bad"),
        SavedShare {
            connected: true,
            ..saved("smb://user:secret@nas/share", "Bad")
        },
        saved("", ""),
    ];
    let mounts = [
        NetworkMount {
            mounted: false,
            ..mounted("smb://nas/work", "")
        },
        mounted("file:///mnt/disk", ""),
    ];
    let stable = [StableMount {
        path: "/mnt/disk".into(),
        label: String::new(),
        filesystem: "ext4".into(),
    }];
    assert!(merge_network_locations(&shares, &mounts, &stable, &[]).is_empty());
}

/// One `user-dirs.dirs` fixture.
struct UserDirsCase {
    /// The case directory, which stands for `$XDG_CONFIG_HOME`.
    name: &'static str,
    /// The file contents; `None` means the file does not exist.
    contents: Option<&'static str>,
}

/// Valid, unusual and hostile `user-dirs.dirs` files that both parsers read.
const USER_DIRS_CASES: [UserDirsCase; 10] = [
    UserDirsCase {
        name: "missing",
        contents: None,
    },
    UserDirsCase {
        name: "empty",
        contents: Some(""),
    },
    UserDirsCase {
        name: "home_variables",
        contents: Some(
            "XDG_DESKTOP_DIR=\"$HOME/Desk\"\nXDG_DOWNLOAD_DIR=\"${HOME}/Incoming\"\n\
             XDG_DOCUMENTS_DIR=\"$HOME\"\nXDG_MUSIC_DIR=\"$HOME/\"\n",
        ),
    },
    UserDirsCase {
        name: "escapes",
        contents: Some(
            "XDG_DOCUMENTS_DIR=\"$HOME/My \\\"Docs\\\"\"\nXDG_PICTURES_DIR=\"/data/back\\\\slash\"\n\
             XDG_MUSIC_DIR=\"/data/\\$USER/\\`x\\`\"\nXDG_VIDEOS_DIR=\"/data/keep\\q\"\n",
        ),
    },
    UserDirsCase {
        name: "shell_syntax",
        contents: Some(
            "XDG_DESKTOP_DIR=\"/data/$USER/Desktop\"\nXDG_DOWNLOAD_DIR=\"/data/`id`\"\n\
             XDG_DOCUMENTS_DIR=\"$HOMEDIR/Docs\"\n",
        ),
    },
    UserDirsCase {
        name: "normalisation",
        contents: Some(
            "XDG_PICTURES_DIR=\"/data//Pictures/\"\nXDG_VIDEOS_DIR=\"$HOME/./Videos\"\n\
             XDG_MUSIC_DIR=\"/data/a/../Music\"\nXDG_TEMPLATES_DIR=\"//server/Templates\"\n\
             XDG_PUBLICSHARE_DIR=\"///Public\"\n",
        ),
    },
    UserDirsCase {
        name: "relative_and_control",
        contents: Some(
            "XDG_DESKTOP_DIR=\"Desktop\"\nXDG_DOWNLOAD_DIR=\"~/Downloads\"\n\
             XDG_DOCUMENTS_DIR=\"/tab\there\"\nXDG_PICTURES_DIR=\"/del\u{7f}\"\n\
             XDG_MUSIC_DIR=\"/unterminated\n",
        ),
    },
    UserDirsCase {
        name: "comments_and_spacing",
        contents: Some(
            "  XDG_DESKTOP_DIR = \"/data/Desktop\"   # moved\n# XDG_DOWNLOAD_DIR=\"/nope\"\n\
             \u{a0}XDG_MUSIC_DIR=\"/data/Music\"\nXDG_VIDEOS_DIR=\"/data/Videos\" trailing\n\
             xdg_pictures_dir=\"/lower\"\nXDG_UNKNOWN_DIR=\"/x\"\n\
             XDG_TEMPLATES_DIR=\"/data/\\\"#not a comment\"\n",
        ),
    },
    UserDirsCase {
        name: "duplicates",
        contents: Some(
            "XDG_DESKTOP_DIR=\"/first\"\nXDG_DESKTOP_DIR=\"/second\"\n\
             XDG_DOWNLOAD_DIR=\"/valid\"\nXDG_DOWNLOAD_DIR=\"/in$valid\"\n",
        ),
    },
    UserDirsCase {
        name: "line_boundaries",
        contents: Some(
            "XDG_DESKTOP_DIR=\"/a\u{1c}b\"\r\nXDG_MUSIC_DIR=\"/data/Música\"\r\
             XDG_VIDEOS_DIR=\"/data/V\u{2028}x\"\n\u{b}XDG_PICTURES_DIR=\"/data/P\"\u{85}\
             XDG_PUBLICSHARE_DIR=\"/data/Public\"",
        ),
    },
];

/// Prints `FolderLocations.paths()` from `desktop/folder_locations.py` for
/// every case directory under `sys.argv[1]`, with `sys.argv[2]` as the home
/// folder, as `{case: {XDG key: path}}`.
const PYTHON_PRINTS_FOLDER_PATHS: &str = r"
import json, sys
from pathlib import Path
from folder_locations import FolderLocations
root, home = Path(sys.argv[1]), Path(sys.argv[2])
cases = {config.name: FolderLocations(root / 'unused', home=home, config=config).paths()
         for config in sorted(root.iterdir()) if config.is_dir()}
print(json.dumps(cases))
";

/// The Python app's standard folders for every case directory under
/// `root`.
fn python_folder_paths(root: &Path, home: &Path) -> Value {
    let printed = run_python(PYTHON_PRINTS_FOLDER_PATHS, &[root, home]);
    serde_json::from_str(&printed).expect("Python printed JSON")
}

/// The same result from [`FolderLocations::read_paths`].
fn rust_folder_paths(root: &Path, home: &Path) -> Value {
    let mut cases = BTreeMap::new();
    for case in &USER_DIRS_CASES {
        let paths = FolderLocations::new(home.to_path_buf(), &root.join(case.name)).read_paths();
        let folders: BTreeMap<&str, String> = KnownFolder::ALL
            .into_iter()
            .map(|folder| (folder.xdg_key(), paths.path(folder).display().to_string()))
            .collect();
        cases.insert(case.name, folders);
    }
    serde_json::to_value(cases).expect("paths serialise to JSON")
}

/// Both applications must resolve the same standard folders, because
/// Quick access matches their URIs against `hiddenQuick` and `quickOrder`.
#[test]
fn user_dirs_resolve_like_the_python_app() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("cases");
    let home = temporary.path().join("home");
    for case in &USER_DIRS_CASES {
        let config = root.join(case.name);
        fs::create_dir_all(&config).unwrap();
        if let Some(contents) = case.contents {
            fs::write(config.join("user-dirs.dirs"), contents).unwrap();
        }
    }
    assert_eq!(rust_folder_paths(&root, &home), python_folder_paths(&root, &home));
}
