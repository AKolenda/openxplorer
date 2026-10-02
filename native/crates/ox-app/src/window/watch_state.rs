// SPDX-License-Identifier: AGPL-3.0-only
//! Telling the user when the folder shown is not watched for changes
//! (VIEW-056). A folder GIO cannot monitor, such as an SMB share through
//! `GVfs`, is listed again every network refresh interval of Settings
//! (30 seconds, 1 minute or 5 minutes) instead, and the status bar says
//! "Not updated live", its tooltip how often the folder is checked.
//! Background tabs keep their changes for when they are shown, as before
//! (TAB-056).

use std::time::Duration;

use gtk::glib;
use gtk::subclass::prelude::*;

use super::BrowserWindow;
use crate::folder_view::watch::Watch;

/// How the status bar names a refresh interval of `seconds`.
fn interval_text(seconds: u32) -> String {
    match seconds {
        60 => "1 minute".to_owned(),
        seconds if seconds % 60 == 0 => format!("{} minutes", seconds / 60),
        seconds => format!("{seconds} seconds"),
    }
}

impl BrowserWindow {
    /// The network refresh interval of Settings, in seconds.
    fn fallback_seconds(&self) -> u32 {
        self.context().settings_data().preferences.network_interval
    }

    /// Has `watch` list its folder again every refresh interval should the
    /// folder turn out not to be watchable, and the status bar say so.
    pub(super) fn follow_watch_health(&self, watch: &Watch) {
        let interval = Duration::from_secs(u64::from(self.fallback_seconds()));
        watch.when_unwatched(
            interval,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.update_status()
            ),
        );
    }

    /// Shows in the status bar whether the active tab's folder is watched.
    pub(super) fn show_watch_state(&self) {
        let unwatched = {
            let session = self.imp().session.borrow();
            let watch = session.active().and_then(|tab| tab.watch.as_ref());
            watch.is_some_and(|watch| !watch.is_live())
        };
        let interval = unwatched.then(|| interval_text(self.fallback_seconds()));
        self.status_bar().show_unwatched(interval.as_deref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_read_as_settings_names_them() {
        assert_eq!(interval_text(30), "30 seconds");
        assert_eq!(interval_text(60), "1 minute");
        assert_eq!(interval_text(300), "5 minutes");
    }
}
