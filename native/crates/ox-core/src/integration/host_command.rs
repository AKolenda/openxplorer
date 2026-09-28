// SPDX-License-Identifier: AGPL-3.0-only
//! Running a desktop program on the host: `xdg-mime` for the default
//! applications and the terminal emulator for Open in Terminal.
//!
//! Ports the `subprocess` calls of `desktop/desktop_integration.py`
//! (`DesktopIntegration._run`) and `desktop/terminal_integration.py`
//! (`launch_terminal`). A command is always an argument list; no shell
//! ever parses it, so nothing in a folder name can become code.
//!
//! Inside a Flatpak sandbox the host's programs are not on the sandbox's
//! file system, and a program run there would change the sandbox's copy
//! of the user's files instead of the host's. There, a command runs on the
//! host through `flatpak-spawn --host`, which needs the app's
//! `--talk-name=org.freedesktop.Flatpak` permission.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::sandbox::Sandbox;

/// The Flatpak tool that runs a command on the host.
const FLATPAK_SPAWN: &str = "flatpak-spawn";

/// How often a running command is checked for having exited.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A program with its arguments, working directory and environment
/// changes, to be run on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostCommand {
    program: OsString,
    arguments: Vec<OsString>,
    directory: Option<PathBuf>,
    environment: Vec<(OsString, OsString)>,
    removed_environment: Vec<OsString>,
}

/// Why a command did not produce its output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandFailure {
    /// The program, or `flatpak-spawn` inside Flatpak, is not installed.
    #[error("the program is not installed")]
    NotInstalled,
    /// The program exited unsuccessfully.
    #[error("the program exited with {0}")]
    Failed(ExitStatus),
    /// The program did not finish in time and was stopped.
    #[error("the program did not finish in time")]
    TimedOut,
    /// Starting or waiting for the program failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl HostCommand {
    /// Runs `program`, found on `PATH` unless it is an absolute path.
    pub(crate) fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            arguments: Vec::new(),
            directory: None,
            environment: Vec::new(),
            removed_environment: Vec::new(),
        }
    }

    /// Adds one argument.
    pub(crate) fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.arguments.push(argument.as_ref().to_owned());
        self
    }

    /// Starts the program in `directory`.
    pub(crate) fn current_dir(mut self, directory: &Path) -> Self {
        self.directory = Some(directory.to_owned());
        self
    }

    /// Sets an environment variable for the program.
    pub(crate) fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        let variable = (name.as_ref().to_owned(), value.as_ref().to_owned());
        self.environment.push(variable);
        self
    }

    /// Removes an environment variable that the program would inherit.
    pub(crate) fn env_remove(mut self, name: impl AsRef<OsStr>) -> Self {
        self.removed_environment.push(name.as_ref().to_owned());
        self
    }

    /// The process to start: the program itself on the host, or
    /// `flatpak-spawn --host` running it inside Flatpak.
    pub(crate) fn to_command(&self, sandbox: Sandbox) -> Command {
        match sandbox {
            Sandbox::Host => self.to_host_command(),
            Sandbox::Flatpak => self.to_flatpak_spawn_command(),
        }
    }

    /// Runs the command to completion and returns its standard output
    /// without surrounding whitespace, as `subprocess.run(...,
    /// capture_output=True, timeout=..., check=True).stdout.strip()` does.
    /// A command still running after `timeout` is killed.
    ///
    /// # Errors
    ///
    /// [`CommandFailure`] when the program is missing, fails, times out or
    /// cannot be started or waited for.
    pub(crate) fn output_within(
        &self,
        sandbox: Sandbox,
        timeout: Duration,
    ) -> Result<String, CommandFailure> {
        let mut child = self
            .to_command(sandbox)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(CommandFailure::from_spawn_error)?;
        let mut stdout = child
            .stdout
            .take()
            .expect("standard output was requested as a pipe");
        // Read on another thread, so a command that fills the pipe cannot
        // stall the wait below.
        let reader = thread::spawn(move || {
            let mut output = String::new();
            stdout.read_to_string(&mut output).map(|_| output)
        });
        let Some(status) = wait_for_exit(&mut child, timeout)? else {
            child.kill()?;
            child.wait()?;
            return Err(CommandFailure::TimedOut);
        };
        if !status.success() {
            return Err(CommandFailure::Failed(status));
        }
        let output = reader.join().expect("reading a pipe does not panic")?;
        Ok(output.trim().to_owned())
    }

    /// The program run directly.
    fn to_host_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.arguments);
        if let Some(directory) = &self.directory {
            command.current_dir(directory);
        }
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        for name in &self.removed_environment {
            command.env_remove(name);
        }
        command
    }

    /// The program run on the host by `flatpak-spawn --host`.
    ///
    /// The host command starts from the host session's environment, not
    /// the sandbox's, and receives only the variables passed with
    /// `--env=`; the removed variables therefore never reach it anyway.
    fn to_flatpak_spawn_command(&self) -> Command {
        let mut command = Command::new(FLATPAK_SPAWN);
        command.arg("--host");
        if let Some(directory) = &self.directory {
            command.arg(prefixed("--directory=", directory.as_os_str()));
        }
        for (name, value) in &self.environment {
            let mut assignment = prefixed("--env=", name);
            assignment.push("=");
            assignment.push(value);
            command.arg(assignment);
        }
        command.arg(&self.program).args(&self.arguments);
        command
    }
}

impl CommandFailure {
    /// A failure to start the program: a missing program is reported as
    /// [`CommandFailure::NotInstalled`].
    fn from_spawn_error(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotInstalled
        } else {
            Self::Io(error)
        }
    }
}

/// Waits up to `timeout` for `child` to exit; `None` if it is still
/// running then. The child is left running.
///
/// # Errors
///
/// The error of checking the child's state.
pub(crate) fn wait_for_exit(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(EXIT_POLL_INTERVAL);
    }
}

/// `prefix` followed by `value`, without converting `value` to UTF-8.
fn prefixed(prefix: &str, value: &OsStr) -> OsString {
    let mut text = OsString::from(prefix);
    text.push(value);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The program and arguments `command` starts.
    fn argv(command: &Command) -> Vec<&OsStr> {
        let mut argv = vec![command.get_program()];
        argv.extend(command.get_args());
        argv
    }

    fn terminal_in_projects() -> HostCommand {
        HostCommand::new("/usr/bin/gnome-terminal")
            .arg("--working-directory=/home/demo/Projects")
            .current_dir(Path::new("/home/demo/Projects"))
            .env("PWD", "/home/demo/Projects")
            .env_remove("GNOME_TERMINAL_SCREEN")
    }

    #[test]
    fn on_the_host_the_program_runs_directly() {
        let command = terminal_in_projects().to_command(Sandbox::Host);

        assert_eq!(
            argv(&command),
            [
                "/usr/bin/gnome-terminal",
                "--working-directory=/home/demo/Projects"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/home/demo/Projects")));
        let environment: Vec<_> = command.get_envs().collect();
        assert!(environment.contains(&(OsStr::new("PWD"), Some(OsStr::new("/home/demo/Projects")))));
        assert!(environment.contains(&(OsStr::new("GNOME_TERMINAL_SCREEN"), None)));
    }

    #[test]
    fn inside_flatpak_the_program_runs_on_the_host_through_flatpak_spawn() {
        let command = terminal_in_projects().to_command(Sandbox::Flatpak);

        assert_eq!(
            argv(&command),
            [
                "flatpak-spawn",
                "--host",
                "--directory=/home/demo/Projects",
                "--env=PWD=/home/demo/Projects",
                "/usr/bin/gnome-terminal",
                "--working-directory=/home/demo/Projects",
            ]
        );
    }

    #[test]
    fn output_is_captured_and_trimmed() {
        let command = HostCommand::new("printf").arg("  folder-handler.desktop\n");

        let output = command.output_within(Sandbox::Host, Duration::from_secs(8));

        assert_eq!(output.expect("printf runs"), "folder-handler.desktop");
    }

    #[test]
    fn missing_failing_and_slow_programs_are_told_apart() {
        let missing = HostCommand::new("/nonexistent/openxplorer-test-program");
        let failing = HostCommand::new("false");
        let slow = HostCommand::new("sleep").arg("5");

        let timeout = Duration::from_millis(200);
        let missing = missing.output_within(Sandbox::Host, timeout);
        let failing = failing.output_within(Sandbox::Host, timeout);
        let slow = slow.output_within(Sandbox::Host, timeout);

        assert!(
            matches!(missing, Err(CommandFailure::NotInstalled)),
            "{missing:?}"
        );
        assert!(matches!(failing, Err(CommandFailure::Failed(_))), "{failing:?}");
        assert!(matches!(slow, Err(CommandFailure::TimedOut)), "{slow:?}");
    }
}
