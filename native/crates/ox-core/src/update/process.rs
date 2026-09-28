// SPDX-License-Identifier: AGPL-3.0-only
//! Running the package tools and starting the restart launcher, with GIO's
//! subprocess API. Ports the `subprocess.run` and `subprocess.Popen` calls
//! of `desktop/updater.py` and `desktop/winspace.py`.

use std::ffi::{OsStr, OsString};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gio::prelude::*;

use super::{CommandOutput, PackageCommand, PackageManager, UpdateError};

/// Runs the real package tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemPackageManager;

impl PackageManager for SystemPackageManager {
    fn run(&self, command: &PackageCommand) -> Result<CommandOutput, UpdateError> {
        run_to_completion(&command.argv(), command.time_limit())
    }
}

/// Runs `argv` with the parent's standard input, as Python's
/// `subprocess.run(..., capture_output=True)`, and collects its output.
/// With a `time_limit`, a command still running then is killed.
///
/// # Errors
///
/// [`UpdateError::Io`] if it cannot start or be waited for, and
/// [`UpdateError::PackageToolTimedOut`] if it was killed at its limit.
pub(super) fn run_to_completion(
    argv: &[OsString],
    time_limit: Option<Duration>,
) -> Result<CommandOutput, UpdateError> {
    let program = program_name(argv);
    let arguments: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
    let flags = gio::SubprocessFlags::STDIN_INHERIT
        | gio::SubprocessFlags::STDOUT_PIPE
        | gio::SubprocessFlags::STDERR_PIPE;
    let process = gio::Subprocess::newv(&arguments, flags).map_err(|error| process_error(program, &error))?;
    let cancellable = gio::Cancellable::new();
    let watchdog = time_limit.map(|limit| Watchdog::start(cancellable.clone(), limit));
    let communicated = process.communicate(None, Some(&cancellable));
    drop(watchdog);
    match communicated {
        Ok((stdout, stderr)) => Ok(CommandOutput {
            exit_status: exit_status(&process),
            stdout: text(stdout.as_ref()),
            stderr: text(stderr.as_ref()),
        }),
        Err(_) if cancellable.is_cancelled() => {
            process.force_exit();
            // Reap the killed process; it is already gone either way.
            let _ = process.wait(gio::Cancellable::NONE);
            Err(UpdateError::PackageToolTimedOut {
                program: program.to_string_lossy().into_owned(),
                limit: time_limit.unwrap_or_default(),
            })
        }
        Err(error) => Err(process_error(program, &error)),
    }
}

/// Starts `argv` in a new session and does not wait for it, as Python's
/// `subprocess.Popen(argv, start_new_session=True)`: the launcher outlives
/// this process, which it asks to quit.
///
/// # Errors
///
/// [`UpdateError::Io`] if it cannot start.
pub(super) fn start_in_new_session(
    argv: &[&str],
    stdout: gio::SubprocessFlags,
) -> Result<gio::Subprocess, UpdateError> {
    let launcher = gio::SubprocessLauncher::new(stdout);
    launcher.set_child_setup(|| {
        // Runs in the child between fork and exec. setsid fails only for a
        // process group leader, which a freshly forked child never is.
        let _ = rustix::process::setsid();
    });
    let arguments: Vec<&OsStr> = argv.iter().map(OsStr::new).collect();
    let program = argv.first().copied().unwrap_or_default();
    launcher
        .spawn(&arguments)
        .map_err(|error| process_error(OsStr::new(program), &error))
}

/// Cancels a cancellable when a time limit passes, unless dropped first.
struct Watchdog {
    /// Dropping this wakes the thread, which then ends without cancelling.
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    fn start(cancellable: gio::Cancellable, limit: Duration) -> Self {
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            if stopped.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                cancellable.cancel();
            }
        });
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            // The thread only waits and cancels; it cannot panic.
            let _ = thread.join();
        }
    }
}

/// The exit status, or minus the signal that ended the process.
fn exit_status(process: &gio::Subprocess) -> i32 {
    if process.has_exited() {
        process.exit_status()
    } else {
        -process.term_sig()
    }
}

/// Captured output as text; bytes that are not UTF-8 are replaced.
fn text(bytes: Option<&glib::Bytes>) -> String {
    bytes
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
}

/// The program of `argv`, for error messages.
fn program_name(argv: &[OsString]) -> &OsStr {
    argv.first().map_or(OsStr::new(""), OsString::as_os_str)
}

/// A GIO error starting or waiting for `program`.
fn process_error(program: &OsStr, error: &glib::Error) -> UpdateError {
    UpdateError::io(program, std::io::Error::other(error.message().to_owned()))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn shell(script: &str) -> Vec<OsString> {
        ["/bin/sh", "-c", script].map(OsString::from).to_vec()
    }

    #[test]
    fn output_and_exit_status_are_collected() {
        let output = run_to_completion(&shell("echo out; echo err >&2; exit 3"), None).unwrap();

        assert_eq!(output.exit_status, 3);
        assert_eq!(output.stdout, "out\n");
        assert_eq!(output.stderr, "err\n");
    }

    #[test]
    fn a_command_past_its_time_limit_is_killed() {
        let started = Instant::now();

        let result = run_to_completion(&shell("sleep 30"), Some(Duration::from_millis(100)));

        assert!(matches!(result, Err(UpdateError::PackageToolTimedOut { .. })));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_missing_program_is_an_io_error() {
        let result = run_to_completion(&[OsString::from("/nonexistent/fixture-tool")], None);

        assert!(matches!(result, Err(UpdateError::Io { .. })));
    }

    /// Ported from `desktop/tests/test_updater.py::BridgeTests::test_restart_requires_pending_update_idle_writes_and_fixed_launcher`
    ///
    /// The `start_new_session=True` half: the launcher leads a session of
    /// its own, so it survives this process quitting.
    /// parity: UPD-007
    #[test]
    fn the_launcher_starts_in_a_session_of_its_own() {
        let script = "read -r pid _ _ _ _ session _ < /proc/self/stat; echo \"$pid $session\"";
        let process =
            start_in_new_session(&["/bin/sh", "-c", script], gio::SubprocessFlags::STDOUT_PIPE).unwrap();

        let (stdout, _) = process.communicate(None, gio::Cancellable::NONE).unwrap();

        let report = text(stdout.as_ref());
        let (pid, session) = report.trim().split_once(' ').unwrap();
        assert_eq!(pid, session, "the child leads its own session");
        let own_session = rustix::process::getsid(None).unwrap();
        assert_ne!(session, own_session.as_raw_nonzero().to_string());
    }
}
