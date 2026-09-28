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
pub(crate) enum VolumeKind {
    /// A disk, partition or network mount.
    Drive,
    /// A phone, camera or iOS device.
    Device,
}

impl VolumeKind {
    /// The kind of a location at the canonical `root`: a device for the
    /// `mtp:`, `gphoto2:` and `afc:` schemes, a drive otherwise.
    fn of_root(root: &str) -> Self {
        if location::is_device_location(root) {
            VolumeKind::Device
        } else {
            VolumeKind::Drive
        }
    }
}

/// Whether a location can be browsed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeState {
    /// Mounted and browsable.
    Mounted {
        /// The canonical root, spelled as tabs store it.
        uri: String,
        /// The mount offers a Disconnect command.
        can_unmount: bool,
    },
    /// Not mounted yet; clicking the row mounts it.
    Mountable {
        /// Identifies the volume to mount ([`VolumeFacts::id`]).
        id: String,
    },
}

/// One mounted or mountable location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeRow {
    /// Name shown in the sidebar and on This PC.
    pub label: String,
    /// Drive or device glyph.
    pub kind: VolumeKind,
    /// Mounted at a root, or waiting to be mounted.
    pub state: VolumeState,
}

impl VolumeRow {
    /// The mounted root, or `None` while the volume still has to be mounted.
    pub(crate) fn uri(&self) -> Option<&str> {
        match &self.state {
            VolumeState::Mounted { uri, .. } => Some(uri),
            VolumeState::Mountable { .. } => None,
        }
    }

    /// True for a mounted SMB share. It belongs under Network, never among
    /// the drives (`!m.uri?.startsWith('smb:')` in app.js).
    pub(crate) fn is_network(&self) -> bool {
        self.uri().is_some_and(|uri| uri.starts_with("smb:"))
    }
}

/// The parts of a `gio::Mount` this module reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MountFacts {
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
    pub(crate) fn from_mount(mount: &gio::Mount) -> Self {
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
pub(crate) struct VolumeFacts {
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
    pub(crate) fn from_volume(volume: &gio::Volume) -> Self {
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
    /// `volume_id` in `desktop/volume_locations.py`: the UUID, else the
    /// device path, else the activation root, else the name.
    pub(crate) fn id(&self) -> &str {
        self.uuid
            .as_deref()
            .or(self.unix_device.as_deref())
            .or(self.activation_uri.as_deref())
            .unwrap_or(&self.name)
    }
}

/// The row for a mount, or `None` when it is shadowed or its root is not a
/// supported, credential-free location.
fn mounted_row(mount: &MountFacts) -> Option<VolumeRow> {
    if mount.shadowed {
        return None;
    }
    // Data safety (SAFE-010): normalisation is the scheme allowlist and
    // refuses user names, so a root it rejects is hidden instead of shown
    // or opened.
    let uri = location::normalise(&mount.root_uri).ok()?;
    Some(VolumeRow {
        label: mount.name.clone(),
        kind: VolumeKind::of_root(&uri),
        state: VolumeState::Mounted {
            uri,
            can_unmount: mount.can_unmount,
        },
    })
}

/// The click-to-connect row for a volume that can be mounted, or `None`
/// for a volume that is mounted already or cannot be mounted.
fn mountable_row(volume: &VolumeFacts) -> Option<VolumeRow> {
    if volume.mounted || !volume.can_mount {
        return None;
    }
    let kind = match volume.activation_uri.as_deref() {
        None => VolumeKind::Drive,
        Some(activation_uri) => {
            // Like the Python module, a volume whose activation root
            // normalisation refuses is skipped: once mounted, its root
            // would be hidden anyway.
            let root = location::normalise(activation_uri).ok()?;
            VolumeKind::of_root(&root)
        }
    };
    Some(VolumeRow {
        label: volume.name.clone(),
        kind,
        state: VolumeState::Mountable {
            id: volume.id().to_owned(),
        },
    })
}

/// Mounted rows first, then unmounted volumes, as `locations()` in
/// `desktop/volume_locations.py` lists them.
pub(crate) fn locations(mounts: &[MountFacts], volumes: &[VolumeFacts]) -> Vec<VolumeRow> {
    let mounted = mounts.iter().filter_map(mounted_row);
    let mountable = volumes.iter().filter_map(mountable_row);
    mounted.chain(mountable).collect()
}

/// Reads the rows from a real volume monitor. Reading the monitor's state
/// never mounts, probes or lists a device.
pub(crate) fn from_monitor(monitor: &gio::VolumeMonitor) -> Vec<VolumeRow> {
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

    /// The URI of the row for a single mount at `root_uri`, or `None` when
    /// the mount is hidden.
    fn row_uri(root_uri: &str) -> Option<String> {
        let rows = locations(&[mount("Disk", root_uri)], &[]);
        let row = rows.into_iter().next()?;
        row.uri().map(str::to_owned)
    }

    /// Ported from `desktop/tests/test_volume_locations.py::test_mounted_mtp_phone_and_afc_device_are_visible`
    ///
    /// parity: DEV-001
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
        assert!(rows.iter().all(|row| row.uri().is_some()), "every row is mounted");
    }

    /// Ported from `desktop/tests/test_volume_locations.py::test_unmounted_phone_is_click_to_connect_device`
    ///
    /// parity: DEV-001
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
                kind: VolumeKind::Device,
                state: VolumeState::Mountable {
                    id: "mtp://[usb:001,011]/".into()
                },
            }]
        );
        assert_eq!(phone.id(), "mtp://[usb:001,011]/");
    }

    /// Ported from `desktop/tests/test_volume_locations.py::test_unsupported_and_shadowed_mounts_stay_hidden`
    ///
    /// parity: DEV-001
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
    fn volume_ids_prefer_uuid_then_device_then_activation_root_then_name() {
        let mut volume = VolumeFacts {
            name: "USB".into(),
            uuid: Some("1234-ABCD".into()),
            unix_device: Some("/dev/sdb1".into()),
            activation_uri: Some("mtp://[usb:001,011]/".into()),
            ..VolumeFacts::default()
        };
        assert_eq!(volume.id(), "1234-ABCD");
        volume.uuid = None;
        assert_eq!(volume.id(), "/dev/sdb1");
        volume.unix_device = None;
        assert_eq!(volume.id(), "mtp://[usb:001,011]/");
        volume.activation_uri = None;
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

    /// parity: DEV-001, SAFE-010
    #[test]
    fn mount_roots_with_a_user_name_are_hidden() {
        assert_eq!(row_uri("smb://user@nas/share/"), None);
    }

    #[test]
    fn smb_roots_lose_the_trailing_slash_and_the_host_case() {
        assert_eq!(row_uri("smb://NAS/share/").as_deref(), Some("smb://nas/share"));
    }

    /// parity: DEV-005
    #[test]
    fn device_roots_keep_their_authority() {
        assert_eq!(
            row_uri("mtp://[usb:001,010]/").as_deref(),
            Some("mtp://[usb:001,010]/")
        );
    }

    /// parity: DEV-001
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

    /// parity: NET-018
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
