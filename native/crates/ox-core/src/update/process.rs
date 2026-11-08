// SPDX-License-Identifier: AGPL-3.0-only
//! Running the package tools and starting the restart launcher, with GIO's
//! subprocess API. Ports the `subprocess.run` and `subprocess.Popen` calls
//! of `v2.0.0:desktop/updater.py` and `v2.0.0:desktop/winspace.py`.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::iter;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gio::prelude::*;

use super::{CommandOutput, PackageCommand, PackageManager, UpdateError};
use crate::transfer::Cancellation;

/// How often the cancellation watcher looks whether the user cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// The line that asks the update helper to stop before APT starts.
pub(super) const CANCEL_LINE: &[u8] = b"cancel\n";

/// Runs the real package tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemPackageManager;

impl PackageManager for SystemPackageManager {
    fn run(&self, command: &PackageCommand) -> Result<CommandOutput, UpdateError> {
        run_command(
            command.program(),
            &command.arguments(),
            command.time_limit(),
            command.reads_input(),
        )
    }

    /// Streams the output of a command without a time limit (the update
    /// helper); a command with one runs as [`run`](PackageManager::run)
    /// does.
    fn run_reporting(
        &self,
        command: &PackageCommand,
        on_line: &mut dyn FnMut(&str),
        cancel: &Cancellation,
    ) -> Result<CommandOutput, UpdateError> {
        if command.time_limit().is_some() {
            let output = self.run(command)?;
            output.stdout.lines().for_each(&mut *on_line);
            return Ok(output);
        }
        run_streaming(command.program(), &command.arguments(), on_line, cancel)
    }
}

/// Runs `program` with `arguments`, handing each line of its standard
/// output to `on_line` as it comes, and writes [`CANCEL_LINE`] to its
/// standard input once `cancel` is cancelled. It is never killed.
///
/// # Errors
///
/// [`UpdateError::Io`] if it cannot start or be waited for.
fn run_streaming(
    program: &'static str,
    arguments: &[OsString],
    on_line: &mut dyn FnMut(&str),
    cancel: &Cancellation,
) -> Result<CommandOutput, UpdateError> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| UpdateError::io(program, error))?;
    let watcher = CancelWatcher::start(child.stdin.take(), cancel);
    let errors = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            // A broken pipe only ends what there is to read.
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        })
    });
    let mut stdout = Vec::new();
    if let Some(output) = child.stdout.take() {
        let mut reader = BufReader::new(output);
        let mut line = Vec::new();
        while matches!(reader.read_until(b'\n', &mut line), Ok(count) if count > 0) {
            let text = String::from_utf8_lossy(&line);
            on_line(text.trim_end_matches('\n'));
            stdout.extend_from_slice(&line);
            line.clear();
        }
    }
    let status = child.wait().map_err(|error| UpdateError::io(program, error));
    watcher.stop();
    let stderr = errors.and_then(|thread| thread.join().ok()).unwrap_or_default();
    let status = status?;
    Ok(CommandOutput {
        exit_status: status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or_default()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Writes [`CANCEL_LINE`] to a child's standard input once the user
/// cancels, and closes it when stopped.
struct CancelWatcher {
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CancelWatcher {
    /// Watches `cancel` for the child whose standard input is `input`.
    fn start(input: Option<std::process::ChildStdin>, cancel: &Cancellation) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let thread = input.map(|mut input| {
            let watching = Arc::clone(&stopped);
            let cancel = cancel.clone();
            thread::spawn(move || {
                while !watching.load(Ordering::SeqCst) {
                    if cancel.is_cancelled() {
                        // A helper that already ended no longer reads.
                        let _ = input.write_all(CANCEL_LINE);
                        let _ = input.flush();
                        return;
                    }
                    thread::sleep(CANCEL_POLL);
                }
            })
        });
        Self { stopped, thread }
    }

    /// Ends the watch and waits for its thread.
    fn stop(mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Runs `program` with `arguments`, as Python's `subprocess.run(...,
/// capture_output=True)`, and collects its output. It reads the parent's
/// standard input only if `reads_input`; otherwise it reads `/dev/null`.
/// With a `time_limit`, a command still running then is killed.
///
/// # Errors
///
/// [`UpdateError::Io`] if it cannot start or be waited for, and
/// [`UpdateError::PackageToolTimedOut`] if it was killed at its limit.
pub(super) fn run_command(
    program: &'static str,
    arguments: &[OsString],
    time_limit: Option<Duration>,
    reads_input: bool,
) -> Result<CommandOutput, UpdateError> {
    let argv: Vec<&OsStr> = iter::once(OsStr::new(program))
        .chain(arguments.iter().map(OsString::as_os_str))
        .collect();
    // Without STDIN_INHERIT, GIO gives the command /dev/null.
    let input = if reads_input {
        gio::SubprocessFlags::STDIN_INHERIT
    } else {
        gio::SubprocessFlags::NONE
    };
    let flags = input | gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE;
    let process = gio::Subprocess::newv(&argv, flags).map_err(|error| process_error(program, &error))?;
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
                program,
                limit: time_limit.unwrap_or_default(),
            })
        }
        Err(error) => Err(process_error(program, &error)),
    }
}

/// Starts `argv` in a new session with the subprocess `flags` and does not
/// wait for it, as Python's `subprocess.Popen(argv,
/// start_new_session=True)`: the launcher outlives this process, which it
/// asks to quit.
///
/// # Errors
///
/// [`UpdateError::Io`] if it cannot start.
pub(super) fn start_in_new_session(
    argv: &[&str],
    flags: gio::SubprocessFlags,
) -> Result<gio::Subprocess, UpdateError> {
    let launcher = gio::SubprocessLauncher::new(flags);
    launcher.set_child_setup(|| {
        // Runs in the child between fork and exec. setsid fails only for a
        // process group leader, which a freshly forked child never is.
        let _ = rustix::process::setsid();
    });
    let arguments: Vec<&OsStr> = argv.iter().map(OsStr::new).collect();
    let program = argv.first().copied().unwrap_or_default();
    launcher
        .spawn(&arguments)
        .map_err(|error| process_error(program, &error))
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

/// A GIO error starting or waiting for `program`.
fn process_error(program: &str, error: &glib::Error) -> UpdateError {
    UpdateError::io(program, std::io::Error::other(error.message().to_owned()))
}

#[cfg(test)]
mod tests {
    use rustix::process::Signal;
    use std::time::Instant;

    use super::*;

    /// The shell that runs the test scripts.
    const SHELL: &str = "/bin/sh";

    /// The shell's arguments that run `script`.
    fn script_arguments(script: &str) -> Vec<OsString> {
        ["-c", script].map(OsString::from).to_vec()
    }

    #[test]
    fn output_and_exit_status_are_collected() {
        let arguments = script_arguments("echo out; echo err >&2; exit 3");

        let output = run_command(SHELL, &arguments, None, true).unwrap();

        assert_eq!(output.exit_status, 3);
        assert_eq!(output.stdout, "out\n");
        assert_eq!(output.stderr, "err\n");
    }

    #[test]
    fn a_command_ended_by_a_signal_reports_minus_the_signal_number() {
        let arguments = script_arguments("kill -TERM $$");

        let output = run_command(SHELL, &arguments, None, true).unwrap();

        assert_eq!(output.exit_status, -Signal::TERM.as_raw());
    }

    #[test]
    fn a_command_past_its_time_limit_is_killed() {
        let started = Instant::now();

        let result = run_command(
            SHELL,
            &script_arguments("sleep 30"),
            Some(Duration::from_millis(100)),
            true,
        );

        let error = result.unwrap_err();
        assert!(
            matches!(error, UpdateError::PackageToolTimedOut { program: SHELL, .. }),
            "{error:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_command_that_reads_no_input_gets_dev_null() {
        let arguments = script_arguments("readlink /proc/self/fd/0");

        let output = run_command(SHELL, &arguments, None, false).unwrap();

        assert_eq!(output.stdout.trim(), "/dev/null");
    }

    #[test]
    fn streamed_lines_arrive_one_by_one_with_the_whole_output() {
        let arguments = script_arguments("echo one; echo two >&2; printf 'three'; exit 4");
        let mut lines = Vec::new();

        let output = run_streaming(
            SHELL,
            &arguments,
            &mut |line| lines.push(line.to_owned()),
            &Cancellation::new(),
        )
        .unwrap();

        assert_eq!(lines, ["one", "three"]);
        assert_eq!(output.exit_status, 4);
        assert_eq!(output.stdout, "one\nthree");
        assert_eq!(output.stderr, "two\n");
    }

    /// The update helper hears `cancel` on its standard input, and nothing
    /// else is written there.
    #[test]
    fn a_cancelled_stream_tells_the_command_to_cancel() {
        let arguments = script_arguments("echo ready; read -r line; echo \"got $line\"");
        let cancel = Cancellation::new();
        let mut lines = Vec::new();

        let output = run_streaming(
            SHELL,
            &arguments,
            &mut |line| {
                lines.push(line.to_owned());
                cancel.cancel();
            },
            &cancel,
        )
        .unwrap();

        assert_eq!(lines, ["ready", "got cancel"]);
        assert!(output.is_success());
    }

    #[test]
    fn a_missing_program_is_an_io_error() {
        let result = run_command("/nonexistent/fixture-tool", &[], None, true);

        assert!(matches!(result, Err(UpdateError::Io { .. })));
    }

    /// Ported from `v2.0.0:desktop/tests/test_updater.py::BridgeTests::test_restart_requires_pending_update_idle_writes_and_fixed_launcher`
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
