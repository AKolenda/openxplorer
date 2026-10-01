// SPDX-License-Identifier: AGPL-3.0-only
//! Open in Terminal: a local terminal in a checked folder.
//!
//! Ports `desktop/terminal_integration.py` (OPEN-017, OPEN-018, OPEN-020).
//! Only a location crosses from the window; the terminal program comes
//! from the system folders, the folder is checked against fresh metadata
//! and the file system, and nothing derived from a file name is ever run.
//! For a folder on an SMB share the terminal is a local shell in its
//! mounted path, not an SSH session on the server. Inside Flatpak the
//! host's terminal is found under `/run/host` and started on the host.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `emulator` | Which terminal: [`find_terminal`], [`Terminal`] |
//! | `preference` | The desktop's configured terminal: [`desktop_terminal`] |
//! | `directory` | Which folder: [`prepare_directory`], [`checked_directory`] |
//! | `launch` | Starting it: [`launch_terminal`] |
//! | `command` | A chosen program run in it: [`command_in_terminal`] |
//! | `error` | [`TerminalError`] |

mod command;
mod directory;
mod emulator;
mod error;
mod launch;
mod preference;

use std::future::Future;

pub use command::{command_in_terminal, HOLD_SCRIPT};
pub use directory::{checked_directory, prepare_directory, DirectoryChecks, PreparedDirectory};
pub use emulator::{find_terminal, ExecutableSearch, Terminal, TerminalKind, SYSTEM_PATH};
pub use error::TerminalError;
pub(crate) use launch::INHERITED_TERMINAL_VARIABLES;
pub use launch::{launch_terminal, terminal_arguments, LaunchedTerminal};
pub use preference::{desktop_terminal, DesktopTerminalConfig};

use super::sandbox::Sandbox;
use super::worker::on_worker;
use crate::transfer::Cancellation;

/// Prepares the folder of `uri`, finds the terminal and starts it, on a
/// worker thread: the whole Open in Terminal request of the Python app's
/// `openTerminal` operation. Cancelling `cancel` stops it before the
/// terminal starts.
///
/// # Errors
///
/// The future resolves to the errors of [`prepare_directory`],
/// [`find_terminal`] and [`launch_terminal`], or to
/// [`TerminalError::Cancelled`].
pub fn open_terminal_in_background<C>(
    uri: String,
    checks: C,
    sandbox: Sandbox,
    cancel: Cancellation,
) -> impl Future<Output = Result<LaunchedTerminal, TerminalError>> + 'static
where
    C: DirectoryChecks + Send + 'static,
{
    on_worker(move || {
        let prepared = prepare_directory(&uri, &checks, &cancel)?;
        let preferred = desktop_terminal(&DesktopTerminalConfig::of_session());
        let terminal = find_terminal(&ExecutableSearch::for_sandbox(sandbox).preferring(preferred))?;
        if cancel.is_cancelled() {
            return Err(TerminalError::Cancelled);
        }
        launch_terminal(&prepared, &terminal, sandbox)
    })
}
