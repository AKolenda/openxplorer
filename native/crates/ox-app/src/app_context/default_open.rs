// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a file in its default application, as the user asked for it
//! from a folder, and recording it among the recent files.
//!
//! Ports `resolve_activation` and `launch_default` in
//! `desktop/winspace.py` over ox-core's [`DefaultOpener`] (OPEN-005 to
//! OPEN-007): the file is queried again on a worker thread, its
//! application is chosen by content type and never `OpenXplorer`, a file on
//! a share is handed over by its local path when it has one, and nothing
//! is ever executed. A file inside a snapshot or backup is refused, so no
//! application can change it in place.

use gtk::gio;
use gtk::prelude::*;
#[cfg(test)]
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;
use ox_core::integration::{DefaultOpener, Launcher, OpenTarget, PreparedOpen};
use ox_core::network::local_path;
use ox_core::transfer::Cancellation;

use super::{recent_entry, AppContext};

impl AppContext {
    /// Opens `entry` in its default application and records it among the
    /// recently opened files.
    ///
    /// # Errors
    ///
    /// The message to show when it could not be opened: the file is in a
    /// snapshot, is no longer a regular file, has no application or needs
    /// a local path, or the application did not start.
    pub(crate) async fn open_file(&self, entry: &Entry, window: &gtk::Window) -> Result<(), String> {
        let uri = entry.navigation_uri().to_owned();
        // Test safety: tests record the file instead of starting a real
        // application on the developer's desktop.
        #[cfg(test)]
        if let Some(launches) = self.imp().recorded_launches.borrow_mut().as_mut() {
            launches.push(uri);
            return Ok(());
        }
        self.previous_versions()
            .check_writable(&uri)
            .map_err(|refusal| refusal.to_string())?;
        let opener = DefaultOpener::new(local_path, self.desktop_integration().sandbox());
        let prepared = opener
            .prepare_in_background(uri, Cancellation::new())
            .await
            .map_err(|error| error.to_string())?;
        launch(&prepared, window).await?;
        self.remember_open(recent_entry(&prepared.entry));
        Ok(())
    }
}

/// Starts the application `prepared` names on its file, with the
/// window's launch context for startup notification and focus (INT-023).
async fn launch(prepared: &PreparedOpen, window: &gtk::Window) -> Result<(), String> {
    let file = match &prepared.target {
        OpenTarget::LocalPath(path) => gio::File::for_path(path),
        OpenTarget::Uri(uri) => gio::File::for_uri(uri),
    };
    match &prepared.launcher {
        Launcher::Application { id, .. } => {
            let application = gio::AppInfo::all()
                .into_iter()
                .find(|application| application.id().as_deref() == Some(id.as_str()))
                .ok_or_else(|| NOT_INSTALLED.to_owned())?;
            let context = WidgetExt::display(window).app_launch_context();
            application
                .launch(&[file], Some(&context))
                .map_err(|_| NOT_ACCEPTED.to_owned())
        }
        Launcher::DesktopPortal => gtk::FileLauncher::new(Some(&file))
            .launch_future(Some(window))
            .await
            .map_err(|error| error.to_string()),
    }
}

/// Why the chosen application could not be found again to launch it.
const NOT_INSTALLED: &str = "That application is no longer installed.";

/// Why the chosen application did not open the file (`launch_default`).
const NOT_ACCEPTED: &str = "The application did not accept this file.";
