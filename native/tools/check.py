#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Run the native checks with a private display, session bus and disposable user data.

The checks run in this order: the parity inventory tests and validation, this
driver's own tests, rustfmt, Clippy with the workspace lints, then every
compiled Rust test executable and the doctests. Each test executable runs under
xvfb-run and dbus-run-session with its own temporary HOME and XDG directories,
so GTK, GIO and settings code never reach the user's display, session bus or
configuration. Every process a test run starts is stopped before its temporary
HOME is deleted, also when the run times out.

The isolation covers the desktop session, not the filesystem: tests can still
reach absolute paths such as /, so they must keep their writes inside
temporary directories.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterator, Sequence
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any

NATIVE = Path(__file__).resolve().parents[1]

# The driver runs cargo and the isolation tools. The Rust tests themselves call
# python3 (settings interop) and mkfifo.
REQUIRED_TOOLS = ('cargo', 'python3', 'dbus-run-session', 'xvfb-run', 'Xvfb', 'xauth', 'mkfifo')

DEFAULT_TEST_TIMEOUT = 180.0
# How long a stop signal may take before the next, stronger one is sent.
STOP_GRACE_SECONDS = 5.0
# How often to look whether a stopped process group has emptied.
POLL_SECONDS = 0.05
# One virtual screen of 1440x1000 pixels with 24-bit colour.
XVFB_SERVER_ARGUMENTS = '-screen 0 1440x1000x24'

# Variables that would connect a test to the user's live desktop session.
LIVE_SESSION_VARIABLES = (
    'DISPLAY',
    'WAYLAND_DISPLAY',
    'DBUS_SESSION_BUS_ADDRESS',
    'DBUS_SESSION_BUS_PID',
    'SESSION_MANAGER',
)

# Per-user directories, each redirected to this subdirectory of a run's root.
USER_DIRECTORIES = {
    'HOME': 'home',
    'XDG_CONFIG_HOME': 'config',
    'XDG_CACHE_HOME': 'cache',
    'XDG_DATA_HOME': 'data',
    'XDG_STATE_HOME': 'state',
    'XDG_RUNTIME_DIR': 'runtime',
}

# Settings that keep GTK and GIO deterministic and away from user state.
GTK_AND_GIO_SETTINGS = {
    # The private display is Xvfb, an X11 server.
    'GDK_BACKEND': 'x11',
    # A GLib or GTK critical warning aborts the test instead of scrolling past.
    'G_DEBUG': 'fatal-criticals',
    # Xvfb has no GPU, so GTK renders in software with Cairo.
    'GSK_RENDERER': 'cairo',
    # GSettings values stay in memory and never reach dconf.
    'GSETTINGS_BACKEND': 'memory',
    # GVfs does not start its FUSE daemon, which would mount a filesystem in
    # the runtime directory.
    'GVFS_DISABLE_FUSE': '1',
    # GIO skips the GVfs volume monitor processes (udisks2, MTP, gphoto2 and
    # others), so tests do not see the machine's drives and phones.
    'GVFS_REMOTE_VOLUME_MONITOR_IGNORE': '1',
}


class CheckError(Exception):
    """A check failed in a way that has no process exit status to report."""


class CheckTimeoutError(CheckError):
    """A check exceeded its time limit; its processes have been stopped."""


def announce(command: Sequence[str]) -> None:
    """Print the command about to run, so the log shows every step."""
    print('+', shlex.join(command), flush=True)


def run(*command: str, capture_stdout: bool = False) -> subprocess.CompletedProcess[str]:
    """Run one check step in the native workspace and raise if it fails.

    With capture_stdout, standard output is returned instead of printed.
    """
    announce(command)
    stdout = subprocess.PIPE if capture_stdout else None
    return subprocess.run(command, cwd=NATIVE, check=True, stdout=stdout, text=True)


def cargo_messages(output: str) -> Iterator[dict[str, Any]]:
    """Parse Cargo's --message-format=json output, one message per line."""
    for line in output.splitlines():
        yield json.loads(line)


def executables_in(output: str) -> set[Path]:
    """Return the test executables named in Cargo's JSON build messages."""
    executables: set[Path] = set()
    for message in cargo_messages(output):
        is_artifact = message.get('reason') == 'compiler-artifact'
        built_for_tests = is_artifact and message.get('profile', {}).get('test')
        if built_for_tests and message.get('executable'):
            executables.add(Path(message['executable']))
    return executables


def print_compiler_errors(output: str) -> None:
    """Show the compiler diagnostics that JSON output would otherwise hide."""
    for message in cargo_messages(output):
        if message.get('reason') == 'compiler-message':
            diagnostic = message['message']
            print(diagnostic.get('rendered', diagnostic['message']), file=sys.stderr)


def compiled_test_binaries() -> list[Path]:
    """Build every test target and return the executables Cargo produced.

    Asking Cargo, instead of globbing target/, runs exactly the tests that were
    just built and none left over from earlier builds.
    """
    try:
        build = run('cargo', 'test', '--workspace', '--all-targets', '--locked', '--no-run',
                    '--message-format=json', capture_stdout=True)
    except subprocess.CalledProcessError as error:
        print_compiler_errors(error.stdout or '')
        raise
    binaries = sorted(executables_in(build.stdout))
    if not binaries:
        raise CheckError('Cargo produced no test executables.')
    return binaries


def private_user_directories(root: Path) -> dict[str, str]:
    """Create HOME and the XDG directories inside root and return their variables."""
    variables = {}
    for name, directory in USER_DIRECTORIES.items():
        path = root / directory
        path.mkdir(mode=0o700)
        variables[name] = str(path)
    return variables


def isolated_environment(root: Path) -> dict[str, str]:
    """Return an environment that keeps GTK, GIO and Python tests off the real session.

    The display and session bus variables are removed, HOME and the XDG
    directories point into root, and GSettings keeps its values in memory.
    GIO keeps using GVfs on purpose: the app relies on its smb:// and mtp://
    URI handling, and the tests exercise it. The private bus has no user
    mounts and GVfs does not mount remote locations on access; tests must
    never mount one, and use simulated devices for transfers instead.
    """
    environment = os.environ.copy()
    # Cargo and rustup must still find the installed compiler when running
    # doctests with a disposable HOME. These are build-tool directories.
    environment.setdefault('CARGO_HOME', str(Path.home() / '.cargo'))
    environment.setdefault('RUSTUP_HOME', str(Path.home() / '.rustup'))
    for name in LIVE_SESSION_VARIABLES:
        environment.pop(name, None)
    environment.update(private_user_directories(root))
    environment.update(GTK_AND_GIO_SETTINGS)
    return environment


def isolated_command(root: Path, command: Sequence[str]) -> list[str]:
    """Wrap a command so it runs on a private X display and D-Bus session.

    The X authority file is kept in root. Without --auth-file, xvfb-run makes
    its own directory under TMPDIR and removes it only when it exits normally,
    so a run stopped by a timeout would leave the directory behind.
    """
    return [
        'xvfb-run',
        '--auto-servernum',
        f'--auth-file={root / "Xauthority"}',
        f'--server-args={XVFB_SERVER_ARGUMENTS}',
        'dbus-run-session',
        '--',
        *command,
    ]


def group_has_members(process: subprocess.Popen[bytes]) -> bool:
    """Return whether any process is left in the process group led by process."""
    process.poll()  # Reap the leader: an unreaped child still counts as a member.
    try:
        os.killpg(process.pid, 0)
    except ProcessLookupError:
        return False
    return True


def wait_for_empty_group(process: subprocess.Popen[bytes], grace_seconds: float) -> bool:
    """Wait up to grace_seconds for the group to empty; return whether it did."""
    deadline = time.monotonic() + grace_seconds
    while group_has_members(process):
        if time.monotonic() >= deadline:
            return False
        time.sleep(POLL_SECONDS)
    return True


def stop_process_group(process: subprocess.Popen[bytes], grace_seconds: float) -> None:
    """Stop every process in the group, sending SIGKILL only if SIGTERM is not enough.

    SIGTERM comes first so Xvfb can remove its display lock and socket.
    """
    for stop_signal in (signal.SIGTERM, signal.SIGKILL):
        if not group_has_members(process):
            return
        try:
            os.killpg(process.pid, stop_signal)
        except ProcessLookupError:
            return  # The last member exited after the check above.
        if wait_for_empty_group(process, grace_seconds):
            return
    print(f'warning: processes of group {process.pid} still run after SIGKILL.',
          file=sys.stderr)


def run_in_own_session(command: Sequence[str], environment: dict[str, str], timeout: float,
                       *, grace_seconds: float = STOP_GRACE_SECONDS) -> None:
    """Run a command in a new session and stop all of its processes afterwards.

    xvfb-run starts Xvfb and dbus-run-session starts a bus daemon. A timeout of
    subprocess.run() kills only the direct child, which would leave those and
    the test itself running on a deleted HOME. A new session puts them all in
    one process group that is stopped together, whether the command succeeds,
    fails, times out or the driver is interrupted.
    """
    announce(command)
    process = subprocess.Popen(command, cwd=NATIVE, env=environment, start_new_session=True)
    try:
        returncode = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        raise CheckTimeoutError(f'{shlex.join(command)} did not finish within {timeout:g} s; '
                           'its processes were stopped.') from None
    finally:
        stop_process_group(process, grace_seconds)
    if returncode != 0:
        raise subprocess.CalledProcessError(returncode, list(command))


def run_isolated(command: Sequence[str], timeout: float) -> None:
    """Run a command on a private display and bus with disposable user data."""
    with tempfile.TemporaryDirectory(prefix='openxplorer-native-test-') as temporary:
        root = Path(temporary)
        run_in_own_session(isolated_command(root, command), isolated_environment(root), timeout)


def check_inventories_and_driver() -> None:
    """Test and validate the parity inventories, then test this driver."""
    python = sys.executable
    run(python, '-m', 'unittest', 'discover', '-s', 'parity', '-p', 'test_*.py')
    run(python, 'parity/check.py')
    run(python, '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_*.py')


def check_formatting_and_lints() -> None:
    """Check rustfmt formatting and lint every target with the workspace lints.

    The lint set lives in [workspace.lints] in native/Cargo.toml, which every
    crate inherits, so no lint is named here. -D warnings makes each finding
    fail the check.
    """
    run('cargo', 'fmt', '--all', '--check')
    run('cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')


def check_rust_tests(test_timeout: float) -> int:
    """Run every test executable, then the doctests, each in isolation.

    Returns the number of test executables that ran.
    """
    binaries = compiled_test_binaries()
    for binary in binaries:
        run_isolated([str(binary), '--test-threads=1'], test_timeout)
    run_isolated(['cargo', 'test', '--workspace', '--doc', '--locked'], test_timeout)
    return len(binaries)


def run_all_checks(test_timeout: float) -> None:
    """Run every check in order, raising on the first failure."""
    check_inventories_and_driver()
    check_formatting_and_lints()
    executable_count = check_rust_tests(test_timeout)
    print(f'Native checks passed ({executable_count} test executables plus doctests).')


def positive_seconds(text: str) -> float:
    """Parse a command-line duration that must be greater than zero."""
    try:
        seconds = float(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f'not a number: {text!r}') from None
    if not seconds > 0:  # Written this way so that NaN is rejected too.
        raise argparse.ArgumentTypeError(f'must be greater than zero: {text!r}')
    return seconds


def parse_arguments(argv: Sequence[str] | None) -> argparse.Namespace:
    """Read the command-line options."""
    parser = argparse.ArgumentParser(
        description='Run the native checks with a private display, session bus and '
                    'disposable user data. Run it from any directory.')
    parser.add_argument(
        '--test-timeout', type=positive_seconds, default=DEFAULT_TEST_TIMEOUT,
        metavar='SECONDS',
        help='stop a test executable, or the doctest run, that takes longer than this '
             f'(default: {DEFAULT_TEST_TIMEOUT:g})')
    return parser.parse_args(argv)


def missing_tools() -> list[str]:
    """Return the required tools that are not on PATH."""
    return [tool for tool in REQUIRED_TOOLS if shutil.which(tool) is None]


def main(argv: Sequence[str] | None = None) -> int:
    """Run all checks and return the exit status: 0 passed, 1 failed, 2 unusable setup."""
    arguments = parse_arguments(argv)
    missing = missing_tools()
    if missing:
        print(f'Required native check tools are missing: {", ".join(missing)}. '
              'See "Build and run" and "Checks" in native/README.md for how to install them.',
              file=sys.stderr)
        return 2
    try:
        run_all_checks(arguments.test_timeout)
    except (CheckError, subprocess.CalledProcessError) as error:
        print(f'Native checks failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
