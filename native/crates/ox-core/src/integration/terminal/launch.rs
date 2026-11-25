// SPDX-License-Identifier: AGPL-3.0-only
//! Starting the terminal in the prepared folder.
//!
//! Ports `terminal_argv` and `launch_terminal` in
//! `v2.0.0:desktop/terminal_integration.py` (OPEN-020). The terminal gets its
//! program path and, where it has one, its working-directory option;
//! never `-c`, `-e`, a shell, a `cd` command or a script string, so a
//! folder named like code stays a name.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::Duration;

use std::os::unix::process::{CommandExt, ExitStatusExt};

use super::directory::{checked_directory, PreparedDirectory};
use super::emulator::{Terminal, TerminalKind, SYSTEM_PATH};
use super::TerminalError;
use crate::integration::host_command::{wait_for_exit, HostCommand};
use crate::integration::sandbox::Sandbox;

/// How long a terminal may take to fail before it counts as started.
const START_CHECK: Duration = Duration::from_millis(250);

/// Variables that would make GNOME Terminal open a tab in the terminal
/// the app was started from, instead of a new window.
pub(crate) const INHERITED_TERMINAL_VARIABLES: [&str; 2] =
    ["GNOME_TERMINAL_SCREEN", "GNOME_TERMINAL_SERVICE"];

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

/// The program and arguments that start `terminal` in `directory`: its
/// program, its leading options and its working-directory option.
///
/// # Errors
///
/// As [`checked_directory`].
pub fn terminal_arguments(terminal: &Terminal, directory: &Path) -> Result<Vec<OsString>, TerminalError> {
    let directory = checked_directory(directory)?;
    let mut arguments = vec![terminal.executable().as_os_str().to_owned()];
    arguments.extend(terminal.kind().leading_options().iter().map(OsString::from));
    arguments.extend(directory_arguments(terminal.kind(), &directory));
    Ok(arguments)
}

/// The arguments that tell a terminal of `kind` to start in `directory`:
/// none for the `XTerm` family, the folder joined to an option ending in
/// `=`, or the option and the folder as two arguments.
pub(super) fn directory_arguments(kind: TerminalKind, directory: &Path) -> Vec<OsString> {
    match kind.working_directory_option() {
        None => Vec::new(),
        Some(option) if option.ends_with('=') => {
            let mut argument = OsString::from(option);
            argument.push(directory.as_os_str());
            vec![argument]
        }
        Some(option) => vec![OsString::from(option), directory.as_os_str().to_owned()],
    }
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
    let command = terminal_command(terminal, &directory, sandbox)?;
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
/// be checked, from `sandbox`.
///
/// `xdg-terminal-exec` is a shell script that finds the terminal on
/// `PATH`, and it runs in the folder, so on the host it gets `PATH`
/// without its empty and relative folders, which would find a program in
/// the browsed folder; the absolute ones stay, so a terminal the user
/// installed in, say, `~/.local/bin` still starts. Inside Flatpak the
/// host command starts from the host session's environment, never the
/// sandbox's `PATH`, which is left alone.
fn terminal_command(
    terminal: &Terminal,
    directory: &Path,
    sandbox: Sandbox,
) -> Result<HostCommand, TerminalError> {
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
    if terminal.kind() == TerminalKind::XdgTerminalExec && sandbox == Sandbox::Host {
        command = command.env("PATH", absolute_search_path(env::var_os("PATH").as_deref()));
    }
    Ok(command)
}

/// `path`, a `PATH` value, with only its absolute folders; the
/// [`SYSTEM_PATH`] folders when it has none or is unset, since an empty
/// `PATH` means the working directory and an unset one lets the shell
/// choose a default that may hold it too.
fn absolute_search_path(path: Option<&OsStr>) -> OsString {
    let folders: Vec<PathBuf> = path
        .map(|path| {
            env::split_paths(path)
                .filter(|folder| folder.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    if folders.is_empty() {
        return OsString::from(SYSTEM_PATH.join(":"));
    }
    // The folders came out of a `PATH`, so none holds a `:`.
    env::join_paths(folders).unwrap_or_else(|_| OsString::from(SYSTEM_PATH.join(":")))
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

    #[test]
    fn the_terminal_gets_a_new_window_environment_and_starts_in_the_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal =
            Terminal::new("/usr/bin/gnome-terminal".into(), TerminalKind::GnomeTerminal).expect("absolute");

        let command = terminal_command(&terminal, &directory, Sandbox::Host)
            .expect("command")
            .to_command(Sandbox::Host);

        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let environment: Vec<_> = command.get_envs().collect();
        assert!(environment.contains(&(OsStr::new("PWD"), Some(directory.as_os_str()))));
        for variable in INHERITED_TERMINAL_VARIABLES {
            assert!(environment.contains(&(OsStr::new(variable), None)), "{variable}");
        }
    }

    /// `xdg-terminal-exec` is told the folder with `--dir=` and also starts
    /// in it, since a terminal whose entry has no `X-TerminalArgDir` starts
    /// in its working directory.
    ///
    /// parity: OPEN-019, OPEN-020
    #[test]
    fn xdg_terminal_exec_starts_in_the_folder_and_is_told_it() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal = Terminal::new("/usr/bin/xdg-terminal-exec".into(), TerminalKind::XdgTerminalExec)
            .expect("absolute");

        let command = terminal_command(&terminal, &directory, Sandbox::Host)
            .expect("command")
            .to_command(Sandbox::Host);

        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let mut expected = OsString::from("--dir=");
        expected.push(directory.as_os_str());
        assert_eq!(command.get_args().collect::<Vec<_>>(), [expected.as_os_str()]);
        assert_eq!(terminal.label(), "Terminal");
    }

    /// `xdg-terminal-exec` runs in the browsed folder and finds the
    /// terminal on `PATH`, so it never gets an empty or relative `PATH`
    /// folder, which would find a program in the browsed folder; inside
    /// Flatpak the host's own `PATH` is left alone.
    ///
    /// parity: OPEN-019, OPEN-020, SAFE-015
    #[test]
    fn xdg_terminal_exec_never_searches_the_browsed_folder() {
        let cases: [(Option<&str>, &str); 5] = [
            (
                Some("/home/ada/.local/bin::bin:./tools:/usr/bin:"),
                "/home/ada/.local/bin:/usr/bin",
            ),
            (Some("/usr/local/bin:/usr/bin"), "/usr/local/bin:/usr/bin"),
            (Some(""), "/usr/bin:/bin:/usr/local/bin"),
            (Some(".:bin"), "/usr/bin:/bin:/usr/local/bin"),
            (None, "/usr/bin:/bin:/usr/local/bin"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                absolute_search_path(path.map(OsStr::new)),
                OsStr::new(expected),
                "{path:?}"
            );
        }

        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal = Terminal::new("/usr/bin/xdg-terminal-exec".into(), TerminalKind::XdgTerminalExec)
            .expect("absolute");
        let host = terminal_command(&terminal, &directory, Sandbox::Host)
            .expect("command")
            .to_command(Sandbox::Host);
        let flatpak = terminal_command(&terminal, &directory, Sandbox::Flatpak)
            .expect("command")
            .to_command(Sandbox::Flatpak);
        let konsole = Terminal::new("/usr/bin/konsole".into(), TerminalKind::Konsole).expect("absolute");
        let other = terminal_command(&konsole, &directory, Sandbox::Host)
            .expect("command")
            .to_command(Sandbox::Host);

        let path = host
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value))
            .flatten()
            .expect("PATH is set");
        assert!(!path.is_empty());
        assert!(
            env::split_paths(path).all(|folder| folder.is_absolute()),
            "{path:?}"
        );
        assert!(
            !argv(&flatpak)
                .iter()
                .any(|argument| argument.to_string_lossy().starts_with("--env=PATH")),
            "{flatpak:?}"
        );
        assert!(
            !other.get_envs().any(|(name, _)| name == "PATH"),
            "other terminals inherit it"
        );
    }

    #[test]
    fn inside_flatpak_the_terminal_starts_on_the_host_in_the_folder() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let directory = checked_directory(folder.path()).expect("folder");
        let terminal = Terminal::new("/usr/bin/konsole".into(), TerminalKind::Konsole).expect("absolute");

        let command = terminal_command(&terminal, &directory, Sandbox::Flatpak)
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
