// SPDX-License-Identifier: AGPL-3.0-only
//! A folder the user may not write to (CMD-011).
//!
//! Each time the free space is read, the window also asks GIO, off the
//! main thread, whether the user may write to the folder itself: a folder
//! without write permission takes no new items, so New and Paste are off
//! there. A reply for a folder the tab has left is dropped.
//!
//! A drive mounted read-only is not singled out (DEV-015): as in Windows
//! Explorer, the commands stay on and a write that fails shows the
//! system's error.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::BrowserWindow;
use crate::locations::Page;

/// Whether GIO says the user may not write into the local folder `file`:
/// false when it may, or when GIO cannot tell. Only a local folder is
/// asked, where GIO answers with access(2); a remote backend may guess
/// from permission bits that do not apply to the user.
async fn is_unwritable_folder(file: &gio::File) -> bool {
    if !file.is_native() {
        return false;
    }
    let info = file
        .query_info_future(
            gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::LOW,
        )
        .await;
    info.is_ok_and(|info| {
        info.has_attribute(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE)
            && !info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE)
    })
}

impl BrowserWindow {
    /// Asks whether the user may not write to the active tab's folder, and
    /// shows the answer.
    pub(super) fn refresh_drive_access(&self) {
        let folder = self.current_uri().filter(|uri| Page::from_uri(uri).is_none());
        let Some(uri) = folder else {
            self.show_folder_access(None);
            return;
        };
        let file = gio::File::for_uri(&uri);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let unwritable = is_unwritable_folder(&file).await;
                if window.current_uri().as_deref() == Some(uri.as_str()) {
                    window.show_folder_access(unwritable.then_some(uri));
                }
            }
        ));
    }

    /// Records that the user may not write to the folder `unwritable`, or
    /// that nothing says so, and updates the commands when that changed.
    fn show_folder_access(&self, unwritable: Option<String>) {
        if *self.imp().unwritable_folder.borrow() == unwritable {
            return;
        }
        self.imp().unwritable_folder.replace(unwritable);
        self.update_file_commands();
    }

    /// Whether the user may not write to the folder `uri`, as last read.
    pub(crate) fn is_unwritable_folder(&self, uri: &str) -> bool {
        self.imp().unwritable_folder.borrow().as_deref() == Some(uri)
    }
}
