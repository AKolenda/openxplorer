// SPDX-License-Identifier: AGPL-3.0-only
//! Changing the defaults on request: Make `OpenXplorer` default, Restore
//! previous, the ZIP handler, and enabling, testing and disabling Show in
//! folder.
//!
//! Ports the `desktopDefault`, `desktopRestore`, `zipDefault`,
//! `zipRestore`, `revealEnable`, `revealDisable` and `revealTest`
//! branches of `dispatch` in `desktop/winspace.py` (INT-008, INT-009,
//! INT-011, INT-012, INT-015, INT-016), with the toasts of
//! `changeDefault` and `changeZipDefault` in `desktop/ui/app.js`. Each
//! change runs off the main thread; the rules (record before replacing,
//! a later choice wins, never replace a foreign override) are ox-core's.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{
    DefaultApps, DefaultAppsError, DisabledReveal, RegistrationFailed, RestoreScope, RevealError,
    RevealRegistration, ZipAssociation, BUS_NAME, OBJECT_PATH,
};
use ox_core::location::file_uri;

use super::DesktopIntegration;

/// How long the Show in folder test waits for the answer, as in the
/// Python app.
const TEST_TIMEOUT_MS: i32 = 5000;

/// Why a change to the defaults or Show in folder failed. `Display` is
/// the message the window shows.
#[derive(Debug, thiserror::Error)]
pub(crate) enum IntegrationError {
    /// Reading or changing a default handler failed.
    #[error(transparent)]
    Defaults(#[from] DefaultAppsError),
    /// Writing or removing the Show in folder files failed.
    #[error(transparent)]
    Reveal(#[from] RevealError),
    /// The `FileManager1` object could not be exported.
    #[error(transparent)]
    Registration(#[from] RegistrationFailed),
    /// The test needs the service to own the name first (`revealTest`).
    #[error(
        "OpenXplorer does not own Show in folder yet. Close other file managers, or log out and back in \
         after enabling."
    )]
    NotOwner,
    /// The test request was not answered.
    #[error("{0}")]
    TestFailed(glib::Error),
    /// The application is not on the session bus, which the service needs.
    #[error("Show in folder needs the desktop session bus.")]
    NoApplication,
}

/// The options sent with Make `OpenXplorer` default (INT-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MakeDefaultChoice {
    /// Whether ZIP files open in the app too; off by default.
    pub(crate) zip: ZipAssociation,
    /// Whether Show in folder is enabled too; on by default.
    pub(crate) show_in_folder: bool,
}

/// What a change did, for its toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeOutcome {
    /// It did what was asked; the text is the toast.
    Done(&'static str),
    /// The handlers changed, but Show in folder could not be enabled.
    ShowInFolderFailed(String),
}

impl ChangeOutcome {
    /// The toast (`changeDefault`).
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Done(message) => (*message).to_owned(),
            Self::ShowInFolderFailed(error) => {
                format!("File handlers updated, but Show in folder setup failed: {error}")
            }
        }
    }
}

impl DesktopIntegration {
    /// Makes the app the default for folders and SMB links, and ZIP files
    /// when chosen, then enables Show in folder when chosen
    /// (`desktopDefault`).
    ///
    /// # Errors
    ///
    /// The [`DefaultAppsError`] of the change; nothing was enabled then.
    pub(crate) async fn make_default(
        &self,
        choice: MakeDefaultChoice,
    ) -> Result<ChangeOutcome, IntegrationError> {
        let defaults = &self.services().defaults;
        defaults
            .run_in_background(move |defaults| defaults.make_default(choice.zip))
            .await?;
        if choice.show_in_folder {
            if let Err(error) = self.enable_show_in_folder().await {
                return Ok(ChangeOutcome::ShowInFolderFailed(error.to_string()));
            }
        }
        Ok(ChangeOutcome::Done(
            "Requested associations updated. Review each status below.",
        ))
    }

    /// Puts every recorded handler back where the app is still the
    /// default, then turns Show in folder off (`desktopRestore`).
    ///
    /// # Errors
    ///
    /// The [`DefaultAppsError`] of the restore, when nothing is recorded
    /// or the desktop refuses; Show in folder stays as it was then.
    pub(crate) async fn restore_previous(&self) -> Result<ChangeOutcome, IntegrationError> {
        let defaults = &self.services().defaults;
        defaults
            .run_in_background(|defaults| defaults.restore(RestoreScope::Everything))
            .await?;
        self.disable_show_in_folder().await?;
        Ok(ChangeOutcome::Done("Previous recorded handlers restored."))
    }

    /// Makes the app the default for every ZIP type (`zipDefault`).
    ///
    /// # Errors
    ///
    /// The [`DefaultAppsError`] of the change.
    pub(crate) async fn make_zip_default(&self) -> Result<ChangeOutcome, IntegrationError> {
        let defaults = &self.services().defaults;
        defaults.run_in_background(DefaultApps::make_zip_default).await?;
        Ok(ChangeOutcome::Done("ZIP files now open in OpenXplorer."))
    }

    /// Gives ZIP files back to their recorded handler (`zipRestore`).
    ///
    /// # Errors
    ///
    /// The [`DefaultAppsError`] of the restore.
    pub(crate) async fn restore_zip(&self) -> Result<ChangeOutcome, IntegrationError> {
        let defaults = &self.services().defaults;
        defaults
            .run_in_background(|defaults| defaults.restore(RestoreScope::ZipOnly))
            .await?;
        Ok(ChangeOutcome::Done("Previous ZIP handlers restored."))
    }

    /// Writes the two Show in folder session files and starts answering
    /// `FileManager1` (`enable_reveal`).
    ///
    /// # Errors
    ///
    /// The [`RevealError`] of the files, such as a foreign override, or a
    /// service that cannot start.
    pub(crate) async fn enable_show_in_folder(&self) -> Result<(), IntegrationError> {
        let reveal = &self.services().reveal;
        reveal.run_in_background(RevealRegistration::enable).await?;
        self.start_file_manager_service()
    }

    /// Removes the unmodified Show in folder files and stops answering
    /// `FileManager1` (`disable_reveal`). Files the user changed are kept
    /// and listed in the result.
    ///
    /// # Errors
    ///
    /// The [`RevealError`] of a file that cannot be removed; the service
    /// keeps answering then.
    pub(crate) async fn disable_show_in_folder(&self) -> Result<DisabledReveal, IntegrationError> {
        let reveal = &self.services().reveal;
        let disabled = reveal.run_in_background(RevealRegistration::disable).await?;
        self.stop_file_manager_service();
        Ok(disabled)
    }

    /// Sends `ShowFolders` for the home folder through the bus, as a
    /// browser's Show in folder would (`revealTest`). It checks
    /// `FileManager1`, not Brave or its portal.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::NotOwner`] unless this app owns the name, and
    /// [`IntegrationError::TestFailed`] when the call is not answered.
    pub(crate) async fn test_show_in_folder(&self) -> Result<ChangeOutcome, IntegrationError> {
        if !self.owns_file_manager() {
            return Err(IntegrationError::NotOwner);
        }
        let connection = self.session_connection().ok_or(IntegrationError::NoApplication)?;
        let home = file_uri(&glib::home_dir());
        let arguments = (vec![home], String::new()).to_variant();
        let call = connection.call_future(
            Some(BUS_NAME),
            OBJECT_PATH,
            BUS_NAME,
            "ShowFolders",
            Some(&arguments),
            None,
            gio::DBusCallFlags::NONE,
            TEST_TIMEOUT_MS,
        );
        call.await.map_err(IntegrationError::TestFailed)?;
        Ok(ChangeOutcome::Done("Test request sent through FileManager1."))
    }
}
