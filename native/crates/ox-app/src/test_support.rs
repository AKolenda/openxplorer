// SPDX-License-Identifier: AGPL-3.0-only
//! What the crate's tests share: listed entries built through ox-core's own
//! conversion, so they carry every field a real listing does, rows of the
//! Network list and of the volume monitor, the GTK [`harness`] for tests
//! that open windows, [`portal`], which exports fake desktop portals,
//! [`python`], which runs the Python app's settings code, and [`search`],
//! which starts the search cache beside a window.

pub(crate) mod harness;
pub(crate) mod portal;
pub(crate) mod python;
pub(crate) mod search;

use gtk::gio;
use ox_core::entry::{entry_from_info, Entry};
use ox_core::places::{NetworkKind, NetworkLocation};

use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};

/// An entry named `name` in `/tmp/ox-test`, of `file_type`, as a listing
/// would produce it. Nothing is created on disk.
fn entry(name: &str, file_type: gio::FileType) -> Entry {
    let info = gio::FileInfo::new();
    info.set_file_type(file_type);
    info.set_display_name(name);
    let file = gio::File::for_path(format!("/tmp/ox-test/{name}"));
    entry_from_info(&file, &info)
}

/// A regular file named `name` in `/tmp/ox-test`.
pub(crate) fn file_entry(name: &str) -> Entry {
    entry(name, gio::FileType::Regular)
}

/// A folder named `name` in `/tmp/ox-test`.
pub(crate) fn folder_entry(name: &str) -> Entry {
    entry(name, gio::FileType::Directory)
}

/// A volume monitor row for a mounted volume of `kind` called `label`,
/// open at `uri`, that the user may unmount.
pub(crate) fn mounted_volume(label: &str, uri: &str, kind: VolumeKind) -> VolumeRow {
    VolumeRow {
        label: label.into(),
        kind,
        state: VolumeState::Mounted {
            uri: uri.into(),
            controls: MountControls::UNMOUNTABLE,
        },
    }
}

/// The Network row of the SMB server `smb://studio-nas/`, browsed this
/// session and connected.
pub(crate) fn studio_nas_server() -> NetworkLocation {
    NetworkLocation {
        uri: "smb://studio-nas/".to_owned(),
        label: "studio-nas".to_owned(),
        is_saved: false,
        is_connected: true,
        kind: NetworkKind::Server,
    }
}

/// The Network row of the share `smb://studio-nas/projects`, saved as the
/// mapped drive "Studio NAS (Z:)" and not connected now.
pub(crate) fn studio_nas_mapped_drive() -> NetworkLocation {
    NetworkLocation {
        uri: "smb://studio-nas/projects".to_owned(),
        label: "Studio NAS (Z:)".to_owned(),
        is_saved: true,
        is_connected: false,
        kind: NetworkKind::Share,
    }
}
