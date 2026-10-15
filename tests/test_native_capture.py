# SPDX-License-Identifier: AGPL-3.0-only
"""Capture launch isolation, using fictional executable fixtures and no GTK.

The outer bubblewrap gives /home a disposable fictional checkout. The inner
one is the capture tool's real filesystem sandbox. Only its D-Bus/Xvfb launch
wrappers are omitted: the selected executable runs without taking a picture.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import native_capture


class CaptureBuildTests(unittest.TestCase):
    """Captures use the executable Cargo built, including custom target locations."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-capture-build-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.native = self.root / 'native'
        self.native.mkdir()
        self.program = self.native / 'build/release/openxplorer-native'
        self.program.parent.mkdir(parents=True)
        self.program.write_bytes(b'fictional current executable')

    def build(self, messages: list[dict], target: str | None = None) -> Path:
        output = '\n'.join(json.dumps(message) for message in messages) + '\n'
        result = subprocess.CompletedProcess(['cargo'], 0, stdout=output)
        environment = dict(os.environ)
        environment.pop('CARGO_TARGET_DIR', None)
        if target is not None:
            environment['CARGO_TARGET_DIR'] = target
        with patch.object(native_capture, 'NATIVE', self.native), \
                patch.object(native_capture.shutil, 'which', return_value='/fixture/tool'), \
                patch.object(native_capture.subprocess, 'run', return_value=result), \
                patch.dict(os.environ, environment, clear=True):
            return native_capture.build_app()

    def artifact(self) -> dict:
        return {'reason': 'compiler-artifact', 'target': {'name': 'openxplorer-native'},
                'executable': str(self.program)}

    def test_relative_target_directory_uses_cargos_native_working_directory(self) -> None:
        self.assertEqual(self.build([self.artifact()], 'build'), self.program)

    def test_cargo_configuration_cannot_select_a_stale_default_executable(self) -> None:
        stale = self.native / 'target/release/openxplorer-native'
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b'fictional stale executable')
        self.assertEqual(self.build([self.artifact()]), self.program)

    def test_missing_app_artifact_never_falls_back_to_a_stale_binary(self) -> None:
        stale = self.native / 'target/release/openxplorer-native'
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b'fictional stale executable')
        other = {'reason': 'compiler-artifact', 'target': {'name': 'build-script-build'},
                 'executable': str(self.program)}
        with self.assertRaises(native_capture.CaptureError):
            self.build([other, {'reason': 'build-finished', 'success': True}])


@unittest.skipUnless(shutil.which('bwrap'), 'Capture isolation requires bubblewrap')
class CaptureExecutableTests(unittest.TestCase):
    """A capture executable remains available without revealing its home directory."""

    @classmethod
    def setUpClass(cls) -> None:
        """Skip hosts without the namespaces required by the actual capture tool."""
        probe = subprocess.run(
            ['bwrap', '--ro-bind', '/', '/', '--unshare-net', '--unshare-pid', '/bin/true'],
            capture_output=True, text=True, check=False)
        if probe.returncode:
            raise unittest.SkipTest('Capture namespaces unavailable: ' + probe.stderr.strip())

    def setUp(self) -> None:
        """Make an executable and unrelated private fixture outside the real home."""
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-capture-test-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.home = self.root / 'demo'
        self.runtime = self.root / 'runtime'
        self.home.mkdir()
        self.runtime.mkdir(mode=0o700)
        (self.home / 'fixture.txt').write_text('Fictional demo data')
        (self.root / 'private.txt').write_text('Fictional private data')
        self.program = self.root / 'fixture-program'
        self.program.write_text(
            '#!/bin/sh\nset -eu\n'
            'test "$HOME" = /home/demo\n'
            'test -f "$HOME/fixture.txt"\n'
            'test ! -e /home/fictional-checkout/private.txt\n'
            'test ! -e /home/fictional-checkout/fixture-program\n'
            'if { printf "changed" >> "$0"; } 2>/dev/null; then exit 23; fi\n'
            'printf "FICTIONAL_CAPTURE_EXECUTED\\n"\n')
        self.program.chmod(0o755)

    def run_fixture(self, program: Path) -> subprocess.CompletedProcess[str]:
        """Run the selected toy program with the actual capture filesystem arguments."""
        command = native_capture.isolated_command(program, self.home, self.runtime, {})
        # No app, screenshot, X server or session bus is started by this test.
        command = command[:command.index('dbus-run-session')] + [command[-1]]
        outer = ['bwrap', '--dev-bind', '/', '/', '--tmpfs', '/home',
                 '--bind', str(self.root), '/home/fictional-checkout', *command]
        environment = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TZ': 'UTC'}
        return subprocess.run(outer, capture_output=True, text=True, check=False,
                              env=environment, timeout=15)

    def test_program_under_hidden_home_runs_without_exposing_neighboring_files(self) -> None:
        """A home checkout can launch while its siblings stay hidden and it stays read-only."""
        original = self.program.read_bytes()
        result = self.run_fixture(Path('/home/fictional-checkout/fixture-program'))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'FICTIONAL_CAPTURE_EXECUTED\n')
        self.assertEqual(self.program.read_bytes(), original)

    def test_program_outside_home_uses_the_same_read_only_launch(self) -> None:
        """The isolated executable also works for a checkout outside the hidden home."""
        original = self.program.read_bytes()
        result = self.run_fixture(self.program)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'FICTIONAL_CAPTURE_EXECUTED\n')
        self.assertEqual(self.program.read_bytes(), original)

    def test_configured_program_link_in_home_remains_runnable(self) -> None:
        """A configured executable may be a link without exposing its containing folder."""
        (self.root / 'program-link').symlink_to(self.program.name)
        result = self.run_fixture(Path('/home/fictional-checkout/program-link'))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'FICTIONAL_CAPTURE_EXECUTED\n')


if __name__ == '__main__':
    unittest.main()
