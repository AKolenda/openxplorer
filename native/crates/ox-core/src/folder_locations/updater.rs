// SPDX-License-Identifier: AGPL-3.0-only
//! Changing a standard folder in `user-dirs.dirs` with
//! `xdg-user-dirs-update`.
//!
//! Ports `FolderLocations._run` in `desktop/folder_locations.py`: the
//! program runs from an argument list, never through a shell, and is
//! stopped after 15 seconds (SAFE-020).

use std::path::Path;
use std::time::Duration;

use super::RelocationError;
use crate::integration::host_command::{CommandFailure, HostCommand};
use crate::integration::Sandbox;
use crate::places::KnownFolder;

/// The freedesktop tool that writes `user-dirs.dirs`.
const XDG_USER_DIRS_UPDATE: &str = "xdg-user-dirs-update";

/// How long `xdg-user-dirs-update` may take, as in the Python app.
pub(super) const UPDATE_TIMEOUT: Duration = Duration::from_secs(15);

/// Writes a standard folder's new path into the configuration.
/// [`XdgUserDirsUpdate`] is the desktop's; tests supply their own.
pub trait UserDirsUpdater: Send + Sync {
    /// Makes `path` the location of `folder`.
    ///
    /// # Errors
    ///
    /// [`RelocationError::UpdaterMissing`] or
    /// [`RelocationError::UpdaterFailed`].
    fn set_folder(&self, folder: KnownFolder, path: &Path) -> Result<(), RelocationError>;
}

/// `xdg-user-dirs-update --set <NAME> <path>`, run on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XdgUserDirsUpdate {
    sandbox: Sandbox,
}

impl XdgUserDirsUpdate {
    /// The tool as reached from the sandbox the app runs in.
    pub fn detect() -> Self {
        Self {
            sandbox: Sandbox::detect(),
        }
    }
}

impl UserDirsUpdater for XdgUserDirsUpdate {
    fn set_folder(&self, folder: KnownFolder, path: &Path) -> Result<(), RelocationError> {
        let command = update_command(folder, path);
        match command.output_within(self.sandbox, UPDATE_TIMEOUT) {
            Ok(_) => Ok(()),
            Err(CommandFailure::NotInstalled) => Err(RelocationError::UpdaterMissing),
            Err(_) => Err(RelocationError::UpdaterFailed),
        }
    }
}

/// The argument list that sets `folder` to `path`.
fn update_command(folder: KnownFolder, path: &Path) -> HostCommand {
    HostCommand::new(XDG_USER_DIRS_UPDATE)
        .arg("--set")
        .arg(folder.xdg_key())
        .arg(path)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::integration::host_command::argv;

    /// A folder name full of shell syntax stays one argument.
    ///
    /// parity: PROP-031, SAFE-020
    #[test]
    fn the_update_runs_without_a_shell_with_the_path_as_one_argument() {
        let path = Path::new("/data/$(reboot); `id` Downloads");

        let command = update_command(KnownFolder::Downloads, path).to_command(Sandbox::Host);

        let expected: [&OsStr; 4] = [
            OsStr::new("xdg-user-dirs-update"),
            OsStr::new("--set"),
            OsStr::new("DOWNLOAD"),
            path.as_os_str(),
        ];
        assert_eq!(argv(&command), expected);
        assert_eq!(UPDATE_TIMEOUT, Duration::from_secs(15));
    }
}
