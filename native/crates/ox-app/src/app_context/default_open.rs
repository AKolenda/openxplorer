// SPDX-License-Identifier: AGPL-3.0-only
//! Opening a file in its default application, as the user asked for it
//! from a folder, and recording it among the recent files.
//!
//! Ports `resolve_activation` and `launch_default` in
//! `v2.0.0:desktop/winspace.py` over ox-core's [`DefaultOpener`] (OPEN-005 to
//! OPEN-007): the file is queried again on a worker thread, its
//! application is chosen by content type and never `OpenXplorer`, a file on
//! a share is handed over by its local path when it has one, and nothing
//! is ever executed. A file inside a snapshot or backup is refused, so no
//! application can change it in place.
//!
//! Opened files and visited folders also go to the desktop's recently used
//! list, as Dolphin records them (OPEN-025), so the file chooser's Recent
//! and other applications show them, while the desktop's privacy settings
//! allow it ([`super::recent_privacy`]).

use gtk::gio;
use gtk::prelude::*;
#[cfg(test)]
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;
use ox_core::integration::{DefaultOpener, Launcher, OpenTarget, PreparedOpen, UNKNOWN_CONTENT_TYPE};
use ox_core::network::local_path;
use ox_core::transfer::Cancellation;

use super::{recent_entry, AppContext};

impl AppContext {
    /// Opens `entry` in its default application and records it among the
    /// recently opened files, the app's and the desktop's.
    ///
    /// # Errors
    ///
    /// The message to show when it could not be opened: the file is in a
    /// snapshot, is no longer a regular file, has no application or needs
    /// a local path, or the application did not start.
    pub(crate) async fn open_file(&self, entry: &Entry, window: &gtk::Window) -> Result<(), String> {
        let uri = entry.navigation_uri().to_owned();
        let Some(prepared) = self.launch_in_application(uri, window).await? else {
            return Ok(());
        };
        let content_type = prepared.entry.content_type.as_deref();
        self.record_opened(
            recent_entry(&prepared.entry),
            content_type.unwrap_or(UNKNOWN_CONTENT_TYPE),
        );
        Ok(())
    }

    /// Opens the file at `uri` in its default application, never
    /// `OpenXplorer`, without recording it among the recent files: a ZIP
    /// from its browser's "Open in archive manager", where the desktop's
    /// default for ZIPs may be `OpenXplorer` itself (ARC-021), or a
    /// member's private copy.
    ///
    /// # Errors
    ///
    /// The messages of [`AppContext::open_file`].
    pub(crate) async fn open_uri_in_application(
        &self,
        uri: String,
        window: &gtk::Window,
    ) -> Result<(), String> {
        self.launch_in_application(uri, window).await.map(|_| ())
    }

    /// Prepares and launches `uri` as [`AppContext::open_file`] does;
    /// returns what was opened, or `None` when a test recorded the launch.
    async fn launch_in_application(
        &self,
        uri: String,
        window: &gtk::Window,
    ) -> Result<Option<PreparedOpen>, String> {
        self.previous_versions()
            .check_writable(&uri)
            .map_err(|refusal| refusal.to_string())?;
        let opener = DefaultOpener::new(local_path, self.desktop_integration().sandbox());
        let prepared = opener
            .prepare_in_background(uri, Cancellation::new())
            .await
            .map_err(|error| error.to_string())?;
        // Test safety: tests record the file the application would get
        // instead of starting a real application on the developer's desktop.
        #[cfg(test)]
        if let Some(launches) = self.imp().recorded_launches.borrow_mut().as_mut() {
            launches.push(match &prepared.target {
                OpenTarget::LocalPath(path) => ox_core::location::file_uri(path),
                OpenTarget::Uri(uri) => uri.clone(),
            });
            return Ok(None);
        }
        launch(&prepared, window).await?;
        Ok(Some(prepared))
    }
}

/// Adds `uri`, of `content_type`, to the desktop's recently used list,
/// named as `OpenXplorer`'s. Tests add only with a private data folder
/// (native/tools/check.py), never to the user's.
pub(crate) fn add_to_desktop_history(uri: &str, content_type: &str) {
    if !super::desktop_recent_policy().remember || uri.starts_with("admin:") {
        return;
    }
    #[cfg(test)]
    if !gtk::glib::user_data_dir().starts_with(std::env::temp_dir()) {
        return;
    }
    let program = gtk::glib::prgname().unwrap_or_else(|| "openxplorer".into());
    let exec = format!("{program} %u");
    let data = gtk::RecentData::new(None, None, content_type, "OpenXplorer", &exec, &[], false);
    gtk::RecentManager::default().add_full(uri, &data);
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
            let application = crate::integration::installed_application(id)
                .ok_or_else(|| ox_core::i18n::gettext_static(NOT_INSTALLED).to_owned())?;
            let context = WidgetExt::display(window).app_launch_context();
            application
                .launch(&[file], Some(&context))
                .map_err(|_| ox_core::i18n::gettext_static(NOT_ACCEPTED).to_owned())
        }
        Launcher::DesktopPortal => gtk::FileLauncher::new(Some(&file))
            .launch_future(Some(window))
            .await
            .map_err(|error| error.to_string()),
    }
}

/// Why the chosen application could not be found again to launch it.
const NOT_INSTALLED: &str = crate::i18n::message_id("That application is no longer installed.");

/// Why the chosen application did not open the file (`launch_default`).
const NOT_ACCEPTED: &str = crate::i18n::message_id("The application did not accept this file.");

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPEN-025
    #[gtk::test]
    fn an_opened_file_joins_the_desktop_recent_list() {
        if !gtk::glib::user_data_dir().starts_with(std::env::temp_dir()) {
            return;
        }
        let folder = tempfile::tempdir().expect("a temporary folder");
        let uri = ox_core::location::file_uri(&folder.path().join("report.txt"));
        std::fs::write(folder.path().join("report.txt"), "report").expect("the folder is writable");

        add_to_desktop_history(&uri, "text/plain");

        let recent = gtk::RecentManager::default();
        let is_listed = || recent.items().iter().any(|item| item.uri() == uri);
        crate::test_support::harness::wait_until("the desktop to list it", is_listed);
    }
}
