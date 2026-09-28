// SPDX-License-Identifier: AGPL-3.0-only
//! Mounting a drive or device from the sidebar or This PC, then opening
//! it.
//!
//! Ports `mountVolume` in `desktop/ui/app.js`. The mount runs through GIO
//! without blocking the window, and the desktop's own dialog asks for a
//! password when the volume needs one.

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::volumes::VolumeFacts;

use super::BrowserWindow;

impl BrowserWindow {
    /// Mounts the volume `id` (asking for a password through the desktop's
    /// dialog if needed), then opens it (`mountVolume` in app.js).
    pub(super) fn mount_volume(&self, id: &str) {
        let volume = self
            .volume_monitor()
            .volumes()
            .into_iter()
            .find(|volume| VolumeFacts::from_volume(volume).id() == id);
        let Some(volume) = volume else {
            self.show_message("This device is no longer connected.");
            return;
        };
        let operation = gtk::MountOperation::new(Some(self));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let mounted = volume
                    .mount_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await;
                match (mounted, volume.get_mount()) {
                    (Err(error), _) => window.show_message(error.message()),
                    (Ok(()), Some(mount)) => window.navigate_or_report(&mount.root().uri()),
                    (Ok(()), None) => {}
                }
            }
        ));
    }
}
