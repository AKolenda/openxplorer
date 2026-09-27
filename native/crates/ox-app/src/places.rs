// SPDX-License-Identifier: AGPL-3.0-only
//! What the sidebar and the landing pages list.
//!
//! Ports the Quick access and network parts of `environment()` in
//! `desktop/winspace.py` and the sections of `renderSidebar` in
//! `desktop/ui/app.js`:
//!
//! - Quick access: the known folders and pins, with `is_shared` set for
//!   locations on SMB or on a kernel CIFS/SMB3 mount.
//! - Drives: every volume row except mounted SMB shares, which belong under
//!   Network (`!m.uri?.startsWith('smb:')` in app.js).
//! - Network: saved shares (connected when a mount equals or contains
//!   them), GIO SMB mounts, kernel CIFS/SMB3 mounts and the servers visited
//!   this session, merged by ox-core's `merge_network_locations`.
//!
//! Composition is a pure function of one snapshot, so it is tested without
//! a volume monitor.

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;
use ox_core::location::split_location;
use ox_core::places::{
    compose_quick_access, merge_network_locations, NetworkLocation, NetworkMount, Place, StableMount,
};
use ox_core::settings::{Bookmark, SettingsData};

use crate::volumes::VolumeRow;

/// Everything [`compose`] reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlaceSources<'a> {
    /// Pins, saved shares and the Quick access order.
    pub settings: &'a SettingsData,
    /// The known folders (Desktop, Downloads, ...).
    pub known_folders: &'a [Place],
    /// Rows from the volume monitor.
    pub volumes: &'a [VolumeRow],
    /// Kernel CIFS/SMB3 mounts. ox-core does not read the mount table yet
    /// (`read_mounts` in `desktop/mount_support.py`), so the window passes
    /// none; the composition already handles them.
    pub stable_mounts: &'a [StableMount],
    /// SMB servers and shares browsed this session.
    pub visited_network: &'a [Bookmark],
}

/// A saved network share and whether it is mounted now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedShare {
    /// The share as saved in settings.
    pub bookmark: Bookmark,
    /// A current mount equals or contains the share.
    pub connected: bool,
}

/// The composed sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Places {
    /// Known folders and pins, in their saved order.
    pub quick_access: Vec<Place>,
    /// Drives and devices, without mounted SMB shares.
    pub drives: Vec<VolumeRow>,
    /// Saved shares with their connection state (This PC's Network
    /// locations section).
    pub saved_shares: Vec<SavedShare>,
    /// Every network location, merged (the sidebar and the Network page).
    pub network: Vec<NetworkLocation>,
}

/// Composes the sections from one snapshot.
pub(crate) fn compose(sources: PlaceSources<'_>) -> Places {
    let mount_points: Vec<PathBuf> = sources
        .stable_mounts
        .iter()
        .map(|mount| mount.path.clone())
        .collect();
    let quick_access = compose_quick_access(sources.settings, sources.known_folders, &mount_points);
    let drives = sources
        .volumes
        .iter()
        .filter(|row| !row.is_network())
        .cloned()
        .collect();
    let saved_shares: Vec<SavedShare> = sources
        .settings
        .shares
        .iter()
        .map(|share| saved_share(share, sources.volumes))
        .collect();
    let network = merge_network_locations(
        &saved_with_state(&saved_shares),
        &network_mounts(sources.volumes),
        sources.stable_mounts,
        sources.visited_network,
    );
    Places {
        quick_access,
        drives,
        saved_shares,
        network,
    }
}

fn saved_share(share: &Bookmark, volumes: &[VolumeRow]) -> SavedShare {
    SavedShare {
        bookmark: share.clone(),
        connected: is_share_connected(&share.uri, volumes),
    }
}

fn saved_with_state(shares: &[SavedShare]) -> Vec<(Bookmark, bool)> {
    shares
        .iter()
        .map(|share| (share.bookmark.clone(), share.connected))
        .collect()
}

/// The mounted SMB rows, in the form `merge_network_locations` reads.
fn network_mounts(volumes: &[VolumeRow]) -> Vec<NetworkMount> {
    volumes
        .iter()
        .filter(|row| row.is_network())
        .filter_map(|row| {
            let uri = row.uri()?.to_owned();
            Some(NetworkMount {
                uri,
                label: row.label.clone(),
                mounted: true,
            })
        })
        .collect()
}

/// True when a mounted row's root is the share or contains it, compared
/// with GIO's own `equal` and `has_prefix`, as winspace.py does.
pub(crate) fn is_share_connected(share_uri: &str, volumes: &[VolumeRow]) -> bool {
    let share = gio::File::for_uri(share_uri);
    volumes
        .iter()
        .filter_map(VolumeRow::uri)
        .map(gio::File::for_uri)
        .any(|root| share.equal(&root) || share.has_prefix(&root))
}

/// The Network row a browsed SMB location adds for the session: the server
/// for `smb://nas/`, else the share (`smb://nas/media` for
/// `smb://nas/media/2024`). `None` for anything but SMB. The label is left
/// empty so the merge names it after the share or server.
pub(crate) fn visited_root(uri: &str) -> Option<Bookmark> {
    let parts = split_location(uri).ok()?;
    if parts.scheme != "smb" {
        return None;
    }
    let share = parts.path.split('/').find(|segment| !segment.is_empty());
    let root = match share {
        Some(share) => format!("smb://{}/{share}", parts.netloc),
        None => format!("smb://{}/", parts.netloc),
    };
    Some(Bookmark {
        uri: root,
        label: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use ox_core::places::NetworkKind;

    use super::*;
    use crate::volumes::{VolumeKind, VolumeState};

    fn mounted(label: &str, uri: &str) -> VolumeRow {
        VolumeRow {
            label: label.into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mounted {
                uri: uri.into(),
                can_unmount: true,
            },
        }
    }

    fn bookmark(uri: &str, label: &str) -> Bookmark {
        Bookmark {
            uri: uri.into(),
            label: label.into(),
        }
    }

    fn compose_with(settings: &SettingsData, volumes: &[VolumeRow], stable: &[StableMount]) -> Places {
        compose(PlaceSources {
            settings,
            known_folders: &[],
            volumes,
            stable_mounts: stable,
            visited_network: &[],
        })
    }

    #[test]
    fn an_smb_mount_is_listed_under_network_and_not_among_the_drives() {
        let volumes = [
            mounted("media on nas", "smb://nas/media"),
            mounted("Backup", "file:///media/u/Backup"),
        ];
        let places = compose_with(&SettingsData::default(), &volumes, &[]);
        let drives: Vec<&str> = places.drives.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(drives, ["Backup"]);
        assert_eq!(places.network.len(), 1);
        assert_eq!(places.network[0].uri, "smb://nas/media");
        assert!(places.network[0].connected);
        assert!(!places.network[0].saved);
    }

    #[test]
    fn a_saved_share_that_is_mounted_appears_once_and_connected() {
        let settings = SettingsData {
            shares: vec![bookmark("smb://nas/media", "Media")],
            ..SettingsData::default()
        };
        let places = compose_with(&settings, &[mounted("media on nas", "smb://nas/media")], &[]);
        assert_eq!(places.network.len(), 1);
        let row = &places.network[0];
        assert_eq!(
            (row.label.as_str(), row.saved, row.connected),
            ("Media", true, true)
        );
        assert!(places.saved_shares[0].connected);
    }

    #[test]
    fn an_unmounted_saved_share_is_not_connected() {
        let settings = SettingsData {
            shares: vec![bookmark("smb://nas/media", "Media")],
            ..SettingsData::default()
        };
        let places = compose_with(&settings, &[], &[]);
        assert!(!places.network[0].connected);
        assert!(places.network[0].saved);
    }

    #[test]
    fn a_pin_inside_a_cifs_mount_is_marked_shared() {
        let settings = SettingsData {
            pins: vec![bookmark("file:///mnt/nas/work", "Work")],
            ..SettingsData::default()
        };
        let stable = [StableMount {
            path: PathBuf::from("/mnt/nas"),
            label: String::new(),
            filesystem: "cifs".into(),
        }];
        let places = compose_with(&settings, &[], &stable);
        assert!(places.quick_access[0].is_shared);
        assert_eq!(places.network[0].kind, NetworkKind::Mount);
    }

    #[test]
    fn visited_servers_are_listed_after_saved_shares() {
        let settings = SettingsData {
            shares: vec![bookmark("smb://nas/media", "Media")],
            ..SettingsData::default()
        };
        let visited = [visited_root("smb://studio/").expect("SMB server")];
        let places = compose(PlaceSources {
            settings: &settings,
            known_folders: &[],
            volumes: &[],
            stable_mounts: &[],
            visited_network: &visited,
        });
        let rows: Vec<(&str, NetworkKind)> = places
            .network
            .iter()
            .map(|row| (row.uri.as_str(), row.kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("smb://nas/media", NetworkKind::Share),
                ("smb://studio/", NetworkKind::Server)
            ]
        );
    }

    #[test]
    fn visited_roots_are_the_server_or_the_share() {
        let root = |uri: &str| visited_root(uri).map(|bookmark| bookmark.uri);
        assert_eq!(root("smb://nas/").as_deref(), Some("smb://nas/"));
        assert_eq!(
            root("smb://nas/media/2024/June").as_deref(),
            Some("smb://nas/media")
        );
        assert_eq!(root("file:///srv"), None);
    }
}
