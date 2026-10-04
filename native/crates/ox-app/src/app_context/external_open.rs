// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a location in the desktop's application for its type, without
//! recording it among the recent files.
//!
//! Web pages (the issue tracker, a web address typed in the address bar)
//! and tools started on files, such as a comparison tool, use it: neither
//! is a file the user opened from a folder, so neither belongs in Recent.
//! The archive browser's Open in archive manager and a member's private
//! copy go through [`AppContext::open_uri_in_application`] instead, which
//! never picks this app.

use gtk::prelude::*;
#[cfg(test)]
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::AppContext;

impl AppContext {
    /// Starts `app` on `uris`, such as a file comparison tool on two
    /// files (OPEN-023).
    ///
    /// # Errors
    ///
    /// GIO's reason when the application did not start.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "tests record the launch on the context instead"
        )
    )]
    pub(crate) fn launch_tool(
        &self,
        app: &gio::AppInfo,
        uris: &[String],
        window: &gtk::Window,
    ) -> Result<(), glib::Error> {
        // Test safety: tests record the launch instead of starting a real
        // application on the developer's desktop.
        #[cfg(test)]
        if let Some(launches) = self.imp().recorded_launches.borrow_mut().as_mut() {
            launches.push(uris.join(" "));
            return Ok(());
        }
        let files: Vec<gio::File> = uris.iter().map(|uri| gio::File::for_uri(uri)).collect();
        let context = WidgetExt::display(window).app_launch_context();
        app.launch(&files, Some(&context))
    }

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
    pub(crate) fn record_tool_launch(
        &self,
        tool: ox_core::integration::DiskTool,
        target: &std::path::Path,
    ) -> bool {
        let mut launches = self.imp().recorded_launches.borrow_mut();
        let Some(launches) = launches.as_mut() else {
            return false;
        };
        launches.push(format!("{tool:?} {}", target.display()));
        true
    }
}
