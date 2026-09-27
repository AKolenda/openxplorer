// SPDX-License-Identifier: AGPL-3.0-only
//! Drives, volumes and phones for This PC.
//!
//! Ports `desktop/volume_locations.py` and its tests
//! (`desktop/tests/test_volume_locations.py`). The volume monitor's current
//! state is turned into sidebar rows without mounting, probing or listing
//! anything. Mounted roots outside the supported schemes (for example a
//! web location) and shadowed mounts are hidden; a volume that is not
//! mounted yet becomes a click-to-connect row.
//!
//! The classification works on plain [`MountFacts`] and [`VolumeFacts`]
//! so it can be tested without GIO; [`from_monitor`] reads them from a real
//! `gio::VolumeMonitor`.

use gio::prelude::*;
use ox_core::location;

/// Schemes a mount root may use. ox-core's `normalise_location` does not
/// enforce the Python allowlist yet, so this module does.
const SUPPORTED_SCHEMES: [&str; 5] = ["file", "smb", "mtp", "gphoto2", "afc"];

/// What a row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A disk or partition.
    Drive,
    /// A phone, camera or iOS device.
    Device,
}

/// One This PC row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeRow {
    /// Name shown in the sidebar and on This PC.
    pub label: String,
    /// The mounted root; `None` for a volume that still has to be mounted.
    pub uri: Option<String>,
    /// Identifier used to mount an unmounted volume.
    pub id: Option<String>,
    /// Drive or device glyph.
    pub kind: VolumeKind,
    /// The volume is mounted and can be browsed.
    pub mounted: bool,
    /// Offered a Disconnect command.
    pub can_unmount: bool,
}

/// The parts of a `gio::Mount` this module reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountFacts {
    /// Display name supplied by the volume monitor.
    pub name: String,
    /// URI of the mounted filesystem root.
    pub root_uri: String,
    /// Hidden behind a replacement mount.
    pub shadowed: bool,
    /// The mount offers an unmount operation.
    pub can_unmount: bool,
}

/// The parts of a `gio::Volume` this module reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolumeFacts {
    /// Display name supplied by the volume monitor.
    pub name: String,
    /// A mount already exists for this volume.
    pub mounted: bool,
    /// The volume offers a mount operation.
    pub can_mount: bool,
    /// Stable filesystem identifier when available.
    pub uuid: Option<String>,
    /// Native device path when available.
    pub unix_device: Option<String>,
    /// Root offered by a portable device or virtual volume.
    pub activation_uri: Option<String>,
}

impl VolumeFacts {
    /// A stable-enough identifier for a single mount request, as
    /// `volume_id` in volume_locations.py.
    pub fn id(&self) -> String {
        self.uuid
            .clone()
            .or_else(|| self.unix_device.clone())
            .or_else(|| self.activation_uri.clone())
            .unwrap_or_else(|| self.name.clone())
    }
}

/// Canonical URI for a supported root, or `None` for anything else.
fn supported_uri(uri: &str) -> Option<String> {
    let scheme = location::scheme(uri);
    if !SUPPORTED_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    if location::is_device_location(uri) {
        // GIO would rewrite `mtp://[usb:001,010]/`; keep device roots as-is.
        return Some(uri.to_string());
    }
    Some(gio::File::for_uri(uri).uri().to_string())
}

fn kind_for(uri: &str) -> VolumeKind {
    if location::is_device_location(uri) {
        VolumeKind::Device
    } else {
        VolumeKind::Drive
    }
}

/// Mounted rows first, then unmounted volumes, as `locations()` does.
pub fn locations(mounts: &[MountFacts], volumes: &[VolumeFacts]) -> Vec<VolumeRow> {
    let mut rows = Vec::new();
    for mount in mounts.iter().filter(|mount| !mount.shadowed) {
        let Some(uri) = supported_uri(&mount.root_uri) else {
            continue;
        };
        rows.push(VolumeRow {
            label: mount.name.clone(),
            kind: kind_for(&uri),
            uri: Some(uri),
            id: None,
            mounted: true,
            can_unmount: mount.can_unmount,
        });
    }
    for volume in volumes
        .iter()
        .filter(|volume| !volume.mounted && volume.can_mount)
    {
        let activation = volume.activation_uri.as_deref().and_then(supported_uri);
        let kind = activation.as_deref().map_or(VolumeKind::Drive, kind_for);
        rows.push(VolumeRow {
            label: volume.name.clone(),
            uri: None,
            id: Some(volume.id()),
            kind,
            mounted: false,
            can_unmount: false,
        });
    }
    rows
}

/// Reads the facts from a real volume monitor.
pub fn from_monitor(monitor: &gio::VolumeMonitor) -> Vec<VolumeRow> {
    let mounts: Vec<MountFacts> = monitor
        .mounts()
        .iter()
        .map(|mount| MountFacts {
            name: mount.name().to_string(),
            root_uri: mount.root().uri().to_string(),
            shadowed: mount.is_shadowed(),
            can_unmount: mount.can_unmount(),
        })
        .collect();
    let volumes: Vec<VolumeFacts> = monitor
        .volumes()
        .iter()
        .map(|volume| VolumeFacts {
            name: volume.name().to_string(),
            mounted: volume.get_mount().is_some(),
            can_mount: volume.can_mount(),
            uuid: volume.uuid().map(|s| s.to_string()),
            unix_device: volume.identifier("unix-device").map(|s| s.to_string()),
            activation_uri: volume.activation_root().map(|root| root.uri().to_string()),
        })
        .collect();
    locations(&mounts, &volumes)
}

/// Finds the monitor's volume for a row identifier from [`from_monitor`].
pub fn find_volume(monitor: &gio::VolumeMonitor, id: &str) -> Option<gio::Volume> {
    monitor.volumes().into_iter().find(|volume| {
        let facts = VolumeFacts {
            name: volume.name().to_string(),
            uuid: volume.uuid().map(|s| s.to_string()),
            unix_device: volume.identifier("unix-device").map(|s| s.to_string()),
            activation_uri: volume.activation_root().map(|root| root.uri().to_string()),
            ..VolumeFacts::default()
        };
        facts.id() == id
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(name: &str, uri: &str) -> MountFacts {
        MountFacts {
            name: name.into(),
            root_uri: uri.into(),
            shadowed: false,
            can_unmount: true,
        }
    }

    /// Ported from desktop/tests/test_volume_locations.py::test_mounted_mtp_phone_and_afc_device_are_visible
    #[test]
    fn mounted_mtp_phone_and_afc_device_are_visible() {
        let mounts = [
            mount("Pixel 9", "mtp://[usb:001,010]/"),
            mount("iPhone", "afc://00008020-001C/"),
            mount("Disk", "file:///media/example/Disk"),
        ];
        let rows = locations(&mounts, &[]);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        let kinds: Vec<VolumeKind> = rows.iter().map(|row| row.kind).collect();
        assert_eq!(labels, ["Pixel 9", "iPhone", "Disk"]);
        assert_eq!(kinds, [VolumeKind::Device, VolumeKind::Device, VolumeKind::Drive]);
        assert!(rows.iter().all(|row| row.mounted));
    }

    /// Ported from desktop/tests/test_volume_locations.py::test_unmounted_phone_is_click_to_connect_device
    #[test]
    fn unmounted_phone_is_click_to_connect_device() {
        let phone = VolumeFacts {
            name: "Android Phone".into(),
            can_mount: true,
            activation_uri: Some("mtp://[usb:001,011]/".into()),
            ..VolumeFacts::default()
        };
        let rows = locations(&[], std::slice::from_ref(&phone));
        assert_eq!(
            rows,
            [VolumeRow {
                label: "Android Phone".into(),
                uri: None,
                id: Some("mtp://[usb:001,011]/".into()),
                kind: VolumeKind::Device,
                mounted: false,
                can_unmount: false,
            }]
        );
        assert_eq!(phone.id(), "mtp://[usb:001,011]/");
    }

    /// Ported from desktop/tests/test_volume_locations.py::test_unsupported_and_shadowed_mounts_stay_hidden
    #[test]
    fn unsupported_and_shadowed_mounts_stay_hidden() {
        let shadow = MountFacts {
            shadowed: true,
            ..mount("Shadow", "mtp://[usb:001,012]/")
        };
        let unavailable = VolumeFacts {
            name: "Unavailable".into(),
            can_mount: false,
            ..VolumeFacts::default()
        };
        let rows = locations(
            &[mount("Web", "https://example.invalid/files"), shadow],
            &[unavailable],
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn volume_ids_prefer_uuid_then_device_then_activation_root() {
        let mut volume = VolumeFacts {
            name: "USB".into(),
            uuid: Some("1234-ABCD".into()),
            unix_device: Some("/dev/sdb1".into()),
            ..VolumeFacts::default()
        };
        assert_eq!(volume.id(), "1234-ABCD");
        volume.uuid = None;
        assert_eq!(volume.id(), "/dev/sdb1");
        volume.unix_device = None;
        assert_eq!(volume.id(), "USB");
    }
}
