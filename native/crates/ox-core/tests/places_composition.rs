// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar regression cases: the `NetworkTests` of `desktop/tests/test_v07.py`
//! and the Quick access rules of `environment` in `desktop/winspace.py`.

use std::path::PathBuf;

use ox_core::places::{
    compose_quick_access, known_folders, merge_network_locations, network_key, FolderGlyph, NetworkKind,
    NetworkMount, Place, PlaceOrigin, SavedShare, StableMount,
};
use ox_core::settings::{Bookmark, SettingsData};

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

fn active_mount(uri: &str, label: &str) -> NetworkMount {
    NetworkMount {
        uri: uri.into(),
        label: label.into(),
        is_mounted: true,
    }
}

/// The Documents folder's glyph.
const DOCUMENTS_GLYPH: FolderGlyph = FolderGlyph {
    name: "documents",
    color: "#4a94d1",
};

/// A built-in Quick access row at `uri`. Every fixture row carries the
/// Documents glyph, whatever its label, so a built-in row that survives
/// composition is told apart from a pin, whose glyph is `None`.
fn built_in_place(uri: &str, label: &str) -> Place {
    Place {
        uri: uri.into(),
        label: label.into(),
        glyph: Some(DOCUMENTS_GLYPH),
        origin: PlaceOrigin::KnownFolder,
        is_shared: false,
    }
}

/// parity: SIDE-005
#[test]
fn quick_access_hides_builtins_preserves_labels_and_keeps_unranked_order() {
    let known = [
        built_in_place("file:///home/demo/Desktop", "Desktop"),
        built_in_place("file:///home/demo/Documents", "Documents"),
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
    assert_eq!(
        rows.iter().map(|row| row.label.as_str()).collect::<Vec<_>>(),
        ["Work", "Documents", "Other"]
    );
    assert!(rows[0].is_shared);
    assert_eq!(rows[0].origin, PlaceOrigin::Pin);
    assert_eq!(rows[1].glyph, Some(DOCUMENTS_GLYPH));
    assert_eq!(rows[1].origin, PlaceOrigin::KnownFolder);
}

/// A standard folder and the glyph the Python app draws for it.
struct KnownFolderCase {
    label: &'static str,
    glyph: FolderGlyph,
}

/// The standard folders in sidebar order.
const KNOWN_FOLDER_CASES: [KnownFolderCase; 6] = [
    KnownFolderCase {
        label: "Desktop",
        glyph: FolderGlyph {
            name: "desktop",
            color: "#3b8ec7",
        },
    },
    KnownFolderCase {
        label: "Downloads",
        glyph: FolderGlyph {
            name: "downloads",
            color: "#138266",
        },
    },
    KnownFolderCase {
        label: "Documents",
        glyph: DOCUMENTS_GLYPH,
    },
    KnownFolderCase {
        label: "Pictures",
        glyph: FolderGlyph {
            name: "pictures",
            color: "#9a79cb",
        },
    },
    KnownFolderCase {
        label: "Music",
        glyph: FolderGlyph {
            name: "music",
            color: "#c66b9c",
        },
    },
    KnownFolderCase {
        label: "Videos",
        glyph: FolderGlyph {
            name: "videos",
            color: "#b48540",
        },
    },
];

/// parity: SIDE-005, LOOK-015
#[test]
fn known_folders_use_the_standard_glyphs_and_colours() {
    let folders = known_folders();
    assert_eq!(folders.len(), KNOWN_FOLDER_CASES.len());
    for (folder, case) in folders.iter().zip(KNOWN_FOLDER_CASES) {
        assert_eq!(folder.label, case.label);
        assert_eq!(folder.glyph, Some(case.glyph), "{}", case.label);
        assert_eq!(folder.origin, PlaceOrigin::KnownFolder, "{}", case.label);
        assert!(folder.uri.starts_with("file:///"), "{}", folder.uri);
    }
}

/// parity: NET-006, LOOK-016
#[test]
fn mount_badges_respect_path_boundaries_and_escaping() {
    let known = [
        built_in_place("file:///mnt/Team%20%281%29/Docs", "Shared"),
        built_in_place("file:///mnt/Team%20%281%29-other", "Local"),
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
/// `desktop/tests/test_v07.py::NetworkTests::test_custom_port_distinct` and
/// `desktop/tests/test_v07.py::NetworkTests::test_host_aliases_not_merged`
///
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
///
/// parity: NET-018
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

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_connected_unsaved_share`
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

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_uri_not_label_is_unique_key`
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

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_unsaved_host_session_entry`
/// and `desktop/tests/test_v07.py::NetworkTests::test_stable_cifs_mount`
///
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
    assert!(rows[0].is_connected);
    assert_eq!(
        (rows[1].label.as_str(), rows[1].kind),
        ("nas", NetworkKind::Server)
    );
    assert!(!rows[1].is_connected);
    assert!(rows.iter().all(|row| !row.is_saved));
}

/// Ported from `desktop/tests/test_v07.py::NetworkTests::test_ignore_local_and_unmounted`
/// and `desktop/tests/test_v07.py::NetworkTests::test_invalid_saved_ignored`
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
