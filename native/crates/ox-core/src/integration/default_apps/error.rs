// SPDX-License-Identifier: AGPL-3.0-only
//! Why a default-application change or status read failed, in the words
//! of `v2.0.0:desktop/desktop_integration.py`.

use std::io;
use std::path::PathBuf;

/// A failure to read or change the default applications. `Display` is
/// the message the Settings card shows.
#[derive(Debug, thiserror::Error)]
pub enum DefaultAppsError {
    /// `xdg-mime` is not installed (INT-010); inside Flatpak, not on the
    /// host.
    #[error(
        "{}",
        crate::i18n::gettext("Install xdg-utils to manage the default file explorer.")
    )]
    XdgUtilsMissing,
    /// `xdg-mime` exited unsuccessfully.
    #[error(
        "{}",
        crate::i18n::gettext("The desktop did not accept the file-association change.")
    )]
    NotAccepted,
    /// `xdg-mime` did not finish within 8 seconds and was stopped.
    #[error(
        "{}",
        crate::i18n::gettext("The desktop took too long to update the default. Try again.")
    )]
    TimedOut,
    /// `xdg-mime` could not be started or waited for; the operating
    /// system's reason follows the program's name.
    #[error("{}", crate::i18n::format_message("xdg-mime could not be run: {error}", &[("error", &.0.to_string())]))]
    CommandFailed(io::Error),
    /// The handler the app would replace is not a plain desktop ID, so
    /// it cannot be put back later (INT-008).
    #[error(
        "{}",
        crate::i18n::gettext("The current desktop handler cannot be safely recorded.")
    )]
    UnrecordableHandler,
    /// After the change, `xdg-mime` does not report the app for every
    /// requested type (INT-008).
    #[error("{}", crate::i18n::gettext("The desktop did not confirm all requested defaults. Check your system’s Default Applications settings."))]
    NotConfirmed,
    /// Restore found no recorded handler to put back (INT-011).
    #[error(
        "{}",
        crate::i18n::gettext("No previous handler was recorded. Choose one in your desktop settings.")
    )]
    NoPreviousHandler,
    /// The record of the previous handlers could not be written, so no
    /// default was changed.
    #[error("{error}: {}", path.display())]
    RecordNotSaved {
        /// `previous-defaults.json`.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}
