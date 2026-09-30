// SPDX-License-Identifier: AGPL-3.0-only
//! The free space of the current folder's volume in the status bar, as
//! Dolphin shows it: "12.4 GB free", with "12.4 GB free out of 100 GB
//! (88% used)" as its tooltip (VIEW-053). It is read when a folder
//! starts listing and again whenever a listing ends, so F5 refreshes it,
//! and hidden on landing pages and where GIO cannot tell.

use gtk::{gio, glib};
use ox_core::format;

use super::landing::{measure_capacity, Capacity};
use super::BrowserWindow;
use crate::locations::Page;

impl Capacity {
    /// The status bar's text: "12.4 GB free".
    pub(super) fn free_text(self) -> String {
        format!("{} free", format::pretty_bytes(self.free))
    }

    /// The status bar's tooltip: "12.4 GB free out of 100 GB (88% used)".
    pub(super) fn free_tooltip(self) -> String {
        let free = format::pretty_bytes(self.free);
        let size = format::pretty_bytes(self.size);
        let used = (self.used_share() * 100.0).round();
        format!("{free} free out of {size} ({used}% used)")
    }
}

impl BrowserWindow {
    /// Reads the free space of the active tab's volume into the status
    /// bar, or hides it on a landing page. A reply for a folder the tab
    /// has left is dropped.
    pub(super) fn refresh_free_space(&self) {
        let folder = self.current_uri().filter(|uri| Page::from_uri(uri).is_none());
        let Some(uri) = folder else {
            self.status_bar().show_free_space(None);
            return;
        };
        let file = gio::File::for_uri(&uri);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let capacity = measure_capacity(&file).await;
                if window.current_uri().as_deref() == Some(uri.as_str()) {
                    window.status_bar().show_free_space(capacity);
                }
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-053
    #[test]
    fn the_status_bar_says_how_much_room_is_left() {
        let gigabyte = 1 << 30;
        let capacity = Capacity {
            size: 100 * gigabyte,
            free: 12 * gigabyte,
        };
        assert_eq!(capacity.free_text(), "12.0 GB free");
        assert_eq!(capacity.free_tooltip(), "12.0 GB free out of 100 GB (88% used)");
    }
}
