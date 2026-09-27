# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for the check driver's session isolation and process cleanup.

The driver must stop every process a test run starts, including Xvfb and the
D-Bus daemon, before that run's temporary HOME is deleted. These tests start
real short-lived processes with the live display and session bus removed from
their environment, and find survivors by a marker variable that every
descendant inherits.
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import types
import unittest
from unittest.mock import patch
import uuid

CHECK_PATH = Path(__file__).resolve().with_name('check.py')
MARKER_NAME = 'OPENXPLORER_CHECK_TEST_MARKER'
ISOLATION_TOOLS = ('xvfb-run', 'Xvfb', 'xauth', 'dbus-run-session')


def load_check_driver() -> types.ModuleType:
    """Import check.py by path, because native/parity has a check.py as well."""
    spec = importlib.util.spec_from_file_location('native_check_driver', CHECK_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


check = load_check_driver()


def processes_with_marker(marker: str) -> list[int]:
    """Return the running processes whose environment carries the marker."""
    needle = f'{MARKER_NAME}={marker}'.encode()
    found = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            variables = (entry / 'environ').read_bytes().split(b'\0')
        except OSError:
            continue  # The process exited meanwhile, or belongs to another user.
        if needle in variables:
            found.append(int(entry.name))
    return found


class MarkedProcessTestCase(unittest.TestCase):
    """Gives each test a disposable root and an isolated, marked environment."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-check-test-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.marker = uuid.uuid4().hex
        self.environment = check.isolated_environment(self.root)
        self.environment[MARKER_NAME] = self.marker
        self.addCleanup(self.stop_survivors)

    def stop_survivors(self) -> None:
        """Keep a failing test from leaving its processes behind.

        SIGTERM comes first so that a surviving Xvfb removes its display lock.
        """
        for stop_signal in (signal.SIGTERM, signal.SIGKILL):
            survivors = processes_with_marker(self.marker)
            for pid in survivors:
                with contextlib.suppress(ProcessLookupError):
                    os.kill(pid, stop_signal)
            if survivors:
                time.sleep(1)

    def assert_nothing_left_running(self) -> None:
        """Fail if any process started by the command still runs."""
        self.assertEqual(processes_with_marker(self.marker), [])


class ProcessCleanupTests(MarkedProcessTestCase):
    """run_in_own_session() stops the whole process group, however it ends."""

    def run_shell(self, script: str, timeout: float = 30) -> None:
        """Run a shell script with a short grace period between stop signals."""
        with contextlib.redirect_stdout(io.StringIO()):  # Hide the command echo.
            check.run_in_own_session(['sh', '-c', script], self.environment, timeout,
                                     grace_seconds=0.5)

    def test_a_timeout_stops_the_command_and_its_background_children(self) -> None:
        with self.assertRaises(check.CheckTimeout):
            self.run_shell('sleep 300 & touch "$HOME/started"; wait', timeout=1)
        self.assertTrue((self.root / 'home/started').exists(), 'the script never started')
        self.assert_nothing_left_running()

    def test_processes_that_ignore_sigterm_are_killed(self) -> None:
        with self.assertRaises(check.CheckTimeout):
            self.run_shell("trap '' TERM; sleep 300 & touch \"$HOME/started\"; wait", timeout=1)
        self.assertTrue((self.root / 'home/started').exists(), 'the script never started')
        self.assert_nothing_left_running()

    def test_background_children_are_stopped_after_a_successful_exit(self) -> None:
        self.run_shell('sleep 300 &')
        self.assert_nothing_left_running()

    def test_a_failing_command_reports_its_exit_status(self) -> None:
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.run_shell('exit 3')
        self.assertEqual(raised.exception.returncode, 3)


@unittest.skipUnless(all(shutil.which(tool) for tool in ISOLATION_TOOLS),
                     'needs xvfb-run, Xvfb, xauth and dbus-run-session')
class IsolatedRunTests(MarkedProcessTestCase):
    """isolated_command() with a real Xvfb and D-Bus session, as the driver runs tests."""

    def setUp(self) -> None:
        super().setUp()
        # xvfb-run would create its own authority directory here.
        self.temporary_files = self.root / 'tmp'
        self.temporary_files.mkdir()
        self.environment['TMPDIR'] = str(self.temporary_files)

    def run_isolated_shell(self, script: str, timeout: float) -> None:
        """Run a shell script the way the driver runs a test executable."""
        command = check.isolated_command(self.root, ['sh', '-c', script])
        with contextlib.redirect_stdout(io.StringIO()):  # Hide the command echo.
            check.run_in_own_session(command, self.environment, timeout)

    def test_the_command_gets_a_private_display_bus_and_authority_file(self) -> None:
        self.run_isolated_shell(
            'printf "%s\\n" "$DISPLAY" "$DBUS_SESSION_BUS_ADDRESS" "$XAUTHORITY"'
            ' > "$HOME/session"', timeout=60)
        display, bus, authority = (self.root / 'home/session').read_text().splitlines()
        self.assertRegex(display, r'^:\d+$')
        self.assertNotEqual(display, os.environ.get('DISPLAY'))
        self.assertTrue(bus)
        self.assertNotEqual(bus, os.environ.get('DBUS_SESSION_BUS_ADDRESS'))
        self.assertEqual(authority, str(self.root / 'Xauthority'))
        self.assert_nothing_left_running()

    def test_a_hung_test_leaves_no_display_bus_or_temporary_files(self) -> None:
        with self.assertRaises(check.CheckTimeout):
            self.run_isolated_shell('touch "$HOME/started"; exec sleep 300', timeout=5)
        self.assertTrue((self.root / 'home/started').exists(), 'the command never started')
        self.assert_nothing_left_running()
        self.assertEqual(list(self.temporary_files.iterdir()), [])


class EnvironmentTests(unittest.TestCase):
    """isolated_environment() hides the live session and redirects user data."""

    def test_the_live_session_is_removed_and_user_directories_are_private(self) -> None:
        live_session = {'DISPLAY': ':0', 'WAYLAND_DISPLAY': 'wayland-0',
                        'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/run/user/1000/bus'}
        with tempfile.TemporaryDirectory(prefix='openxplorer-check-test-') as temporary:
            root = Path(temporary)
            with patch.dict(os.environ, live_session):
                environment = check.isolated_environment(root)
            for name in live_session:
                self.assertNotIn(name, environment)
            for name in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME',
                         'XDG_STATE_HOME', 'XDG_RUNTIME_DIR'):
                with self.subTest(variable=name):
                    self.assertTrue(Path(environment[name]).is_relative_to(root))
                    self.assertTrue(Path(environment[name]).is_dir())
            self.assertEqual(environment['GSETTINGS_BACKEND'], 'memory')


if __name__ == '__main__':
    unittest.main()
