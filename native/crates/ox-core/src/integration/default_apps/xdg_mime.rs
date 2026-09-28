// SPDX-License-Identifier: AGPL-3.0-only
//! Reading and setting per-user default handlers with `xdg-mime`.
//!
//! Ports `DesktopIntegration._run` and its `xdg-mime query default` and
//! `xdg-mime default` calls in `desktop/desktop_integration.py`.
//! `xdg-mime` writes only the user's own `mimeapps.list`; the app
//! never runs it with `sudo`.

use std::time::Duration;

use super::record::DesktopId;
use super::DefaultAppsError;
use crate::integration::host_command::{CommandFailure, HostCommand};
use crate::integration::mime_type::MimeType;
use crate::integration::sandbox::Sandbox;

/// How long one `xdg-mime` call may take, as in the Python app.
const XDG_MIME_TIMEOUT: Duration = Duration::from_secs(8);

/// Where the default handler of each MIME type is read and set.
/// [`XdgMime`] is the desktop's; tests supply their own.
pub trait MimeDefaults {
    /// The desktop ID of the default handler for `mime_type`, exactly as
    /// the desktop reports it, or an empty string when there is none.
    ///
    /// # Errors
    ///
    /// A [`DefaultAppsError`] when the desktop cannot be asked.
    fn default_handler(&self, mime_type: MimeType) -> Result<String, DefaultAppsError>;

    /// Makes `handler` the user's default for `mime_type`.
    ///
    /// # Errors
    ///
    /// A [`DefaultAppsError`] when the desktop refuses or cannot be asked.
    fn set_default_handler(&self, handler: &DesktopId, mime_type: MimeType) -> Result<(), DefaultAppsError>;
}

/// The desktop's default handlers, through `xdg-mime`. Inside Flatpak it
/// runs on the host, so it changes the user's real `mimeapps.list` rather
/// than the sandbox's copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XdgMime {
    sandbox: Sandbox,
}

impl XdgMime {
    /// `xdg-mime` as reached from `sandbox`.
    pub fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }

    /// Runs one `xdg-mime` command and returns its trimmed output.
    fn run(self, command: &HostCommand) -> Result<String, DefaultAppsError> {
        command
            .output_within(self.sandbox, XDG_MIME_TIMEOUT)
            .map_err(DefaultAppsError::from)
    }
}

impl MimeDefaults for XdgMime {
    fn default_handler(&self, mime_type: MimeType) -> Result<String, DefaultAppsError> {
        let query = HostCommand::new("xdg-mime")
            .arg("query")
            .arg("default")
            .arg(mime_type.as_str());
        self.run(&query)
    }

    fn set_default_handler(&self, handler: &DesktopId, mime_type: MimeType) -> Result<(), DefaultAppsError> {
        let change = HostCommand::new("xdg-mime")
            .arg("default")
            .arg(handler.as_str())
            .arg(mime_type.as_str());
        self.run(&change).map(drop)
    }
}

impl From<CommandFailure> for DefaultAppsError {
    fn from(failure: CommandFailure) -> Self {
        match failure {
            CommandFailure::NotInstalled => Self::XdgUtilsMissing,
            CommandFailure::Failed(_) => Self::NotAccepted,
            CommandFailure::TimedOut => Self::TimedOut,
            CommandFailure::Io(error) => Self::CommandFailed(error),
        }
    }
}
