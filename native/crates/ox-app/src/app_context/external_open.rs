// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a location in the desktop's application for its type, without
//! recording it among the recent files.
//!
//! The archive browser uses it for Open in archive manager and for the
//! private read-only copy of a member (`open` and `open_archive_preview`
//! in `desktop/winspace.py`): neither is a file the user opened from a
//! folder, so neither belongs in Recent.

use gtk::prelude::*;
#[cfg(test)]
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::AppContext;

impl AppContext {
    /// Opens `uri` in the desktop's default application for its type.
    /// `on_error` hears GIO's reason when it could not be opened.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "tests record the launch on the context instead"
        )
    )]
    pub(crate) fn open_uri(
        &self,
        uri: &str,
        window: &gtk::Window,
        on_error: impl FnOnce(glib::Error) + 'static,
    ) {
        // Test safety: tests record the location instead of starting a
        // real application on the developer's desktop.
        #[cfg(test)]
        if let Some(launches) = self.imp().recorded_launches.borrow_mut().as_mut() {
            launches.push(uri.to_owned());
            return;
        }
        let launch_context = WidgetExt::display(window).app_launch_context();
        let uri = uri.to_owned();
        glib::spawn_future_local(async move {
            let launched = gio::AppInfo::launch_default_for_uri_future(&uri, Some(&launch_context)).await;
            if let Err(error) = launched {
                on_error(error);
            }
        });
    }

    /// Records a disk tool started on `target` as `<tool> <target>`, when
    /// the test records launches; true if it did.
    #[cfg(test)]
    pub(crate) fn record_tool_launch(&self, tool: ox_core::integration::DiskTool, target: &std::path::Path) -> bool {
        let mut launches = self.imp().recorded_launches.borrow_mut();
        let Some(launches) = launches.as_mut() else {
            return false;
        };
        launches.push(format!("{tool:?} {}", target.display()));
        true
    }
}
