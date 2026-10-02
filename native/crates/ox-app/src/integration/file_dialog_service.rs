// SPDX-License-Identifier: AGPL-3.0-only
//! Other applications' Open and Save dialogs: the portal backend the app
//! serves, and the opt-in that sends the dialogs to it.
//!
//! New in the native app (INT-032). The backend object is exported on the
//! application's own bus name during D-Bus registration
//! (`application/file_dialogs.rs`); it stays inert until the user enables
//! "Open and Save dialogs" here, which makes the desktop portal route
//! `FileChooser` calls to the packaged `<app id>.portal` backend. The bus
//! name's existing D-Bus service file starts the app on the first call.
//! Each call opens a window in picker mode
//! ([`crate::window::BrowserWindow::begin_picking`]); the checks on the
//! caller and its options are ox-core's
//! ([`ox_core::integration::FileChooserBus`]).

use ox_core::integration::{DisabledFileDialogs, FileDialogRegistration, PortalRestart};

use super::changes::IntegrationError;
use super::DesktopIntegration;

/// Where Open and Save dialogs go, for the Settings page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileDialogsStatus {
    /// The opt-in can work here (not inside Flatpak).
    pub(crate) is_available: bool,
    /// The user's portal configuration prefers the app.
    pub(crate) is_enabled: bool,
    /// The backend the portal configuration names now, if any.
    pub(crate) backend: Option<String>,
}

impl FileDialogsStatus {
    /// The row's status line.
    pub(crate) fn text(&self) -> String {
        if !self.is_available {
            return "Open and Save dialogs: available in the installed package, not the Flatpak.".to_owned();
        }
        if self.is_enabled {
            return "Open and Save dialogs: OpenXplorer. Apps that use the desktop portal show their file \
                    dialogs here after Apply now or the next login."
                .to_owned();
        }
        match self.backend.as_deref() {
            Some(backend) => format!("Open and Save dialogs: the desktop's ({backend})."),
            None => "Open and Save dialogs: the desktop's.".to_owned(),
        }
    }
}

impl DesktopIntegration {
    /// Sends Open and Save dialogs to the app from the portal's next start.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the configuration cannot be
    /// written, or inside Flatpak.
    pub(crate) async fn enable_file_dialogs(&self) -> Result<String, IntegrationError> {
        self.file_dialogs()
            .run_in_background(FileDialogRegistration::enable)
            .await?;
        self.notify_changed();
        Ok("Open and Save dialogs will use OpenXplorer. Click Apply now, or log out and back in.".to_owned())
    }

    /// Gives Open and Save dialogs back to the desktop.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the configuration cannot be
    /// restored.
    pub(crate) async fn disable_file_dialogs(&self) -> Result<String, IntegrationError> {
        let outcome = self
            .file_dialogs()
            .run_in_background(FileDialogRegistration::disable)
            .await?;
        self.notify_changed();
        Ok(match outcome {
            DisabledFileDialogs::Restored => {
                "Open and Save dialogs are back to the desktop's. Click Apply now, or log out and back in."
            }
            DisabledFileDialogs::LineRemoved => {
                "Removed OpenXplorer's line from your portal settings and kept your other edits. Click \
                 Apply now, or log out and back in."
            }
            DisabledFileDialogs::NotEnabled => "Open and Save dialogs were not using OpenXplorer.",
        }
        .to_owned())
    }

    /// Restarts the desktop portal so the choice applies now.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the portal could not be
    /// restarted.
    pub(crate) async fn apply_file_dialogs(&self) -> Result<String, IntegrationError> {
        let restart = self
            .file_dialogs()
            .run_in_background(FileDialogRegistration::restart_portal)
            .await?;
        self.notify_changed();
        Ok(match restart {
            PortalRestart::Restarted => {
                "The desktop portal restarted. Open and Save dialogs follow the new choice."
            }
            PortalRestart::NotRunning => {
                "The desktop portal is not running as a service that can be restarted. The choice applies \
                 the next time you log in."
            }
        }
        .to_owned())
    }

    /// Where Open and Save dialogs go now, read off the main thread.
    pub(super) async fn file_dialogs_status(&self) -> FileDialogsStatus {
        self.file_dialogs()
            .run_in_background(|registration| FileDialogsStatus {
                is_available: registration.is_available(),
                is_enabled: registration.is_enabled(),
                backend: registration.current_backend(),
            })
            .await
    }

    fn file_dialogs(&self) -> &FileDialogRegistration {
        &self.services().file_dialogs
    }
}
