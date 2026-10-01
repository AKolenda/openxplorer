// SPDX-License-Identifier: AGPL-3.0-only
//! Starting the launcher that restarts the app into an installed update.
//! Ports the `subprocess.Popen` call of the `updateRestart` branch of
//! `dispatch` in `v2.0.0:desktop/winspace.py`.

use super::process::start_in_new_session;
use super::UpdateError;

/// The only command a restart runs: the installed launcher, which asks
/// this instance to quit safely, waits for it, then starts the new build
/// (see [`InstanceGuard::require_current`](super::InstanceGuard::require_current)).
pub const RESTART_COMMAND: [&str; 2] = ["/usr/bin/openxplorer", "--restart"];

/// Starts the restart launcher: [`SessionLauncher`] in the app, a
/// recording double in tests, which must never start the real launcher.
pub trait RestartLauncher {
    /// Starts `argv` without waiting for it.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Io`] if it cannot start.
    fn launch(&self, argv: &[&str]) -> Result<(), UpdateError>;
}

/// Starts the launcher in a session of its own, so it outlives this
/// process, as Python's `start_new_session=True`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionLauncher;

impl RestartLauncher for SessionLauncher {
    fn launch(&self, argv: &[&str]) -> Result<(), UpdateError> {
        start_in_new_session(argv, gio::SubprocessFlags::NONE)?;
        Ok(())
    }
}
