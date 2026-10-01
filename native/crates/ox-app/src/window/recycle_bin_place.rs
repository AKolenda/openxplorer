// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin's row in the sidebar follows what is in it (SIDE-025).
//!
//! A directory monitor on GIO's `trash:///` says when its items may have
//! changed; they are then listed as the Recycle Bin page lists them. The
//! count draws the row full or empty and enables its Empty Recycle Bin. Counting runs off the main
//! loop, and the places are redrawn only when the count changed.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::TRASH_URI;
use ox_core::ops::list_recycle_bin;
use ox_core::transfer::Cancellation;

use super::BrowserWindow;

impl BrowserWindow {
    /// Counts the Recycle Bin now and again whenever it changes. Without
    /// a trash backend the row stays empty.
    pub(super) fn watch_recycle_bin(&self) {
        let trash = gio::File::for_uri(TRASH_URI);
        if let Ok(monitor) = trash.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            monitor.connect_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, _, _, _| window.count_recycle_bin()
            ));
            let _ = self.imp().trash_monitor.set(monitor);
        }
        self.count_recycle_bin();
    }

    /// Reads the Recycle Bin's item count and redraws the places when it
    /// changed.
    fn count_recycle_bin(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // Listing is what the Recycle Bin page shows; GVfs's
                // trash::item-count can lag behind it.
                let items = list_recycle_bin(&Cancellation::new()).await;
                let count = items.map_or(0, |items| u32::try_from(items.len()).unwrap_or(u32::MAX));
                if window.imp().trash_items.replace(count) != count {
                    window.render_places();
                }
            }
        ));
    }
}
