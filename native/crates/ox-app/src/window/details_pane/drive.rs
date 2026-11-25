// SPDX-License-Identifier: AGPL-3.0-only
//! The rows of a drive selected on This PC, as Windows 11's Details pane
//! lists a drive: Type, Space used, Space free, Total size and File system
//! ([`super::content::drive_rows`]), worded as the drive's Properties
//! words them ([`crate::properties::mount_at`]).
//!
//! The drive is read off the main thread: the mount table and udev's
//! records on a worker thread, else, for a drive outside the mount table
//! (a share mounted by `GVfs`), its space from GIO at low priority. An
//! answer for a drive no longer selected is dropped, and the same drive
//! is not read again when the pane is redrawn for it.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::content::{drive_rows, Property};
use super::DetailsPane;

impl DetailsPane {
    /// Reads the drive at `uri`, or forgets the one before for `None`,
    /// and shows its rows once read.
    pub(super) fn follow_drive(&self, uri: Option<&str>) {
        let imp = self.imp();
        if imp.drive.borrow().as_deref() == uri {
            return;
        }
        imp.drive.replace(uri.map(str::to_owned));
        imp.drive_generation
            .set(imp.drive_generation.get().wrapping_add(1));
        imp.drive_rows.borrow_mut().clear();
        let Some(uri) = uri else {
            return;
        };
        let generation = imp.drive_generation.get();
        let pane = self.downgrade();
        let uri = uri.to_owned();
        glib::spawn_future_local(async move {
            let rows = read_drive(&uri).await;
            let Some(pane) = pane.upgrade() else {
                return;
            };
            if pane.imp().drive_generation.get() != generation || rows.is_empty() {
                return;
            }
            pane.imp().drive_rows.replace(rows);
            pane.redraw_properties();
        });
    }

    /// The drive's rows read so far.
    pub(super) fn shown_drive_rows(&self) -> Vec<Property> {
        self.imp().drive_rows.borrow().clone()
    }
}

/// The rows of the drive at `uri`: from its mount, as its Properties
/// reads it, else its space alone; none when neither can be read.
async fn read_drive(uri: &str) -> Vec<Property> {
    let file = gio::File::for_uri(uri);
    let mounted = file.clone();
    let mount = gio::spawn_blocking(move || crate::properties::mount_at(&mounted))
        .await
        .ok()
        .flatten();
    if let Some(mount) = mount {
        let Some((free, size)) = mount.space else {
            return Vec::new();
        };
        let kind = mount.drive_type.label();
        return drive_rows(Some(&kind), free, size, Some(&mount.filesystem));
    }
    let space = file
        .query_filesystem_info_future(crate::volumes::SPACE_ATTRIBUTES, glib::Priority::LOW)
        .await
        .ok();
    space
        .as_ref()
        .and_then(crate::volumes::space_of)
        .map(|(free, size)| drive_rows(None, free, size, None))
        .unwrap_or_default()
}
