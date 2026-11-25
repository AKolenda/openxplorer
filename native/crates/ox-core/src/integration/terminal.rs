// SPDX-License-Identifier: AGPL-3.0-only
//! Open in Terminal: a local terminal in a checked folder.
//!
//! Ports `v2.0.0:desktop/terminal_integration.py` (OPEN-017, OPEN-018, OPEN-020).
//! Only a location crosses from the window; the terminal program comes
//! from the system folders, the folder is checked against fresh metadata
//! and the file system, and nothing derived from a file name is ever run.
//! For a folder on an SMB share the terminal is a local shell in its
//! mounted path, not an SSH session on the server. Inside Flatpak the
//! host's terminal is found under `/run/host` and started on the host.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `emulator` | Which terminal: [`find_terminal`], [`find_command_terminal`], [`Terminal`] |
//! | `preference` | The desktop's configured terminals: [`desktop_terminals`] |
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
pub use emulator::{
    find_command_terminal, find_terminal, ExecutableSearch, Terminal, TerminalKind, SYSTEM_PATH,
};
pub use error::TerminalError;
pub(crate) use launch::INHERITED_TERMINAL_VARIABLES;
pub use launch::{launch_terminal, terminal_arguments, LaunchedTerminal};
pub use preference::{desktop_terminals, DesktopTerminalConfig, PreferredTerminal};

use super::sandbox::Sandbox;
use super::worker::on_worker;
use crate::transfer::Cancellation;

/// The terminal of this session (OPEN-018, OPEN-019): [`find_terminal`]
/// among the host's programs as seen from `sandbox`, preferring the
/// terminals the desktop is configured to use. Reads small files and
/// `GSettings`; Open in Terminal calls it on a worker thread.
///
/// # Errors
///
/// As [`find_terminal`].
pub fn session_terminal(sandbox: Sandbox) -> Result<Terminal, TerminalError> {
    find_terminal(&session_search(sandbox))
}

/// The terminal of this session to run a program in (OPEN-019, OPEN-020):
/// [`find_command_terminal`], which never picks `xdg-terminal-exec`, with
/// the same preferences as [`session_terminal`].
///
/// # Errors
///
/// As [`find_command_terminal`].
pub fn session_command_terminal(sandbox: Sandbox) -> Result<Terminal, TerminalError> {
    find_command_terminal(&session_search(sandbox))
}

/// The host's programs as seen from `sandbox`, preferring the terminals
/// the desktop is configured to use.
fn session_search(sandbox: Sandbox) -> ExecutableSearch {
    let preferred = desktop_terminals(&DesktopTerminalConfig::of_session(sandbox));
    ExecutableSearch::for_sandbox(sandbox).preferring(preferred)
}

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
        let terminal = session_terminal(sandbox)?;
        if cancel.is_cancelled() {
            return Err(TerminalError::Cancelled);
        }
        launch_terminal(&prepared, &terminal, sandbox)
    })
}
