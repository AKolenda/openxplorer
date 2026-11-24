// SPDX-License-Identifier: AGPL-3.0-only
//! Drives, volumes, phones and connected shares from the volume monitor.
//!
//! Ports `v2.0.0:desktop/volume_locations.py` and its tests
//! (`v2.0.0:desktop/tests/test_volume_locations.py`). The volume monitor's current
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
use ox_core::integration::DiskTool;
use ox_core::location;
use ox_core::network::volume_id_from;

use crate::icons::{Art, Icon, Medium};

/// The file system attributes a drive's space is read from.
pub(crate) const SPACE_ATTRIBUTES: &str = "filesystem::free,filesystem::size";

/// The free space and size, in bytes, of the file system GIO described in
/// `filesystem`, with the free space at most the size; `None` when GIO
/// could not tell or the size is zero. This PC's drive tiles and a
/// drive's Properties both read it here, so they show one capacity.
pub(crate) fn space_of(filesystem: &gio::FileInfo) -> Option<(u64, u64)> {
    let known = filesystem.has_attribute("filesystem::free") && filesystem.has_attribute("filesystem::size");
    let size = filesystem.attribute_uint64("filesystem::size");
    let free = filesystem.attribute_uint64("filesystem::free").min(size);
    (known && size > 0).then_some((free, size))
}

/// What a row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumeKind {
    /// A disk, partition or network mount.
    Drive,
    /// A phone, camera or iOS device.
    Device,
    /// A Google Drive or `OneDrive` account from GNOME Online Accounts
    /// (DEV-013).
    Cloud,
}

impl VolumeKind {
    /// The kind of a location at the canonical `root`: a device for the
    /// `mtp:`, `gphoto2:` and `afc:` schemes, an online-account drive for
    /// `google-drive:` and `onedrive:`, a drive otherwise.
    fn of_root(root: &str) -> Self {
        if location::is_device_location(root) {
            VolumeKind::Device
        } else if location::is_cloud_location(root) {
            VolumeKind::Cloud
        } else {
            VolumeKind::Drive
        }
    }
}

/// What the desktop lets the user do with a mount: the commands of its
/// sidebar row and its This PC card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent command the desktop allows, not a state"
)]
pub(crate) struct MountControls {
    /// The mount can be unmounted: Disconnect (`canUnmount` in
    /// `v2.0.0:desktop/volume_locations.py`).
    pub can_unmount: bool,
    /// The medium can be ejected: Eject, for removable drives, SD cards
    /// and optical discs (`g_mount_can_eject`).
    pub can_eject: bool,
    /// The drive can be powered off: Safely remove, for USB disks
    /// (`g_drive_can_stop`).
    pub can_stop: bool,
    /// A removable drive with a block device, and GNOME Disks is
    /// installed: Open in Disks (DEV-012).
    pub can_open_in_disks: bool,
    /// A removable drive with a block device: Format…, the app's own
    /// dialog (DEV-012).
    pub can_format: bool,
    /// Not a command: the medium is an optical disc (a `/dev/sr` device),
    /// which a Windows icon file draws as an optical drive (LOOK-030).
    pub is_optical: bool,
}

impl MountControls {
    /// A mount the user cannot remove, such as Local Disk.
    pub(crate) const FIXED: Self = Self {
        can_unmount: false,
        can_eject: false,
        can_stop: false,
        can_open_in_disks: false,
        can_format: false,
        is_optical: false,
    };

    /// A mount that can only be unmounted, such as an internal partition
    /// or a network share.
    #[cfg(test)]
    pub(crate) const UNMOUNTABLE: Self = Self {
        can_unmount: true,
        can_eject: false,
        can_stop: false,
        can_open_in_disks: false,
        can_format: false,
        is_optical: false,
    };

    /// Reads what `mount` and its drive allow.
    fn of_mount(mount: &gio::Mount) -> Self {
        let drive = mount.drive();
        let can_eject = mount.can_eject();
        let can_stop = drive.is_some_and(|drive| drive.can_stop());
        let has_block_device = mount
            .volume()
            .is_some_and(|volume| volume.identifier("unix-device").is_some());
        let is_removable = can_eject || can_stop;
        Self {
            can_unmount: mount.can_unmount(),
            can_eject,
            can_stop,
            can_open_in_disks: is_removable
                && has_block_device
                && crate::window::is_disk_tool_installed(DiskTool::OpenInDisks),
            can_format: is_removable && has_block_device,
            is_optical: is_optical_disc(mount),
        }
    }
}

/// Whether `mount` is an optical disc: its volume's or drive's device is
/// a SCSI CD-ROM (`/dev/sr0`), as the Linux kernel names every optical
/// drive.
fn is_optical_disc(mount: &gio::Mount) -> bool {
    let volume_device = mount.volume().and_then(|volume| volume.identifier("unix-device"));
    let drive_device = mount.drive().and_then(|drive| drive.identifier("unix-device"));
    volume_device
        .or(drive_device)
        .is_some_and(|device| device.starts_with("/dev/sr"))
}

/// Whether a location can be browsed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeState {
    /// Mounted and browsable.
    Mounted {
        /// The canonical root, spelled as tabs store it.
        uri: String,
        /// Disconnect, Eject and Safely remove, as the mount allows them.
        controls: MountControls,
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
    /// The icon of the row's drive or device, in the navigation pane and on
    /// This PC: a phone for a device; for a drive, the drive glyph, which a
    /// Windows icon file draws as a removable or optical drive when the
    /// mount can be ejected or stopped (LOOK-030).
    pub(crate) fn art(&self) -> Art {
        if self.kind == VolumeKind::Device {
            return Art::Glyph(Icon::Phone);
        }
        if self.kind == VolumeKind::Cloud {
            return Art::Glyph(Icon::Cloud);
        }
        match &self.state {
            VolumeState::Mounted { controls, .. } if controls.is_optical => {
                Art::RemovableDrive(Medium::Optical)
            }
            VolumeState::Mounted { controls, .. } if controls.can_eject || controls.can_stop => {
                Art::RemovableDrive(Medium::Disk)
            }
            // The root, Local Disk, is the system's own drive.
            VolumeState::Mounted { uri, .. } if location::same_location(uri, "file:///") => Art::SystemDrive,
            VolumeState::Mounted { .. } | VolumeState::Mountable { .. } => Art::Glyph(Icon::HardDrive),
        }
    }

    /// The mounted root, or `None` while the volume still has to be mounted.
    pub(crate) fn uri(&self) -> Option<&str> {
        match &self.state {
            VolumeState::Mounted { uri, .. } => Some(uri),
            VolumeState::Mountable { .. } => None,
        }
    }

    /// True for a mounted SMB share or other server (SFTP, FTP, WebDAV,
    /// NFS). It belongs under Network, never among
    /// the drives (`!m.uri?.startsWith('smb:')` in app.js). A Google Drive
    /// or `OneDrive` account stays among the drives, mounted or not, as
    /// GNOME Files lists it with them.
    pub(crate) fn is_network(&self) -> bool {
        self.kind != VolumeKind::Cloud && self.uri().is_some_and(location::is_server_location)
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
    /// What the mount and its drive let the user do.
    pub controls: MountControls,
}

impl MountFacts {
    /// Reads the facts of a real mount.
    pub(crate) fn from_mount(mount: &gio::Mount) -> Self {
        Self {
            name: mount.name().to_string(),
            root_uri: mount.root().uri().to_string(),
            shadowed: mount.is_shadowed(),
            controls: MountControls::of_mount(mount),
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
    /// `volume_id` in `v2.0.0:desktop/volume_locations.py`: the UUID, else the
    /// device path, else the activation root, else the name. ox-core's
    /// [`volume_id_from`] decides, so a row and the mount it requests
    /// always agree.
    pub(crate) fn id(&self) -> &str {
        volume_id_from(
            self.uuid.as_deref(),
            self.unix_device.as_deref(),
            self.activation_uri.as_deref(),
            &self.name,
        )
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
            controls: mount.controls,
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
/// `v2.0.0:desktop/volume_locations.py` lists them.
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
            controls: MountControls::UNMOUNTABLE,
        }
    }

    /// The URI of the row for a single mount at `root_uri`, or `None` when
    /// the mount is hidden.
    fn row_uri(root_uri: &str) -> Option<String> {
        let rows = locations(&[mount("Disk", root_uri)], &[]);
        let row = rows.into_iter().next()?;
        row.uri().map(str::to_owned)
    }

    /// Ported from `v2.0.0:desktop/tests/test_volume_locations.py::test_mounted_mtp_phone_and_afc_device_are_visible`
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

    /// Ported from `v2.0.0:desktop/tests/test_volume_locations.py::test_unmounted_phone_is_click_to_connect_device`
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

    /// Ported from `v2.0.0:desktop/tests/test_volume_locations.py::test_unsupported_and_shadowed_mounts_stay_hidden`
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

    /// A Google Drive or `OneDrive` account is a cloud drive among the
    /// drives, waiting to connect or connected, named by its account, as
    /// GNOME Online Accounts and `GVfs` offer it.
    ///
    /// parity: DEV-013
    #[test]
    fn online_account_drives_are_listed_among_the_drives() {
        let google = VolumeFacts {
            name: "alice@example.com".into(),
            can_mount: true,
            uuid: Some("google-drive://alice@example.com/".into()),
            activation_uri: Some("google-drive://alice@example.com/".into()),
            ..VolumeFacts::default()
        };
        let rows = locations(
            &[mount("bob@example.com", "onedrive://bob@example.com/")],
            std::slice::from_ref(&google),
        );
        assert_eq!(
            rows,
            [
                VolumeRow {
                    label: "bob@example.com".into(),
                    kind: VolumeKind::Cloud,
                    state: VolumeState::Mounted {
                        uri: "onedrive://bob@example.com/".into(),
                        controls: MountControls::UNMOUNTABLE,
                    },
                },
                VolumeRow {
                    label: "alice@example.com".into(),
                    kind: VolumeKind::Cloud,
                    state: VolumeState::Mountable {
                        id: "google-drive://alice@example.com/".into()
                    },
                },
            ]
        );
        assert!(rows.iter().all(|row| !row.is_network()));
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
