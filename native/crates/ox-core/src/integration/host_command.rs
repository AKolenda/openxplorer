// SPDX-License-Identifier: AGPL-3.0-only
//! Running a desktop program on the host: `xdg-mime` for the default
//! applications and the terminal emulator for Open in Terminal.
//!
//! Ports the `subprocess` calls of `v2.0.0:desktop/desktop_integration.py`
//! (`DesktopIntegration._run`) and `v2.0.0:desktop/terminal_integration.py`
//! (`launch_terminal`). A command is always an argument list; no shell
//! ever parses it, so nothing in a folder name can become code.
//!
//! Inside a Flatpak sandbox the host's programs are not on the sandbox's
//! file system, and a program run there would change the sandbox's copy
//! of the user's files instead of the host's. There, a command runs on the
//! host through `flatpak-spawn --host`, which needs the app's
//! `--talk-name=org.freedesktop.Flatpak` permission. A command with a time
//! limit runs there under the host's `timeout`, because stopping
//! `flatpak-spawn` does not stop the program it started on the host.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use super::sandbox::Sandbox;

/// The Flatpak tool that runs a command on the host.
const FLATPAK_SPAWN: &str = "flatpak-spawn";

/// The coreutils program that stops a command on the host after a time
/// limit.
const HOST_TIMEOUT: &str = "timeout";

/// How long the host's `timeout` waits after SIGTERM before it sends
/// SIGKILL.
const HOST_KILL_GRACE: Duration = Duration::from_secs(1);

/// How much longer the app waits for `flatpak-spawn` than the host's
/// `timeout` needs to stop the program, so that the host always stops it
/// first.
const FLATPAK_SPAWN_MARGIN: Duration = Duration::from_secs(2);

/// The exit status of `timeout` when the program ran out of time and
/// SIGTERM stopped it.
const TIMEOUT_STOPPED: i32 = 124;

/// The exit status of `timeout` when the program ran out of time and
/// only SIGKILL stopped it (128 + 9).
const TIMEOUT_KILLED: i32 = 137;

/// The exit status of `timeout` when the program is not installed.
const TIMEOUT_NOT_FOUND: i32 = 127;

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
    /// The program is not installed on the host, or `flatpak-spawn` is not
    /// installed in the sandbox.
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
    /// capture_output=True, timeout=..., check=True).stdout.strip()` does:
    /// running the program and collecting its output end after `timeout`.
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
        let wait_limit = app_side_limit(sandbox, timeout);
        let deadline = Instant::now() + wait_limit;
        let mut child = self
            .to_limited_command(sandbox, timeout)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(CommandFailure::from_spawn_error)?;
        let stdout = child
            .stdout
            .take()
            .expect("standard output was requested as a pipe");
        let output = OutputReader::start(stdout);
        let Some(status) = wait_for_exit(&mut child, wait_limit)? else {
            // On the host this stops the program. Inside Flatpak SIGKILL
            // reaches only `flatpak-spawn`, which cannot pass it on, but
            // the host's `timeout` stopped the program before this
            // deadline, so no default changes after the app reports that
            // the desktop took too long.
            child.kill()?;
            child.wait()?;
            return Err(CommandFailure::TimedOut);
        };
        if !status.success() {
            return Err(CommandFailure::from_exit_status(status, sandbox));
        }
        let output = output.finish_by(deadline)?;
        Ok(output.trim().to_owned())
    }

    /// The process that runs the command with a time limit of `timeout`:
    /// the program itself on the host, where the app stops it; inside
    /// Flatpak, [`HostCommand::under_host_timeout`] through
    /// `flatpak-spawn --host`.
    fn to_limited_command(&self, sandbox: Sandbox, timeout: Duration) -> Command {
        match sandbox {
            Sandbox::Host => self.to_host_command(),
            Sandbox::Flatpak => self.under_host_timeout(timeout).to_flatpak_spawn_command(),
        }
    }

    /// This command run by the host's `timeout`, which stops it with
    /// SIGTERM after `timeout` and with SIGKILL one [`HOST_KILL_GRACE`]
    /// later.
    ///
    /// Safety rule "a timed-out change stays stopped"
    /// (`subprocess.run(timeout=8)` in `desktop_integration.py`): SIGKILL
    /// of `flatpak-spawn` cannot be passed on to the host, so only a limit
    /// the host enforces stops a slow `xdg-mime` before the app reports
    /// the timeout.
    fn under_host_timeout(&self, timeout: Duration) -> Self {
        let kill_after = format!("--kill-after={}", HOST_KILL_GRACE.as_secs());
        let seconds = timeout.as_secs_f64().to_string();
        let mut arguments = vec![
            OsString::from(kill_after),
            OsString::from(seconds),
            self.program.clone(),
        ];
        arguments.extend(self.arguments.iter().cloned());
        Self {
            program: OsString::from(HOST_TIMEOUT),
            arguments,
            directory: self.directory.clone(),
            environment: self.environment.clone(),
            removed_environment: self.removed_environment.clone(),
        }
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

    /// The failure an unsuccessful exit `status` means.
    ///
    /// Inside Flatpak the status is that of the host's `timeout`, which
    /// `flatpak-spawn` passes on, and tells a program that ran out of time
    /// or is not installed on the host from one that failed. `timeout`
    /// exits with 127 only when it cannot find the program, and `xdg-mime`
    /// itself exits with 1 to 5, so a missing `xdg-mime` needs no second
    /// query of the host.
    fn from_exit_status(status: ExitStatus, sandbox: Sandbox) -> Self {
        if sandbox == Sandbox::Host {
            return Self::Failed(status);
        }
        match status.code() {
            Some(TIMEOUT_STOPPED | TIMEOUT_KILLED) => Self::TimedOut,
            Some(TIMEOUT_NOT_FOUND) => Self::NotInstalled,
            _ => Self::Failed(status),
        }
    }
}

/// A command's standard output, read to its end on a thread of its own,
/// so that a command that fills the pipe cannot stall the wait for its
/// exit.
struct OutputReader {
    receiver: mpsc::Receiver<io::Result<String>>,
}

impl OutputReader {
    /// Starts reading `stdout`.
    fn start(mut stdout: ChildStdout) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut output = String::new();
            let read = stdout.read_to_string(&mut output).map(|_| output);
            // After a timeout nobody waits for the output any more, and
            // it is not needed.
            let _ = sender.send(read);
        });
        Self { receiver }
    }

    /// The whole output, if the pipe closes before `deadline`.
    ///
    /// A helper that the program left running can hold the pipe open
    /// after the program exits. Python's `run(timeout=8)` limits
    /// collecting the output too, so the wait ends at the same deadline;
    /// the reading thread ends when the helper closes the pipe.
    fn finish_by(self, deadline: Instant) -> Result<String, CommandFailure> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match self.receiver.recv_timeout(remaining) {
            Ok(read) => Ok(read?),
            Err(RecvTimeoutError::Timeout) => Err(CommandFailure::TimedOut),
            Err(RecvTimeoutError::Disconnected) => {
                let lost = io::Error::other("the output of the program was lost");
                Err(CommandFailure::Io(lost))
            }
        }
    }
}

/// How long the app waits for a command limited to `timeout`: exactly
/// that on the host; inside Flatpak long enough for the host's `timeout`
/// to stop the program first, so that its exit status says what
/// happened.
fn app_side_limit(sandbox: Sandbox, timeout: Duration) -> Duration {
    match sandbox {
        Sandbox::Host => timeout,
        Sandbox::Flatpak => timeout + HOST_KILL_GRACE + FLATPAK_SPAWN_MARGIN,
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

/// The program and arguments `command` starts, for the tests of the
/// commands built on [`HostCommand`].
#[cfg(test)]
pub(crate) fn argv(command: &Command) -> Vec<&OsStr> {
    let mut argv = vec![command.get_program()];
    argv.extend(command.get_args());
    argv
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;

    /// An exit code of the program, or of the host's `timeout` inside
    /// Flatpak, and the failure it is reported as.
    struct ExitCase {
        sandbox: Sandbox,
        code: i32,
        expected: &'static str,
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

    #[test]
    fn a_helper_holding_the_output_open_cannot_outlast_the_timeout() {
        let command = HostCommand::new("sh").arg("-c").arg("sleep 30 &");
        let started = Instant::now();

        let output = command.output_within(Sandbox::Host, Duration::from_millis(200));

        assert!(matches!(output, Err(CommandFailure::TimedOut)), "{output:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn inside_flatpak_a_limited_command_runs_under_the_hosts_timeout() {
        let query = HostCommand::new("xdg-mime")
            .arg("query")
            .arg("default")
            .arg("inode/directory");
        let limit = Duration::from_secs(8);

        let on_host = query.to_limited_command(Sandbox::Host, limit);
        let in_flatpak = query.to_limited_command(Sandbox::Flatpak, limit);

        assert_eq!(
            argv(&on_host),
            ["xdg-mime", "query", "default", "inode/directory"]
        );
        assert_eq!(
            argv(&in_flatpak),
            [
                "flatpak-spawn",
                "--host",
                "timeout",
                "--kill-after=1",
                "8",
                "xdg-mime",
                "query",
                "default",
                "inode/directory",
            ]
        );
    }

    #[test]
    fn inside_flatpak_the_hosts_timeout_tells_a_slow_or_missing_program_apart() {
        let cases = [
            ExitCase {
                sandbox: Sandbox::Flatpak,
                code: TIMEOUT_STOPPED,
                expected: "the program did not finish in time",
            },
            ExitCase {
                sandbox: Sandbox::Flatpak,
                code: TIMEOUT_KILLED,
                expected: "the program did not finish in time",
            },
            ExitCase {
                sandbox: Sandbox::Flatpak,
                code: TIMEOUT_NOT_FOUND,
                expected: "the program is not installed",
            },
            ExitCase {
                sandbox: Sandbox::Flatpak,
                code: 4,
                expected: "the program exited with exit status: 4",
            },
            ExitCase {
                sandbox: Sandbox::Host,
                code: TIMEOUT_NOT_FOUND,
                expected: "the program exited with exit status: 127",
            },
        ];
        for case in cases {
            let status = ExitStatus::from_raw(case.code << 8);

            let failure = CommandFailure::from_exit_status(status, case.sandbox);

            assert_eq!(
                failure.to_string(),
                case.expected,
                "{:?} {}",
                case.sandbox,
                case.code
            );
        }
    }

    #[test]
    fn inside_flatpak_the_app_waits_until_the_host_has_stopped_the_program() {
        let timeout = Duration::from_secs(8);

        assert_eq!(app_side_limit(Sandbox::Host, timeout), timeout);
        assert!(app_side_limit(Sandbox::Flatpak, timeout) > timeout + HOST_KILL_GRACE);
    }
}
