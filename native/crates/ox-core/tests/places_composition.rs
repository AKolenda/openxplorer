// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar regression cases: the `NetworkTests` of `v2.0.0:desktop/tests/test_v07.py`,
//! the Quick access rules of `environment` in `v2.0.0:desktop/winspace.py`, and
//! `read_user_dirs` in `v2.0.0:desktop/folder_locations.py`, which runs on the same
//! files as the Rust parser. Every file is inside a temporary directory.

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

/// A pin or share as the settings store it.
fn bookmark(uri: &str, label: &str) -> Bookmark {
    Bookmark {
        uri: uri.into(),
        label: label.into(),
    }
}

/// A saved share that no current mount serves.
fn saved_share(uri: &str, label: &str) -> SavedShare {
    SavedShare {
        bookmark: bookmark(uri, label),
        is_connected: false,
    }
}

/// An active GIO network mount.
fn active_mount(uri: &str, label: &str) -> NetworkMount {
    NetworkMount {
        uri: uri.into(),
        label: label.into(),
        is_mounted: true,
    }
}

/// The Quick access row of the standard folder `folder`, found at `uri`.
fn known_folder_row(folder: KnownFolder, uri: &str) -> Place {
    Place {
        uri: uri.into(),
        label: folder.label().into(),
        known_folder: Some(folder),
        is_shared: false,
    }
}

/// parity: SIDE-005, SIDE-009
#[test]
fn quick_access_hides_builtins_preserves_labels_and_keeps_unranked_order() {
    let known = [
        known_folder_row(KnownFolder::Desktop, "file:///home/demo/Desktop"),
        known_folder_row(KnownFolder::Documents, "file:///home/demo/Documents"),
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
    assert_eq!(rows[0].known_folder, None, "a pin is not a standard folder");
    assert_eq!(rows[1].known_folder, Some(KnownFolder::Documents));
    assert_eq!(rows[1].glyph(), Some("documents"));
}

/// A standard folder and the glyph the Python app draws for it.
struct KnownFolderCase {
    label: &'static str,
    glyph: &'static str,
    color: &'static str,
}

/// The standard folders in sidebar order.
const KNOWN_FOLDER_CASES: [KnownFolderCase; 6] = [
    KnownFolderCase {
        label: "Desktop",
        glyph: "desktop",
        color: "#3b8ec7",
    },
    KnownFolderCase {
        label: "Downloads",
        glyph: "downloads",
        color: "#138266",
    },
    KnownFolderCase {
        label: "Documents",
        glyph: "documents",
        color: "#4a94d1",
    },
    KnownFolderCase {
        label: "Pictures",
        glyph: "pictures",
        color: "#9a79cb",
    },
    KnownFolderCase {
        label: "Music",
        glyph: "music",
        color: "#c66b9c",
    },
    KnownFolderCase {
        label: "Videos",
        glyph: "videos",
        color: "#b48540",
    },
];

/// parity: SIDE-005, SIDE-006, LOOK-015
#[test]
fn known_folders_use_the_standard_glyphs_and_colours() {
    let folders = FolderLocations::from_environment()
        .read_paths()
        .quick_access_places();

    assert_eq!(folders.len(), KNOWN_FOLDER_CASES.len());
    for (folder, case) in folders.iter().zip(KNOWN_FOLDER_CASES) {
        assert_eq!(folder.label, case.label);
        assert_eq!(folder.glyph(), Some(case.glyph), "{}", case.label);
        assert_eq!(folder.glyph_color(), Some(case.color), "{}", case.label);
        assert!(folder.known_folder.is_some(), "{}", case.label);
        assert!(folder.uri.starts_with("file:///"), "{}", folder.uri);
    }
}

/// parity: NET-006, LOOK-016
#[test]
fn mount_badges_respect_path_boundaries_and_escaping() {
    // Documents lives on the share mounted at "/mnt/Team (1)"; Downloads
    // lives in a local folder whose name only starts the same way.
    let known = [
        known_folder_row(KnownFolder::Documents, "file:///mnt/Team%20%281%29/Docs"),
        known_folder_row(KnownFolder::Downloads, "file:///mnt/Team%20%281%29-other"),
    ];
    let rows = compose_quick_access(
        &SettingsData::default(),
        &known,
        &[PathBuf::from("/mnt/Team (1)")],
    );
    assert!(rows[0].is_shared);
    assert!(!rows[1].is_shared);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_default_port_and_case`,
/// `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_custom_port_distinct` and
/// `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_host_aliases_not_merged`
///
/// parity: NET-018, SIDE-019
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

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_saved_label_preserved_and_mount_deduplicated`
///
/// parity: NET-018, SIDE-019
#[test]
fn saved_labels_win_and_connected_state_merges() {
    let saved = [saved_share("smb://nas/Work", "My Work")];
    let mounts = [active_mount("smb://NAS:445/work/", "work")];
    let rows = merge_network_locations(&saved, &mounts, &[], &[]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uri, "smb://nas/Work");
    assert_eq!(rows[0].label, "My Work");
    assert!(rows[0].is_saved);
    assert!(rows[0].is_connected);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_connected_unsaved_share`
///
/// parity: NET-018
#[test]
fn a_mounted_share_is_listed_connected_but_not_saved() {
    let mounts = [active_mount("smb://nas/work", "Work")];
    let rows = merge_network_locations(&[], &mounts, &[], &[]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NetworkKind::Share);
    assert!(rows[0].is_connected);
    assert!(!rows[0].is_saved);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_uri_not_label_is_unique_key`
///
/// parity: NET-018
#[test]
fn rows_are_told_apart_by_location_not_label() {
    let saved = [
        saved_share("smb://a/work", "Work"),
        saved_share("smb://b/work", "Work"),
    ];
    assert_eq!(merge_network_locations(&saved, &[], &[], &[]).len(), 2);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_unsaved_host_session_entry`
/// and `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_stable_cifs_mount`
///
/// parity: NET-006, NET-018, SIDE-019
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
    assert!(rows[0].is_connected);
    assert_eq!(
        (rows[1].label.as_str(), rows[1].kind),
        ("nas", NetworkKind::Server)
    );
    assert!(!rows[1].is_connected);
    assert!(rows.iter().all(|row| !row.is_saved));
}

/// An SFTP folder browsed this session and the mount GIO reports for its
/// server make one connected row, so the row the user opened offers
/// Disconnect; a saved folder on the server keeps its own row.
///
/// parity: NET-030
#[test]
fn a_browsed_sftp_folder_joins_its_servers_mount() {
    let mounts = [active_mount("sftp://anna@build/", "build")];
    let visited = [bookmark("sftp://anna@build/home/anna", "")];
    let rows = merge_network_locations(&[], &mounts, &[], &visited);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uri, "sftp://anna@build/");
    assert!(rows[0].is_connected);
    assert!(!rows[0].is_saved);

    let saved = [saved_share("sftp://anna@build/srv/data", "Data")];
    let rows = merge_network_locations(&saved, &mounts, &[], &visited);
    assert_eq!(rows.len(), 1, "the mount and the visit join the saved row");
    assert_eq!(
        (rows[0].uri.as_str(), rows[0].label.as_str()),
        ("sftp://anna@build/srv/data", "Data")
    );
    assert!(rows[0].is_saved && rows[0].is_connected);

    let other_account = [bookmark("sftp://build/home/anna", "")];
    let rows = merge_network_locations(&[], &mounts, &[], &other_account);
    assert_eq!(rows.len(), 2, "another account is another row");
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_ignore_local_and_unmounted`
/// and `v2.0.0:desktop/tests/test_v07.py::NetworkTests::test_invalid_saved_ignored`
///
/// parity: NET-018, SAFE-010
#[test]
fn malformed_and_non_network_contributors_are_ignored() {
    let saved = [
        saved_share("https://example.invalid", "Bad"),
        SavedShare {
            is_connected: true,
            ..saved_share("smb://user:secret@nas/share", "Bad")
        },
        saved_share("", ""),
    ];
    let mounts = [
        NetworkMount {
            uri: "smb://nas/work".into(),
            label: String::new(),
            is_mounted: false,
        },
        active_mount("file:///mnt/disk", ""),
    ];
    let stable = [StableMount {
        path: "/mnt/disk".into(),
        label: String::new(),
        filesystem: "ext4".into(),
    }];
    assert!(merge_network_locations(&saved, &mounts, &stable, &[]).is_empty());
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

/// Prints `FolderLocations.paths()` from `v2.0.0:desktop/folder_locations.py` for
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
///
/// parity: SIDE-006
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
