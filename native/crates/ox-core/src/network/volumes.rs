// SPDX-License-Identifier: AGPL-3.0-only
//! Connecting drives and phones, Disconnect, Eject and Safely remove.
//!
//! Ports the `mountVolume` and `unmount` operations of
//! `v2.0.0:desktop/winspace.py` and `volume_id` of `v2.0.0:desktop/volume_locations.py`,
//! and adds Dolphin's Eject and Safely remove (DEV-007, DEV-008), which
//! the Python app lacks. Each takes the mount operation from the
//! interface: GTK's shows the password dialog of an encrypted disk, the
//! programs that keep a mount busy and the desktop's "safe to remove"
//! message. Dropping a future cancels its GIO call.

use gio::prelude::*;

use super::error::NetworkError;
use super::mounting::WriteActivity;
use crate::location::normalise;

/// The identifier of `volume` for a single mount request; see
/// [`volume_id_from`].
pub fn volume_id(volume: &gio::Volume) -> String {
    let uuid = volume.uuid();
    let unix_device = volume.identifier("unix-device");
    let activation_uri = volume.activation_root().map(|root| root.uri());
    let name = volume.name();
    let id = volume_id_from(
        uuid.as_deref(),
        unix_device.as_deref(),
        activation_uri.as_deref(),
        &name,
    );
    id.to_owned()
}

/// The identifier of a volume for a single mount request, from what the
/// volume monitor reports: its UUID, else its device path, else its
/// activation root, else its name.
///
/// The one implementation of `volume_id` in `v2.0.0:desktop/volume_locations.py`:
/// the rows the app lists and [`mount_volume`] must agree on it, or
/// Connect would not find the volume. An empty value counts as missing, as
/// Python's `or` does.
pub fn volume_id_from<'a>(
    uuid: Option<&'a str>,
    unix_device: Option<&'a str>,
    activation_uri: Option<&'a str>,
    name: &'a str,
) -> &'a str {
    [uuid, unix_device, activation_uri]
        .into_iter()
        .flatten()
        .find(|id| !id.is_empty())
        .unwrap_or(name)
}

/// Connects a drive or phone: mounts the volume `id` among `volumes` (the
/// volume monitor's) and returns its root as GIO spells it. An already
/// mounted volume just returns its root.
///
/// # Errors
///
/// [`NetworkError::VolumeUnavailable`] when the volume is gone,
/// [`NetworkError::NoMountReturned`] when mounting left no mount, or
/// GIO's error.
pub async fn mount_volume(
    volumes: &[gio::Volume],
    id: &str,
    operation: Option<&gio::MountOperation>,
) -> Result<String, NetworkError> {
    let volume = volumes
        .iter()
        .find(|volume| volume_id(volume) == id)
        .ok_or(NetworkError::VolumeUnavailable)?;
    if let Some(mount) = volume.get_mount() {
        return Ok(mount.root().uri().into());
    }
    volume.mount_future(gio::MountMountFlags::NONE, operation).await?;
    let mount = volume.get_mount().ok_or(NetworkError::NoMountReturned)?;
    Ok(mount.root().uri().into())
}

/// Disconnect: unmounts the user mount among `mounts` (the volume
/// monitor's) that holds `uri`, for every application of the session. It
/// unmounts only; it never ejects or powers off a drive.
///
/// # Errors
///
/// [`NetworkError::WriteInProgress`] while this window writes,
/// [`NetworkError::NoUserMount`] or [`NetworkError::UnmountNotPermitted`]
/// for a location that cannot be disconnected, or GIO's error.
pub async fn unmount_location(
    mounts: &[gio::Mount],
    uri: &str,
    operation: Option<&gio::MountOperation>,
    activity: WriteActivity,
) -> Result<(), NetworkError> {
    let mount = mount_to_remove(mounts, uri, activity)?;
    if !mount.can_unmount() {
        return Err(NetworkError::UnmountNotPermitted);
    }
    mount
        .unmount_with_operation_future(gio::MountUnmountFlags::NONE, operation)
        .await?;
    Ok(())
}

/// Eject: unmounts every mount of the drive that holds `uri`, among
/// `mounts` (the volume monitor's), and ejects its medium, as GNOME and
/// Dolphin eject removable drives, SD cards and optical discs.
///
/// # Errors
///
/// [`NetworkError::WriteInProgress`] while this window writes,
/// [`NetworkError::NoUserMount`] for a location outside every mount,
/// [`NetworkError::CannotEject`] for a medium that cannot be ejected, or
/// GIO's error, for example when a program keeps the drive busy.
pub async fn eject_location(
    mounts: &[gio::Mount],
    uri: &str,
    operation: Option<&gio::MountOperation>,
    activity: WriteActivity,
) -> Result<(), NetworkError> {
    let mount = mount_to_remove(mounts, uri, activity)?;
    if !mount.can_eject() {
        return Err(NetworkError::CannotEject);
    }
    mount
        .eject_with_operation_future(gio::MountUnmountFlags::NONE, operation)
        .await?;
    Ok(())
}

/// Safely remove: unmounts every mount of the drive that holds `uri`,
/// among `mounts` (the volume monitor's), and powers the drive off, as
/// "Safely Remove Drive" in GNOME and Dolphin does for USB disks.
///
/// # Errors
///
/// [`NetworkError::WriteInProgress`] while this window writes,
/// [`NetworkError::NoUserMount`] for a location outside every mount,
/// [`NetworkError::CannotSafelyRemove`] for a drive that cannot be
/// stopped, or GIO's error.
pub async fn safely_remove_location(
    mounts: &[gio::Mount],
    uri: &str,
    operation: Option<&gio::MountOperation>,
    activity: WriteActivity,
) -> Result<(), NetworkError> {
    let mount = mount_to_remove(mounts, uri, activity)?;
    let drive = mount.drive().filter(DriveExt::can_stop);
    let drive = drive.ok_or(NetworkError::CannotSafelyRemove)?;
    drive.stop_future(gio::MountUnmountFlags::NONE, operation).await?;
    Ok(())
}

/// The mount among `mounts` that holds `uri`, for Disconnect, Eject or
/// Safely remove.
///
/// # Errors
///
/// [`NetworkError::WriteInProgress`] while this window writes, the
/// location error of an invalid address, or
/// [`NetworkError::NoUserMount`] for a location outside every mount.
fn mount_to_remove<'a>(
    mounts: &'a [gio::Mount],
    uri: &str,
    activity: WriteActivity,
) -> Result<&'a gio::Mount, NetworkError> {
    // Safety rule (DEV-006): removing a mount never interrupts this
    // window's own writes.
    if activity == WriteActivity::Writing {
        return Err(NetworkError::WriteInProgress);
    }
    let file = gio::File::for_uri(&normalise(uri)?);
    let holding = mounts.iter().find(|mount| {
        let root = mount.root();
        file.equal(&root) || file.has_prefix(&root)
    });
    holding.ok_or(NetworkError::NoUserMount)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VolumeIdCase {
        uuid: Option<&'static str>,
        unix_device: Option<&'static str>,
        activation_uri: Option<&'static str>,
        expected: &'static str,
    }

    /// parity: DEV-003
    #[test]
    fn volume_ids_prefer_uuid_then_device_then_activation_root_then_name() {
        let cases = [
            VolumeIdCase {
                uuid: Some("1234-ABCD"),
                unix_device: Some("/dev/sdb1"),
                activation_uri: Some("mtp://[usb:001,011]/"),
                expected: "1234-ABCD",
            },
            VolumeIdCase {
                uuid: None,
                unix_device: Some("/dev/sdb1"),
                activation_uri: Some("mtp://[usb:001,011]/"),
                expected: "/dev/sdb1",
            },
            VolumeIdCase {
                uuid: Some(""),
                unix_device: None,
                activation_uri: Some("mtp://[usb:001,011]/"),
                expected: "mtp://[usb:001,011]/",
            },
            VolumeIdCase {
                uuid: None,
                unix_device: Some(""),
                activation_uri: None,
                expected: "USB",
            },
        ];
        for case in &cases {
            let id = volume_id_from(case.uuid, case.unix_device, case.activation_uri, "USB");
            assert_eq!(id, case.expected);
        }
    }

    /// parity: DEV-006
    #[test]
    fn disconnect_waits_for_this_windows_writes() {
        let context = glib::MainContext::new();

        let refused = context.block_on(unmount_location(
            &[],
            "file:///media/demo/USB",
            None,
            WriteActivity::Writing,
        ));

        let error = refused.expect_err("disconnect is refused");
        assert_eq!(error.to_string(), "Finish the active file operation first.");
    }

    /// parity: DEV-006
    #[test]
    fn a_location_outside_every_user_mount_cannot_be_disconnected() {
        let context = glib::MainContext::new();

        let refused = context.block_on(unmount_location(
            &[],
            "file:///media/demo/USB",
            None,
            WriteActivity::Idle,
        ));

        let error = refused.expect_err("disconnect is refused");
        assert_eq!(
            error.to_string(),
            "This location has no active user-session mount."
        );
    }

    /// Eject and Safely remove refuse, as Disconnect does, while this
    /// window writes and for a location outside every mount.
    ///
    /// parity: DEV-007, DEV-008
    #[test]
    fn eject_and_safely_remove_wait_for_writes_and_need_a_mount() {
        let context = glib::MainContext::new();
        let usb = "file:///media/demo/USB";

        let ejecting = context.block_on(eject_location(&[], usb, None, WriteActivity::Writing));
        let removing = context.block_on(safely_remove_location(&[], usb, None, WriteActivity::Idle));

        let ejecting = ejecting.expect_err("eject waits for the write");
        let removing = removing.expect_err("no mount holds the location");
        assert_eq!(ejecting.to_string(), "Finish the active file operation first.");
        assert_eq!(
            removing.to_string(),
            "This location has no active user-session mount."
        );
    }

    /// parity: DEV-003
    #[test]
    fn a_volume_that_went_away_is_reported_as_gone() {
        let context = glib::MainContext::new();

        let refused = context.block_on(mount_volume(&[], "1234-ABCD", None));

        let error = refused.expect_err("the volume is gone");
        assert_eq!(error.to_string(), "This volume is no longer available.");
    }
}
