// SPDX-License-Identifier: AGPL-3.0-only
//! Drives, volumes, phones and connected shares from the volume monitor.
//!
//! Ports `desktop/volume_locations.py` and its tests
//! (`desktop/tests/test_volume_locations.py`). The volume monitor's current
//! state is turned into rows without mounting, probing or listing anything.
//!
//! Every root is canonicalised with [`location::normalise`], as the Python
//! module runs `normalise_location` on each one. That matters twice:
//!
//! - A row's URI must equal the URI a tab stores after navigating there, or
//!   the sidebar cannot highlight it. GIO's own spelling escapes fewer
//!   characters (`Backup%20(2024)` instead of `Backup%20%282024%29`).
//! - Normalisation is the scheme allowlist (local paths, `smb:` and the
//!   device schemes) and rejects addresses carrying a user name. A root it
//!   rejects is hidden, and so is a shadowed mount. A volume that is not
//!   mounted yet becomes a click-to-connect row.
//!
//! The classification works on plain [`MountFacts`] and [`VolumeFacts`]
//! so it can be tested without GIO; [`from_monitor`] reads them from a real
//! `gio::VolumeMonitor`.

use gio::prelude::*;
use ox_core::location;

/// What a row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A disk, partition or network mount.
    Drive,
    /// A phone, camera or iOS device.
    Device,
}

/// One mounted or mountable location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeRow {
    /// Name shown in the sidebar and on This PC.
    pub label: String,
    /// The canonical mounted root; `None` for a volume that still has to be
    /// mounted.
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

impl VolumeRow {
    /// True for a mounted SMB share. It belongs under Network, never among
    /// the drives (`!m.uri?.startsWith('smb:')` in app.js).
    pub fn is_network(&self) -> bool {
        self.uri.as_deref().is_some_and(|uri| uri.starts_with("smb:"))
    }
}

/// The parts of a `gio::Mount` this module reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountFacts {
    /// Display name supplied by the volume monitor.
    pub name: String,
    /// URI of the mounted filesystem root, as GIO spells it.
    pub root_uri: String,
    /// Hidden behind a replacement mount.
    pub shadowed: bool,
    /// The mount offers an unmount operation.
    pub can_unmount: bool,
}

impl MountFacts {
    /// Reads the facts of a real mount.
    pub fn from_mount(mount: &gio::Mount) -> Self {
        Self {
            name: mount.name().to_string(),
            root_uri: mount.root().uri().to_string(),
            shadowed: mount.is_shadowed(),
            can_unmount: mount.can_unmount(),
        }
    }
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
    /// Reads the facts of a real volume.
    pub fn from_volume(volume: &gio::Volume) -> Self {
        Self {
            name: volume.name().to_string(),
            mounted: volume.get_mount().is_some(),
            can_mount: volume.can_mount(),
            uuid: volume.uuid().map(Into::into),
            unix_device: volume.identifier("unix-device").map(Into::into),
            activation_uri: volume.activation_root().map(|root| root.uri().into()),
        }
    }

    /// A stable-enough identifier for a single mount request, as
    /// `volume_id` in `desktop/volume_locations.py`.
    pub fn id(&self) -> String {
        self.uuid
            .clone()
            .or_else(|| self.unix_device.clone())
            .or_else(|| self.activation_uri.clone())
            .unwrap_or_else(|| self.name.clone())
    }
}

fn kind_for(uri: &str) -> VolumeKind {
    if location::is_device_location(uri) {
        VolumeKind::Device
    } else {
        VolumeKind::Drive
    }
}

/// The row for a mount, or `None` when it is shadowed or its root is not a
/// supported, credential-free location.
fn mounted_row(mount: &MountFacts) -> Option<VolumeRow> {
    if mount.shadowed {
        return None;
    }
    let uri = location::normalise(&mount.root_uri).ok()?;
    Some(VolumeRow {
        label: mount.name.clone(),
        kind: kind_for(&uri),
        uri: Some(uri),
        id: None,
        mounted: true,
        can_unmount: mount.can_unmount,
    })
}

/// The click-to-connect row for a volume that can be mounted.
///
/// Like the Python module, a volume whose activation root is refused by
/// normalisation is skipped: once mounted, its root would be hidden anyway.
fn mountable_row(volume: &VolumeFacts) -> Option<VolumeRow> {
    if volume.mounted || !volume.can_mount {
        return None;
    }
    let activation = match &volume.activation_uri {
        Some(uri) => Some(location::normalise(uri).ok()?),
        None => None,
    };
    Some(VolumeRow {
        label: volume.name.clone(),
        uri: None,
        id: Some(volume.id()),
        kind: activation.as_deref().map_or(VolumeKind::Drive, kind_for),
        mounted: false,
        can_unmount: false,
    })
}

/// Mounted rows first, then unmounted volumes, as `locations()` does.
pub fn locations(mounts: &[MountFacts], volumes: &[VolumeFacts]) -> Vec<VolumeRow> {
    let mounted = mounts.iter().filter_map(mounted_row);
    let mountable = volumes.iter().filter_map(mountable_row);
    mounted.chain(mountable).collect()
}

/// Reads the rows from a real volume monitor.
pub fn from_monitor(monitor: &gio::VolumeMonitor) -> Vec<VolumeRow> {
    let mounts: Vec<MountFacts> = monitor.mounts().iter().map(MountFacts::from_mount).collect();
    let volumes: Vec<VolumeFacts> = monitor.volumes().iter().map(VolumeFacts::from_volume).collect();
    locations(&mounts, &volumes)
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

    fn row_uri(root_uri: &str) -> Option<String> {
        let rows = locations(&[mount("Disk", root_uri)], &[]);
        rows.into_iter().next().and_then(|row| row.uri)
    }

    /// Ported from `desktop/tests/test_volume_locations.py::test_mounted_mtp_phone_and_afc_device_are_visible`
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

    /// Ported from `desktop/tests/test_volume_locations.py::test_unmounted_phone_is_click_to_connect_device`
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

    /// Ported from `desktop/tests/test_volume_locations.py::test_unsupported_and_shadowed_mounts_stay_hidden`
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

    #[test]
    fn mount_roots_use_the_canonical_spelling_tabs_store() {
        let gio_spelling = gio::File::for_path("/media/u/Backup (2024)").uri();
        assert_eq!(
            row_uri(&gio_spelling).as_deref(),
            Some("file:///media/u/Backup%20%282024%29")
        );
        assert_eq!(
            row_uri("file:///media/u/Bob's%20USB").as_deref(),
            Some("file:///media/u/Bob%27s%20USB")
        );
    }

    #[test]
    fn mount_roots_with_a_user_name_are_hidden() {
        assert_eq!(row_uri("smb://user@nas/share/"), None);
    }

    #[test]
    fn smb_roots_lose_the_trailing_slash_and_the_host_case() {
        assert_eq!(row_uri("smb://NAS/share/").as_deref(), Some("smb://nas/share"));
    }

    #[test]
    fn device_roots_keep_their_authority() {
        assert_eq!(
            row_uri("mtp://[usb:001,010]/").as_deref(),
            Some("mtp://[usb:001,010]/")
        );
    }

    #[test]
    fn a_volume_with_an_unsupported_activation_root_is_skipped() {
        let web = VolumeFacts {
            name: "Web".into(),
            can_mount: true,
            activation_uri: Some("https://example.invalid/".into()),
            ..VolumeFacts::default()
        };
        assert!(locations(&[], &[web]).is_empty());
    }

    #[test]
    fn only_mounted_smb_rows_count_as_network() {
        let rows = locations(
            &[
                mount("share on nas", "smb://nas/share"),
                mount("Disk", "file:///media/u/Disk"),
            ],
            &[],
        );
        let network: Vec<bool> = rows.iter().map(VolumeRow::is_network).collect();
        assert_eq!(network, [true, false]);
    }
}
