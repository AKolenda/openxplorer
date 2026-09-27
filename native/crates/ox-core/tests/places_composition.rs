// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar regression cases from desktop/tests/test_v07.py and environment().

use std::path::PathBuf;

use ox_core::places::{
    compose_quick_access, merge_network_locations, network_key, NetworkKind, NetworkMount, Place, StableMount,
};
use ox_core::settings::{Bookmark, SettingsData};

fn bookmark(uri: &str, label: &str) -> Bookmark {
    Bookmark {
        uri: uri.into(),
        label: label.into(),
    }
}

fn place(uri: &str, label: &str) -> Place {
    Place {
        uri: uri.into(),
        label: label.into(),
        icon: Some("documents"),
        color: Some("#4a94d1"),
        pinned: true,
        is_shared: false,
    }
}

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
    assert_eq!(
        rows.iter().map(|row| row.label.as_str()).collect::<Vec<_>>(),
        ["Work", "Documents", "Other"]
    );
    assert!(rows[0].is_shared);
    assert_eq!(rows[1].icon, Some("documents"));
}

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

#[test]
fn saved_labels_win_and_connected_state_merges() {
    let saved = [(bookmark("smb://nas/Work", "My Work"), false)];
    let mounts = [NetworkMount {
        uri: "smb://NAS:445/work/".into(),
        label: "work".into(),
        mounted: true,
    }];
    let rows = merge_network_locations(&saved, &mounts, &[], &[]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uri, "smb://nas/Work");
    assert_eq!(rows[0].label, "My Work");
    assert!(rows[0].saved && rows[0].connected);
}

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

#[test]
fn malformed_and_non_network_contributors_are_ignored() {
    let saved = [
        (bookmark("https://example.invalid", "Bad"), false),
        (bookmark("smb://user:secret@nas/share", "Bad"), true),
    ];
    let mounts = [
        NetworkMount {
            uri: "smb://nas/work".into(),
            label: String::new(),
            mounted: false,
        },
        NetworkMount {
            uri: "file:///mnt/disk".into(),
            label: String::new(),
            mounted: true,
        },
    ];
    let stable = [StableMount {
        path: "/mnt/disk".into(),
        label: String::new(),
        filesystem: "ext4".into(),
    }];
    assert!(merge_network_locations(&saved, &mounts, &stable, &[]).is_empty());
}
