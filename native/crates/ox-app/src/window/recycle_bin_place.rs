// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin's row in the sidebar follows what is in it (SIDE-025).
//!
//! A directory monitor on GIO's `trash:///` says when its items may have
//! changed; they are then listed as the Recycle Bin page lists them. The
//! count draws the row full or empty and enables its Empty Recycle Bin.
//! Changes are gathered for a moment, so deleting many files lists the
//! Recycle Bin a few times rather than once per file; one listing runs at
//! a time, off the main loop, and a change during it lists again after
//! it. Only the Recycle Bin's own row is redrawn, and only when the count
//! changed.

use std::cell::{Cell, OnceCell};
use std::time::Duration;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::location::TRASH_URI;
use ox_core::ops::list_recycle_bin;
use ox_core::transfer::Cancellation;

use super::{sidebar, BrowserWindow};

/// How long changes to the Recycle Bin are gathered before it is listed.
const SETTLE_TIME: Duration = Duration::from_millis(300);

/// The watch on the Recycle Bin of one window.
#[derive(Debug, Default)]
pub(super) struct RecycleBinWatch {
    /// The watch on `trash:///`, whose monitor is made off the GTK thread.
    watch: OnceCell<crate::folder_view::watch::Watch>,
    /// A listing is due once the changes settle.
    scheduled: Cell<bool>,
    /// A listing runs.
    counting: Cell<bool>,
    /// The Recycle Bin changed while a listing ran, so it is listed again.
    stale: Cell<bool>,
}

impl BrowserWindow {
    /// Counts the Recycle Bin now and again whenever it changes. Without
    /// a trash backend the row stays empty.
    ///
    /// The monitor is made off the GTK thread, as a folder's is
    /// ([`crate::folder_view::watch`]): for `trash:///` `GVfs` answers over
    /// D-Bus and looks at the trash folder of every mount, so a share that
    /// stopped answering held the whole app, an open file dialog
    /// included, each time a window opened.
    pub(super) fn watch_recycle_bin(&self) {
        let watch = crate::folder_view::watch::watch_folder(
            TRASH_URI,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.recycle_bin_changed()
            ),
        );
        let _ = self.imp().recycle_bin_watch.watch.set(watch);
        self.count_recycle_bin();
    }

    /// The Recycle Bin row's count, `None` while a count runs, for tests.
    #[cfg(test)]
    pub(super) fn recycle_bin_row_count(&self) -> Option<u32> {
        let imp = self.imp();
        (!imp.recycle_bin_watch.counting.get()).then(|| imp.trash_items.get())
    }

    /// Lists the Recycle Bin once its changes settle.
    fn recycle_bin_changed(&self) {
        let watch = &self.imp().recycle_bin_watch;
        if watch.scheduled.replace(true) {
            return;
        }
        glib::timeout_add_local_once(
            SETTLE_TIME,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().recycle_bin_watch.scheduled.set(false);
                    window.count_recycle_bin();
                }
            ),
        );
    }

    /// Reads the Recycle Bin's item count and redraws its row when it
    /// changed; while a reading runs, another follows it.
    fn count_recycle_bin(&self) {
        let watch = &self.imp().recycle_bin_watch;
        if watch.counting.replace(true) {
            watch.stale.set(true);
            return;
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                loop {
                    window.imp().recycle_bin_watch.stale.set(false);
                    // Listing is what the Recycle Bin page shows; GVfs's
                    // trash::item-count can lag behind it.
                    let items = list_recycle_bin(&Cancellation::new()).await;
                    let count = items.map_or(0, |items| u32::try_from(items.len()).unwrap_or(u32::MAX));
                    if window.imp().trash_items.replace(count) != count {
                        let [_, _, bin] = sidebar::recent_and_bin_entries(count);
                        window.sidebar().replace_entry(bin);
                    }
                    if !window.imp().recycle_bin_watch.stale.get() {
                        break;
                    }
                }
                window.imp().recycle_bin_watch.counting.set(false);
            }
        ));
    }
}
