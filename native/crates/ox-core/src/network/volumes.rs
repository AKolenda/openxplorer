// SPDX-License-Identifier: AGPL-3.0-only
//! Connecting drives and phones, and Disconnect.
//!
//! Ports the `mountVolume` and `unmount` operations of
//! `desktop/winspace.py` and `volume_id` of `desktop/volume_locations.py`.
//! Both take the mount operation from the interface: GTK's shows the
//! password dialog of an encrypted disk and the programs that keep a mount
//! busy. Dropping a future cancels its GIO call.

use gio::prelude::*;

use super::error::NetworkError;
use super::mounting::WriteActivity;
use crate::location::normalise;

/// The identifier of `volume` for a single mount request: its UUID, else
/// its device path, else its activation root, else its name.
pub fn volume_id(volume: &gio::Volume) -> String {
    let activation_uri = || volume.activation_root().map(|root| root.uri());
    let id = volume
        .uuid()
        .or_else(|| volume.identifier("unix-device"))
        .or_else(activation_uri);
    id.map_or_else(|| volume.name().into(), Into::into)
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
    // Safety rule (DEV-006): unmounting never interrupts this window's
    // own writes.
    if activity == WriteActivity::Writing {
        return Err(NetworkError::WriteInProgress);
    }
    let file = gio::File::for_uri(&normalise(uri)?);
    let holding = mounts.iter().find(|mount| {
        let root = mount.root();
        file.equal(&root) || file.has_prefix(&root)
    });
    let mount = holding.ok_or(NetworkError::NoUserMount)?;
    if !mount.can_unmount() {
        return Err(NetworkError::UnmountNotPermitted);
    }
    mount
        .unmount_with_operation_future(gio::MountUnmountFlags::NONE, operation)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// parity: DEV-003
    #[test]
    fn a_volume_that_went_away_is_reported_as_gone() {
        let context = glib::MainContext::new();

        let refused = context.block_on(mount_volume(&[], "1234-ABCD", None));

        let error = refused.expect_err("the volume is gone");
        assert_eq!(error.to_string(), "This volume is no longer available.");
    }
}
