// SPDX-License-Identifier: AGPL-3.0-only
//! Taking a drive or phone away: Disconnect, Eject and Safely remove.
//!
//! Ports Disconnect of the This PC cards (`unmount` in `desktop/ui/app.js`
//! and `desktop/winspace.py`) and adds Dolphin's Eject and Safely remove
//! (DEV-007, DEV-008), which the Python app lacks. What a mount allows
//! comes from the volume monitor ([`MountControls`]); ox-core's
//! [`network`](ox_core::network) service does the removing, with GTK's
//! mount operation, which shows the programs that keep a drive busy and
//! the desktop's "safe to remove" message.

use gtk::gio;
use ox_core::network::{
    eject_location, safely_remove_location, unmount_location, NetworkError, WriteActivity,
};

use crate::volumes::{MountControls, VolumeKind};

/// How a mounted drive or device is taken away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    /// Unmounts the mount that holds the location, for every application
    /// of the session; the medium stays in the drive.
    Disconnect,
    /// Unmounts every mount of the drive and ejects its medium.
    Eject,
    /// Unmounts every mount of the drive and powers it off.
    SafelyRemove,
}

impl Removal {
    /// The removals `controls` allow, in menu order: Disconnect, Eject,
    /// then Safely remove, as GNOME Files and Dolphin list Unmount, Eject
    /// and Safely Remove.
    pub(crate) fn offered(controls: MountControls) -> Vec<Removal> {
        let allowed = [
            (Removal::Disconnect, controls.can_unmount),
            (Removal::Eject, controls.can_eject),
            (Removal::SafelyRemove, controls.can_stop),
        ];
        let allowed = allowed.into_iter().filter(|(_, is_allowed)| *is_allowed);
        allowed.map(|(removal, _)| removal).collect()
    }

    /// What the eject button of a sidebar row does: eject the medium, else
    /// unmount, as GNOME's places sidebar decides; `None` without either.
    pub(crate) fn for_eject_button(controls: MountControls) -> Option<Removal> {
        if controls.can_eject {
            Some(Removal::Eject)
        } else if controls.can_unmount {
            Some(Removal::Disconnect)
        } else {
            None
        }
    }

    /// The menu item: "Disconnect device" for a phone or camera and
    /// "Disconnect mount" for a drive (`unmount` in app.js), "Eject" and
    /// "Safely remove".
    pub(crate) fn label(self, kind: VolumeKind) -> &'static str {
        match (self, kind) {
            (Removal::Disconnect, VolumeKind::Device) => "Disconnect device",
            (Removal::Disconnect, VolumeKind::Drive) => "Disconnect mount",
            (Removal::Eject, _) => "Eject",
            (Removal::SafelyRemove, _) => "Safely remove",
        }
    }

    /// What the app says once the drive `label` is gone: only Safely
    /// remove says it may be unplugged, as Dolphin does; the desktop tells
    /// when an ejected medium's writes are flushed.
    pub(crate) fn done_message(self, label: &str) -> Option<String> {
        match self {
            Removal::SafelyRemove => Some(format!("“{label}” can now be safely unplugged.")),
            Removal::Disconnect | Removal::Eject => None,
        }
    }

    /// The heading of the message when the removal failed ("Could not
    /// disconnect" in app.js).
    pub(crate) fn failure_title(self) -> &'static str {
        match self {
            Removal::Disconnect => "Could not disconnect",
            Removal::Eject => "Could not eject",
            Removal::SafelyRemove => "Could not safely remove",
        }
    }

    /// Whether the user already heard about `error`: they cancelled, or
    /// GTK's dialog showed why (`G_IO_ERROR_FAILED_HANDLED`, which GIO asks
    /// never to report again).
    pub(crate) fn is_reported_by_desktop(error: &NetworkError) -> bool {
        let is_handled = matches!(
            error,
            NetworkError::Gio(error) if error.matches(gio::IOErrorEnum::FailedHandled)
        );
        is_handled || error.is_cancelled()
    }

    /// Takes away the mount among `mounts` (the volume monitor's) that
    /// holds `uri`, answering its questions through `operation`.
    ///
    /// # Errors
    ///
    /// The [`NetworkError`] of ox-core's removal: a write running in this
    /// window, a location outside every mount, a mount the system keeps,
    /// or GIO's error.
    pub(crate) async fn perform(
        self,
        mounts: &[gio::Mount],
        uri: &str,
        operation: &gio::MountOperation,
        activity: WriteActivity,
    ) -> Result<(), NetworkError> {
        let operation = Some(operation);
        match self {
            Removal::Disconnect => unmount_location(mounts, uri, operation, activity).await,
            Removal::Eject => eject_location(mounts, uri, operation, activity).await,
            Removal::SafelyRemove => safely_remove_location(mounts, uri, operation, activity).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OfferCase {
        controls: MountControls,
        offered: &'static [Removal],
        eject_button: Option<Removal>,
    }

    /// A USB disk can be ejected and powered off, an SD card ejected, an
    /// internal partition or a phone only unmounted, and Local Disk
    /// nothing.
    ///
    /// parity: DEV-007, DEV-008
    #[test]
    fn each_drive_offers_what_it_allows() {
        let usb_disk = MountControls {
            can_unmount: true,
            can_eject: true,
            can_stop: true,
        };
        let sd_card = MountControls {
            can_stop: false,
            ..usb_disk
        };
        let cases = [
            OfferCase {
                controls: usb_disk,
                offered: &[Removal::Disconnect, Removal::Eject, Removal::SafelyRemove],
                eject_button: Some(Removal::Eject),
            },
            OfferCase {
                controls: sd_card,
                offered: &[Removal::Disconnect, Removal::Eject],
                eject_button: Some(Removal::Eject),
            },
            OfferCase {
                controls: MountControls::UNMOUNTABLE,
                offered: &[Removal::Disconnect],
                eject_button: Some(Removal::Disconnect),
            },
            OfferCase {
                controls: MountControls::FIXED,
                offered: &[],
                eject_button: None,
            },
        ];
        for case in cases {
            assert_eq!(
                Removal::offered(case.controls),
                case.offered,
                "{:?}",
                case.controls
            );
            assert_eq!(
                Removal::for_eject_button(case.controls),
                case.eject_button,
                "{:?}",
                case.controls
            );
        }
    }

    /// parity: DEV-006
    #[test]
    fn disconnect_names_a_device_or_a_mount_as_app_js_does() {
        assert_eq!(Removal::Disconnect.label(VolumeKind::Device), "Disconnect device");
        assert_eq!(Removal::Disconnect.label(VolumeKind::Drive), "Disconnect mount");
        assert_eq!(Removal::Eject.label(VolumeKind::Drive), "Eject");
        assert_eq!(Removal::SafelyRemove.label(VolumeKind::Drive), "Safely remove");
    }

    /// parity: DEV-008
    #[test]
    fn only_safely_remove_says_the_drive_can_be_unplugged() {
        assert_eq!(
            Removal::SafelyRemove.done_message("USB stick").as_deref(),
            Some("“USB stick” can now be safely unplugged.")
        );
        assert_eq!(Removal::Eject.done_message("USB stick"), None);
        assert_eq!(Removal::Disconnect.done_message("USB stick"), None);
    }
}
