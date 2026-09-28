// SPDX-License-Identifier: AGPL-3.0-only
//! Starting the terminal in the prepared folder.
//!
//! Ports `terminal_argv` and `launch_terminal` in
//! `desktop/terminal_integration.py` (OPEN-020). The terminal gets its
//! program path and, where it has one, its working-directory option;
//! never `-c`, `-e`, a shell, a `cd` command or a script string, so a
//! folder named like code stays a name.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::Duration;

use std::os::unix::process::{CommandExt, ExitStatusExt};

use super::directory::{checked_directory, PreparedDirectory};
use super::emulator::Terminal;
use super::TerminalError;
use crate::integration::host_command::{wait_for_exit, HostCommand};
use crate::integration::sandbox::Sandbox;

/// How long a terminal may take to fail before it counts as started.
const START_CHECK: Duration = Duration::from_millis(250);

/// Variables that would make GNOME Terminal open a tab in the terminal
/// the app was started from, instead of a new window.
const INHERITED_TERMINAL_VARIABLES: [&str; 2] = ["GNOME_TERMINAL_SCREEN", "GNOME_TERMINAL_SERVICE"];

/// A terminal that was started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedTerminal {
    /// The terminal's name, for "Opened <terminal> in <path>".
    pub terminal: &'static str,
    /// The folder it opened in.
    pub path: PathBuf,
    /// The folder's location, as prepared.
    pub uri: String,
}

/// The program and arguments that start `terminal` in `directory`.
///
/// # Errors
///
/// As [`checked_directory`].
pub fn terminal_arguments(terminal: &Terminal, directory: &Path) -> Result<Vec<OsString>, TerminalError> {
    let directory = checked_directory(directory)?;
    let mut arguments = vec![terminal.executable().as_os_str().to_owned()];
    if let Some(option) = terminal.kind().working_directory_option() {
        let mut argument = OsString::from(option);
        argument.push(directory.as_os_str());
        arguments.push(argument);
    }
    Ok(arguments)
}

/// Starts `terminal` in the prepared folder, on the host (OPEN-020).
///
/// The terminal starts in its own process group with the folder as its
/// working directory and `PWD`, without GNOME Terminal's inherited
/// variables, so it opens a new window. A terminal that fails within a
/// quarter of a second is reported; one that keeps running is waited for
/// on a background thread, so it never becomes a zombie.
///
/// # Errors
///
/// As [`checked_directory`], [`TerminalError::CouldNotStart`] for a
/// terminal that exits unsuccessfully at once, and [`TerminalError::Io`]
/// when it cannot be started.
pub fn launch_terminal(
    prepared: &PreparedDirectory,
    terminal: &Terminal,
    sandbox: Sandbox,
) -> Result<LaunchedTerminal, TerminalError> {
    let directory = checked_directory(&prepared.path)?;
    let command = terminal_command(terminal, &directory)?;
    let child = command
        .to_command(sandbox)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // A process group of its own, so signals for OpenXplorer's group,
        // such as Ctrl+C in the terminal it was started from, never reach
        // the new terminal. The standard library has no safe `setsid`.
        .process_group(0)
        .spawn()
        .map_err(|error| TerminalError::Io {
            path: terminal.executable().to_owned(),
            error,
        })?;
    confirm_start(child, terminal)?;
    Ok(LaunchedTerminal {
        terminal: terminal.label(),
        path: directory,
        uri: prepared.uri.clone(),
    })
}

/// The command that starts `terminal` in `directory`, which must already
/// be checked.
fn terminal_command(terminal: &Terminal, directory: &Path) -> Result<HostCommand, TerminalError> {
    let mut arguments = terminal_arguments(terminal, directory)?.into_iter();
    let program = arguments.next().expect("the arguments start with the program");
    let mut command = HostCommand::new(program)
        .current_dir(directory)
        .env("PWD", directory);
    for argument in arguments {
        command = command.arg(argument);
    }
    for variable in INHERITED_TERMINAL_VARIABLES {
        command = command.env_remove(variable);
    }
    Ok(command)
}

/// Reports a terminal that failed at once, and otherwise leaves it
/// running with a thread that reaps it when it exits.
fn confirm_start(mut child: Child, terminal: &Terminal) -> Result<(), TerminalError> {
    let status = wait_for_exit(&mut child, START_CHECK).map_err(|error| TerminalError::Io {
        path: terminal.executable().to_owned(),
        error,
    })?;
    match status {
        Some(status) if !status.success() => Err(TerminalError::CouldNotStart {
            terminal: terminal.label(),
            code: status
                .code()
                .unwrap_or_else(|| -status.signal().unwrap_or_default()),
        }),
        Some(_) => Ok(()),
        None => {
            // If no thread can be started, the terminal still runs; it is
            // reaped when OpenXplorer exits.
            let _ = thread::Builder::new()
                .name("openxplorer-terminal".to_owned())
                .spawn(move || child.wait());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::integration::host_command::argv;
    use crate::integration::terminal::TerminalKind;

    #[test]
    fn the_terminal_gets_a_new_window_environment_and_starts_in_the_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal =
            Terminal::new("/usr/bin/gnome-terminal".into(), TerminalKind::GnomeTerminal).expect("absolute");

        let command = terminal_command(&terminal, &directory)
            .expect("command")
            .to_command(Sandbox::Host);

        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let environment: Vec<_> = command.get_envs().collect();
        assert!(environment.contains(&(OsStr::new("PWD"), Some(directory.as_os_str()))));
        for variable in INHERITED_TERMINAL_VARIABLES {
            assert!(environment.contains(&(OsStr::new(variable), None)), "{variable}");
        }
    }

    #[test]
    fn inside_flatpak_the_terminal_starts_on_the_host_in_the_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal = Terminal::new("/usr/bin/konsole".into(), TerminalKind::Konsole).expect("absolute");

        let command = terminal_command(&terminal, &directory)
            .expect("command")
            .to_command(Sandbox::Flatpak);

        let folder_text = directory.to_string_lossy();
        let expected: Vec<OsString> = [
            "flatpak-spawn".to_owned(),
            "--host".to_owned(),
            format!("--directory={folder_text}"),
            format!("--env=PWD={folder_text}"),
            "/usr/bin/konsole".to_owned(),
            format!("--workdir={folder_text}"),
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert_eq!(argv(&command), expected);
    }
}
