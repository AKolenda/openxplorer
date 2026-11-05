// SPDX-License-Identifier: AGPL-3.0-only
//! What the sidebar and the landing pages list.
//!
//! Ports the Quick access and network parts of `environment()` in
//! `v2.0.0:desktop/winspace.py` and the sections of `renderSidebar` in
//! `v2.0.0:desktop/ui/app.js`:
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
use ox_core::places::{
    compose_quick_access, merge_network_locations, network_key, NetworkLocation, NetworkMount, Place,
    SavedShare, StableMount,
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
    /// Kernel CIFS/SMB3 mounts (`read_mounts` in
    /// `v2.0.0:desktop/mount_support.py`), as the application last read them.
    pub stable_mounts: &'a [StableMount],
    /// SMB servers and shares browsed this session.
    pub visited_network: &'a [Bookmark],
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
        .map(|share| saved_share_among(share, sources.volumes))
        .collect();
    let network = merge_network_locations(
        &saved_shares,
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

/// `share` with its connection state: connected when a current mount
/// among `volumes` equals or contains it.
fn saved_share_among(share: &Bookmark, volumes: &[VolumeRow]) -> SavedShare {
    SavedShare {
        bookmark: share.clone(),
        is_connected: is_share_connected(&share.uri, volumes),
    }
}

/// The mounted SMB rows, in the form `merge_network_locations` reads.
fn network_mounts(volumes: &[VolumeRow]) -> Vec<NetworkMount> {
    volumes
        .iter()
        .filter(|row| row.is_network())
        .filter_map(network_mount)
        .collect()
}

/// The network mount of a mounted row, or `None` for a row that still has
/// to be mounted.
fn network_mount(row: &VolumeRow) -> Option<NetworkMount> {
    let uri = row.uri()?;
    Some(NetworkMount {
        uri: uri.to_owned(),
        label: row.label.clone(),
        is_mounted: true,
    })
}

/// The network mounts Disconnect network drive can end, one per mount, in
/// the volume monitor's order: each mounted network row's root, as GIO
/// spells it, since unmounting ends the whole mount. A saved share inside a
/// mount is not a drive of its own, and a browsed SMB server (`smb://host/`)
/// is not a drive at all. Roots that name the same location once their
/// spelling is normalised (`network_key`) are listed once.
pub(crate) fn network_drive_mounts(volumes: &[VolumeRow]) -> Vec<NetworkMount> {
    let mut keys = Vec::new();
    network_mounts(volumes)
        .into_iter()
        .filter(|mount| !ox_core::location::is_smb_server(&mount.uri))
        .filter(|mount| {
            let key = network_key(&mount.uri).ok();
            let is_new = !keys.contains(&key);
            keys.push(key);
            is_new
        })
        .collect()
}

/// True when a mounted row's root is the share or contains it, compared
/// with GIO's own `equal` and `has_prefix`, as winspace.py does.
fn is_share_connected(share_uri: &str, volumes: &[VolumeRow]) -> bool {
    let share = gio::File::for_uri(share_uri);
    volumes
        .iter()
        .filter_map(VolumeRow::uri)
        .map(gio::File::for_uri)
        .any(|root| share.equal(&root) || share.has_prefix(&root))
}

/// The row of `network` that stands for `uri` itself, not for a folder
/// inside it. Locations are compared as the merge compares them
/// (`network_key`): SMB's default port filled in, the path decoded and
/// case-folded, a trailing slash ignored. `None` when no row matches or
/// `uri` is not a location.
pub(crate) fn network_row<'a>(network: &'a [NetworkLocation], uri: &str) -> Option<&'a NetworkLocation> {
    let wanted = network_key(uri).ok()?;
    network
        .iter()
        .find(|location| network_key(&location.uri).is_ok_and(|key| key == wanted))
}

#[cfg(test)]
mod tests {
    use ox_core::network::{session_network_root, VisitedNetwork};
    use ox_core::places::NetworkKind;

    use super::*;
    use crate::test_support::mounted_volume;
    use crate::volumes::VolumeKind;

    fn bookmark(uri: &str, label: &str) -> Bookmark {
        Bookmark {
            uri: uri.into(),
            label: label.into(),
        }
    }

    fn settings_with_share(uri: &str, label: &str) -> SettingsData {
        SettingsData {
            shares: vec![bookmark(uri, label)],
            ..SettingsData::default()
        }
    }

    /// The Network rows browsing `uris` adds for the session, as the app
    /// records them.
    fn visited(uris: &[&str]) -> Vec<Bookmark> {
        let mut visited = VisitedNetwork::default();
        for uri in uris {
            visited.remember(uri);
        }
        visited.to_bookmarks()
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

    /// parity: NET-018, SIDE-001, SIDE-019
    /// Disconnect network drive's choices are the mounts, each once: two
    /// rows for one share spelled differently are one drive, a browsed
    /// server is none, and a local drive is not a network drive.
    #[test]
    fn network_drives_are_the_mounts_each_once() {
        let volumes = [
            mounted_volume(
                "share on fileserver",
                "smb://fileserver/share/",
                VolumeKind::Drive,
            ),
            mounted_volume(
                "share on FileServer",
                "smb://FileServer:445/share",
                VolumeKind::Drive,
            ),
            mounted_volume("fileserver", "smb://fileserver/", VolumeKind::Drive),
            mounted_volume("Photos", "sftp://studio-nas/photos/", VolumeKind::Drive),
            mounted_volume("USB", "file:///media/user/USB", VolumeKind::Drive),
        ];
        let drives: Vec<(String, String)> = network_drive_mounts(&volumes)
            .into_iter()
            .map(|mount| (mount.label, mount.uri))
            .collect();
        assert_eq!(
            drives,
            [
                (
                    "share on fileserver".to_owned(),
                    "smb://fileserver/share/".to_owned()
                ),
                ("Photos".to_owned(), "sftp://studio-nas/photos/".to_owned()),
            ]
        );
    }

    #[test]
    fn an_smb_mount_is_listed_under_network_and_not_among_the_drives() {
        let volumes = [
            mounted_volume("media on nas", "smb://nas/media", VolumeKind::Drive),
            mounted_volume("Backup", "file:///media/u/Backup", VolumeKind::Drive),
        ];
        let places = compose_with(&SettingsData::default(), &volumes, &[]);
        let drives: Vec<&str> = places.drives.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(drives, ["Backup"]);
        assert_eq!(places.network.len(), 1);
        assert_eq!(places.network[0].uri, "smb://nas/media");
        assert!(places.network[0].is_connected);
        assert!(!places.network[0].is_saved);
    }

    /// parity: NET-018, SIDE-021
    #[test]
    fn a_saved_share_that_is_mounted_appears_once_and_connected() {
        let settings = settings_with_share("smb://nas/media", "Media");
        let share_mount = mounted_volume("media on nas", "smb://nas/media", VolumeKind::Drive);
        let places = compose_with(&settings, &[share_mount], &[]);
        assert_eq!(places.network.len(), 1);
        let row = &places.network[0];
        assert_eq!(
            (row.label.as_str(), row.is_saved, row.is_connected),
            ("Media", true, true)
        );
        assert!(places.saved_shares[0].is_connected);
    }

    /// parity: SIDE-021
    #[test]
    fn a_saved_share_inside_a_mounted_share_is_connected() {
        let settings = settings_with_share("smb://nas/media/2024", "2024");
        let share_mount = mounted_volume("media on nas", "smb://nas/media", VolumeKind::Drive);
        let places = compose_with(&settings, &[share_mount], &[]);
        assert!(places.saved_shares[0].is_connected);
    }

    /// parity: NET-018, SIDE-021
    #[test]
    fn an_unmounted_saved_share_is_not_connected() {
        let settings = settings_with_share("smb://nas/media", "Media");
        let places = compose_with(&settings, &[], &[]);
        assert!(!places.network[0].is_connected);
        assert!(places.network[0].is_saved);
    }

    /// parity: NET-006
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

    /// parity: NET-018
    #[test]
    fn visited_servers_are_listed_after_saved_shares() {
        let settings = settings_with_share("smb://nas/media", "Media");
        let visited = visited(&["smb://studio/"]);
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
    fn a_location_finds_its_own_network_row_however_it_is_spelt() {
        let settings = settings_with_share("smb://nas/media", "Media (M:)");
        let visited = visited(&["smb://studio/"]);
        let places = compose(PlaceSources {
            settings: &settings,
            known_folders: &[],
            volumes: &[],
            stable_mounts: &[],
            visited_network: &visited,
        });
        let label_of = |uri: &str| network_row(&places.network, uri).map(|row| row.label.as_str());
        assert_eq!(label_of("smb://nas/media"), Some("Media (M:)"));
        assert_eq!(
            label_of("smb://NAS:445/Media/"),
            Some("Media (M:)"),
            "the same share"
        );
        assert_eq!(label_of("smb://studio/"), Some("studio"));
        assert_eq!(
            label_of("smb://nas/media/2024"),
            None,
            "a folder inside the share"
        );
        assert_eq!(label_of("smb://nas/"), None, "the server was never listed");
        assert_eq!(label_of("not a location"), None);
    }

    /// parity: NET-018
    #[test]
    fn visited_roots_are_the_server_or_the_share() {
        let root = session_network_root;
        assert_eq!(root("smb://nas/").as_deref(), Some("smb://nas/"));
        assert_eq!(
            root("smb://nas/media/2024/June").as_deref(),
            Some("smb://nas/media")
        );
        assert_eq!(root("file:///srv"), None);
    }
}
