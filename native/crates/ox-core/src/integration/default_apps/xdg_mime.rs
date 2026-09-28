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

/// The program that reads and sets the default handlers.
const XDG_MIME: &str = "xdg-mime";

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
        self.run(&query_command(mime_type))
    }

    fn set_default_handler(&self, handler: &DesktopId, mime_type: MimeType) -> Result<(), DefaultAppsError> {
        self.run(&change_command(handler, mime_type)).map(drop)
    }
}

/// `xdg-mime query default <type>`: asks for the default handler of
/// `mime_type`.
fn query_command(mime_type: MimeType) -> HostCommand {
    HostCommand::new(XDG_MIME)
        .arg("query")
        .arg("default")
        .arg(mime_type.as_str())
}

/// `xdg-mime default <handler> <type>`: makes `handler` the default of
/// `mime_type`. The handler comes first, as `xdg-mime` expects.
fn change_command(handler: &DesktopId, mime_type: MimeType) -> HostCommand {
    HostCommand::new(XDG_MIME)
        .arg("default")
        .arg(handler.as_str())
        .arg(mime_type.as_str())
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::*;
    use crate::integration::host_command::argv;

    /// One way running `xdg-mime` can fail, and the message the Settings
    /// card shows for it.
    struct FailureCase {
        failure: CommandFailure,
        message: &'static str,
    }

    /// Ported from the `run` double of `desktop/tests/test_rc3.py::DefaultsTests::setUp`,
    /// which checked `args[1:3] == ['query', 'default']`.
    /// parity: INT-010, SAFE-020
    #[test]
    fn the_query_asks_xdg_mime_for_the_default_of_one_type() {
        let query = query_command(MimeType::Directory).to_command(Sandbox::Host);

        assert_eq!(argv(&query), ["xdg-mime", "query", "default", "inode/directory"]);
    }

    /// Ported from the `run` double of `desktop/tests/test_rc3.py::DefaultsTests::setUp`,
    /// which took the handler from `args[2]` and the type from `args[3]`.
    /// parity: INT-008, SAFE-020
    #[test]
    fn the_change_names_the_handler_before_the_type() {
        let change = change_command(&DesktopId::openxplorer(), MimeType::Zip).to_command(Sandbox::Host);

        assert_eq!(
            argv(&change),
            [
                "xdg-mime",
                "default",
                "io.winspace.Development.desktop",
                "application/zip"
            ]
        );
    }

    /// parity: INT-010
    #[test]
    fn each_failure_of_xdg_mime_has_the_python_apps_message() {
        let cases = [
            FailureCase {
                failure: CommandFailure::NotInstalled,
                message: "Install xdg-utils to manage the default file explorer.",
            },
            FailureCase {
                failure: CommandFailure::Failed(ExitStatus::from_raw(4 << 8)),
                message: "The desktop did not accept the file-association change.",
            },
            FailureCase {
                failure: CommandFailure::TimedOut,
                message: "The desktop took too long to update the default. Try again.",
            },
            FailureCase {
                failure: CommandFailure::Io(io::Error::from(io::ErrorKind::PermissionDenied)),
                message: "xdg-mime could not be run: permission denied",
            },
        ];
        for case in cases {
            let expected = case.message;

            let error = DefaultAppsError::from(case.failure);

            assert_eq!(error.to_string(), expected);
        }
    }
}
