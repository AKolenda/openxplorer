# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for the check driver's session isolation, process cleanup and options.

The driver must stop every process a test run starts, including Xvfb and the
D-Bus daemon, before that run's temporary HOME is deleted. These tests start
real short-lived processes with the live display and session bus removed from
their environment, and find survivors by a marker variable that every
descendant inherits.
"""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import importlib.util
import io
import json
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
    if spec is None or spec.loader is None:
        raise ImportError(f'cannot load the check driver from {CHECK_PATH}')
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
        """A timeout stops the grandchildren too, not only the direct child."""
        with self.assertRaises(check.CheckTimeoutError):
            self.run_shell('sleep 300 & touch "$HOME/started"; wait', timeout=1)
        self.assertTrue((self.root / 'home/started').exists(), 'the script never started')
        self.assert_nothing_left_running()

    def test_processes_that_ignore_sigterm_are_killed(self) -> None:
        """SIGKILL follows when a process survives SIGTERM for the grace period."""
        with self.assertRaises(check.CheckTimeoutError):
            self.run_shell("trap '' TERM; sleep 300 & touch \"$HOME/started\"; wait", timeout=1)
        self.assertTrue((self.root / 'home/started').exists(), 'the script never started')
        self.assert_nothing_left_running()

    def test_background_children_are_stopped_after_a_successful_exit(self) -> None:
        """A command that exits cleanly cannot leave a daemon running on a deleted HOME."""
        self.run_shell('sleep 300 &')
        self.assert_nothing_left_running()

    def test_a_failing_command_reports_its_exit_status(self) -> None:
        """The failure carries the command's own exit status."""
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.run_shell('exit 3')
        self.assertEqual(raised.exception.returncode, 3)


@unittest.skipUnless(all(shutil.which(tool) for tool in ISOLATION_TOOLS),
                     'needs xvfb-run, Xvfb, xauth and dbus-run-session')
class IsolatedRunTests(MarkedProcessTestCase):
    """isolated_command() with a real Xvfb and D-Bus session, as the driver runs tests."""

    def setUp(self) -> None:
        super().setUp()

    def run_isolated_shell(self, script: str, timeout: float) -> None:
        """Run a shell script the way the driver runs a test executable."""
        command = check.isolated_command(self.root, ['sh', '-c', script])
        with contextlib.redirect_stdout(io.StringIO()):  # Hide the command echo.
            check.run_in_own_session(command, self.environment, timeout)

    def run_through_driver(self, command: list[str], timeout: float) -> None:
        """Run a command with run_isolated(), which makes its own root and environment.

        The marker goes into this process's environment, which run_isolated()
        copies, so that survivors can still be found.
        """
        with (patch.dict(os.environ, {MARKER_NAME: self.marker}),
              contextlib.redirect_stdout(io.StringIO())):  # Hide the command echo.
            check.run_isolated(command, timeout)

    def test_the_command_gets_a_private_display_bus_and_authority_file(self) -> None:
        """The command sees a new display and bus, never the ones of the live session."""
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
        """A timed-out run stops Xvfb and the bus and leaves nothing in TMPDIR.

        Some xvfb-run versions leave a directory in their TMPDIR when they are
        stopped, so the run's TMPDIR is inside its root, which the driver
        deletes; the TMPDIR the driver itself runs with stays empty.
        """
        driver_temporary_files = self.root / 'driver-tmp'
        driver_temporary_files.mkdir()
        witness = self.root / 'started'
        command = ['sh', '-c', f'touch {witness}; exec sleep 300']
        with (patch.object(tempfile, 'tempdir', str(driver_temporary_files)),
              patch.dict(os.environ, {'TMPDIR': str(driver_temporary_files)}),
              self.assertRaises(check.CheckTimeoutError)):
            self.run_through_driver(command, timeout=5)
        self.assertTrue(witness.exists(), 'the command never started')
        self.assert_nothing_left_running()
        self.assertEqual(list(driver_temporary_files.iterdir()), [])

    def test_a_failing_test_is_reported_by_its_own_command(self) -> None:
        """The failure names the test command and its status, not the isolation wrapper."""
        command = ['sh', '-c', 'exit 3']
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.run_through_driver(command, timeout=60)
        self.assertEqual(raised.exception.cmd, command)
        self.assertEqual(raised.exception.returncode, 3)
        self.assert_nothing_left_running()

    def test_a_hung_test_is_reported_by_its_own_command(self) -> None:
        """The timeout names the test command, not the isolation wrapper."""
        command = ['sh', '-c', 'exec sleep 300']
        with self.assertRaises(check.CheckTimeoutError) as raised:
            self.run_through_driver(command, timeout=3)
        self.assertEqual(raised.exception.command, command)
        self.assertTrue(str(raised.exception).startswith("sh -c 'exec sleep 300' did not"))
        self.assertIn('--test-timeout', str(raised.exception))
        self.assert_nothing_left_running()


class EnvironmentTests(unittest.TestCase):
    """isolated_environment() hides the live session and redirects user data."""

    def test_the_live_session_is_removed_and_user_directories_are_private(self) -> None:
        """No live-session variable survives, and every user directory is a new one in root."""
        # Listed here, not taken from check.LIVE_SESSION_VARIABLES, so that a
        # variable dropped from that tuple makes this test fail.
        live_session = {
            'DISPLAY': ':0',
            'WAYLAND_DISPLAY': 'wayland-0',
            'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/run/user/1000/bus',
            'DBUS_SESSION_BUS_PID': '1234',
            'SESSION_MANAGER': 'local/host:@/tmp/.ICE-unix/1234,unix/host:/tmp/.ICE-unix/1234',
        }
        with tempfile.TemporaryDirectory(prefix='openxplorer-check-test-') as temporary:
            root = Path(temporary)
            with patch.dict(os.environ, live_session):
                environment = check.isolated_environment(root)
            for name in live_session:
                with self.subTest(variable=name):
                    self.assertNotIn(name, environment)
            for name in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME',
                         'XDG_STATE_HOME', 'XDG_RUNTIME_DIR', 'TMPDIR'):
                with self.subTest(variable=name):
                    self.assertTrue(Path(environment[name]).is_relative_to(root))
                    self.assertTrue(Path(environment[name]).is_dir())
            self.assertEqual(environment['GSETTINGS_BACKEND'], 'memory')
            # The launch tests start real processes only when they see it.
            self.assertEqual(environment['OX_ISOLATED_SESSION'], '1')


class FuseMountTests(unittest.TestCase):
    """fuse_mounts_under() finds the portal mounts a run leaves in its root."""

    def test_only_fuse_mounts_below_the_root_are_found_deepest_first(self) -> None:
        """Other filesystems and FUSE mounts elsewhere are left alone."""
        root = Path('/tmp/openxplorer-native-test-abc')
        mount_table = '\n'.join([
            '81 33 0:82 / /tmp/openxplorer-native-test-abc/runtime/doc rw - fuse.portal portal rw',
            '82 81 0:83 / /tmp/openxplorer-native-test-abc/runtime/doc/by\\040app rw - fuse.portal portal rw',
            '83 33 0:84 / /run/user/1000/doc rw - fuse.portal portal rw',
            '84 33 0:85 / /tmp/openxplorer-native-test-abc/home rw - tmpfs tmpfs rw',
            'a line without the separator',
        ])
        self.assertEqual(check.fuse_mounts_under(root, mount_table), [
            root / 'runtime/doc/by app',
            root / 'runtime/doc',
        ])


class TestExecutableTests(unittest.TestCase):
    """executables_in() picks the test executables out of Cargo's JSON messages."""

    def test_only_executables_built_for_tests_are_returned(self) -> None:
        """Libraries, non-test builds and diagnostics are not test executables."""
        messages = [
            {'reason': 'compiler-artifact', 'profile': {'test': True},
             'executable': '/target/debug/deps/browsing-1'},
            {'reason': 'compiler-artifact', 'profile': {'test': False},
             'executable': '/target/debug/openxplorer-native'},
            {'reason': 'compiler-artifact', 'profile': {'test': True}, 'executable': None},
            {'reason': 'compiler-message', 'message': {'message': 'unused variable'}},
            {'reason': 'build-finished', 'success': True},
        ]
        output = '\n'.join(json.dumps(message) for message in messages)
        self.assertEqual(check.executables_in(output), {Path('/target/debug/deps/browsing-1')})


class OptionTests(unittest.TestCase):
    """The command-line options and the start-up check for required tools."""

    def test_the_test_timeout_must_be_a_positive_number(self) -> None:
        """Zero, negative, NaN and non-numeric timeouts are refused; others are parsed."""
        self.assertEqual(check.positive_seconds('2.5'), 2.5)
        refused = {'zero': '0', 'negative': '-1', 'not a number': 'nan', 'a word': 'soon'}
        for case, text in refused.items():
            with self.subTest(case=case), self.assertRaises(argparse.ArgumentTypeError):
                check.positive_seconds(text)

    def test_missing_tools_stop_the_driver_before_any_check(self) -> None:
        """Without its tools the driver runs nothing and exits with status 2."""
        stderr = io.StringIO()
        with (patch.object(check.shutil, 'which', return_value=None),
              patch.object(check, 'run_all_checks') as run_all_checks,
              contextlib.redirect_stderr(stderr)):
            status = check.main([])
        self.assertEqual(status, 2)
        run_all_checks.assert_not_called()
        self.assertIn('Required native check tools are missing: cargo, git, python3',
                      stderr.getvalue())


class FailureReportTests(unittest.TestCase):
    """The driver's last line names what failed, in a form that can be rerun."""

    def test_a_failed_command_is_shown_as_it_would_be_typed(self) -> None:
        """Arguments are shell-quoted, and a signal is told apart from an exit status."""
        cases = {
            'exit status': (
                subprocess.CalledProcessError(101, ('cargo', 'clippy', '--', '-D', 'warnings')),
                'cargo clippy -- -D warnings exited with status 101',
            ),
            'signal': (
                subprocess.CalledProcessError(-9, ['cargo', 'test']),
                'cargo test was killed by signal 9',
            ),
            'path with a space': (
                subprocess.CalledProcessError(1, [Path('/tmp/a b/browsing-1'), '--nocapture']),
                "'/tmp/a b/browsing-1' --nocapture exited with status 1",
            ),
        }
        for case, (error, description) in cases.items():
            with self.subTest(case=case):
                self.assertEqual(check.describe_failed_command(error), description)

    def test_a_failed_step_ends_the_run_with_status_1_and_names_the_step(self) -> None:
        """main() reports the failed command, not a Python repr of it, and returns 1."""
        failure = subprocess.CalledProcessError(101, ('cargo', 'clippy', '--workspace'))
        stderr = io.StringIO()
        with (patch.object(check, 'missing_tools', return_value=[]),
              patch.object(check, 'leftover_desktop_note', return_value=None),
              patch.object(check, 'run_all_checks', side_effect=failure),
              contextlib.redirect_stderr(stderr)):
            status = check.main([])
        self.assertEqual(status, 1)
        self.assertEqual(stderr.getvalue(), 'Native checks failed: cargo clippy --workspace '
                                            'exited with status 101; its output is above.\n')

    def test_a_timeout_ends_the_run_with_status_1_and_says_how_to_recover(self) -> None:
        """A timeout names the command and the option that allows more time."""
        failure = check.CheckTimeoutError(['/target/debug/deps/browsing-1'], 180.0)
        stderr = io.StringIO()
        with (patch.object(check, 'missing_tools', return_value=[]),
              patch.object(check, 'leftover_desktop_note', return_value=None),
              patch.object(check, 'run_all_checks', side_effect=failure),
              contextlib.redirect_stderr(stderr)):
            status = check.main([])
        self.assertEqual(status, 1)
        self.assertEqual(stderr.getvalue(),
                         'Native checks failed: /target/debug/deps/browsing-1 did not finish '
                         'within 180 s; its processes were stopped. Rerun with a larger '
                         '--test-timeout if the test is only slow.\n')


@dataclasses.dataclass(frozen=True)
class SourceLine:
    """One line of one file under crates/, as the drawn-icon guard reads it."""

    file: str
    text: str


class DrawnIconTests(unittest.TestCase):
    """Icons are bundled files: drawing one in code, or shipping another image, fails the check."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-drawn-icons-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, name: str, text: str) -> None:
        """Write one file under the temporary crates directory."""
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text + '\n', encoding='utf-8')

    def assert_each_line_is_found(self, cases: dict[str, SourceLine]) -> None:
        """Each case, alone in its file, is reported with its file and line."""
        for case, source in cases.items():
            with self.subTest(case=case):
                self.write(source.file, source.text)
                expected = [f'{source.file}:1: {source.text}']
                self.assertEqual(check.drawn_icon_lines(self.root), expected)
                (self.root / source.file).unlink()

    def test_svg_path_data_in_code_stylesheets_or_templates_is_found(self) -> None:
        """Each way of writing path data, the old hand-drawn icons' included, is reported."""
        self.assert_each_line_is_found({
            'a d attribute': SourceLine(
                'art/svg.rs', 'const FOLDER: &str = r##"<path d="M4 12a3 3 0 0 1 3-3Z"/>"##;'),
            'an escaped d attribute': SourceLine('art.rs', 'let folder = "<path d=\\"M4 12h3\\"/>";'),
            'a path table': SourceLine(
                'glyphs.rs', 'Glyph::Terminal => "M3 5h18v14H3zM6 9l3 3-3 3M12 15h5",'),
            'a relative move': SourceLine('glyphs.rs', 'Glyph::Close => "m6 6 12 12M18 6 6 18",'),
            'numbers without a leading zero': SourceLine(
                'glyphs.rs', 'const COMPACT: &str = "M.5 2h3v4z";'),
            'a sign as the separator': SourceLine('glyphs.rs', 'const SIGNED: &str = "M2-3h4v4z";'),
            'a path formatted from numbers': SourceLine(
                'glyphs.rs', 'let path = format!("M{x} {y}h{w}");'),
            'a path split by concat!': SourceLine(
                'glyphs.rs', 'const PATH: &str = concat!("M", "3 5h18v14H3z");'),
            'a path continued on the next line': SourceLine(
                'glyphs.rs', 'const PATH: &str = "M3 5\\'),
            'a path in a template': SourceLine(
                'resources/ui/art.ui',
                '<property name="data">&lt;path d=&quot;M4 12h3&quot;/&gt;</property>'),
        })

    def test_drawing_apis_are_found(self) -> None:
        """GTK's and Cairo's ways to draw a shape are reported, also under another name."""
        self.assert_each_line_is_found({
            'a parsed path': SourceLine(
                'glyphs.rs', 'let path = gsk::Path::parse(glyph.path_data());'),
            'the path type renamed': SourceLine('glyphs.rs', 'use gtk::gsk::Path as P;'),
            'the path type imported': SourceLine('glyphs.rs', 'use gtk::gsk::{self, Path};'),
            'a path builder': SourceLine('glyphs.rs', 'let path = gsk::PathBuilder::new();'),
            'a Cairo context': SourceLine('glyphs.rs', 'fn draw(context: &cairo::Context) {'),
            'Cairo imported': SourceLine('glyphs.rs', 'use gtk::cairo;'),
            'a drawing area': SourceLine('glyphs.rs', 'let area = gtk::DrawingArea::new();'),
            'a drawing area in a template': SourceLine(
                'resources/ui/art.ui', '<object class="GtkDrawingArea" id="art"/>'),
            'a draw function': SourceLine('glyphs.rs', 'area.set_draw_func(draw_folder);'),
            'a filled shape': SourceLine(
                'glyphs.rs', 'snapshot.append_fill(&shape, rule, &colour);'),
            'a stroked shape': SourceLine(
                'glyphs.rs', 'snapshot.append_stroke(&shape, &stroke, &colour);'),
            'a coloured rectangle': SourceLine(
                'glyphs.rs', 'snapshot.append_color(&colour, &bounds);'),
            'a gradient': SourceLine(
                'glyphs.rs', 'snapshot.append_linear_gradient(&bounds, &start, &end, &stops);'),
        })

    def test_pictures_embedded_in_code_are_found(self) -> None:
        """A data: URI, or an image file compiled into the binary, is reported."""
        self.assert_each_line_is_found({
            'a data URI in a stylesheet': SourceLine(
                'skin.css', 'image { -gtk-icon-source: url("data:image/svg+xml,<svg/>"); }'),
            'a data URI in a template': SourceLine(
                'resources/ui/art.ui',
                '<property name="file">data:image/svg+xml,&lt;svg/&gt;</property>'),
            'an SVG file compiled in': SourceLine(
                'art.rs', 'const FOLDER: &[u8] = include_bytes!("../resources/art/folder.svg");'),
            'a PNG file compiled in': SourceLine(
                'art.rs', 'const FOLDER: &[u8] = include_bytes!("folder.PNG");'),
        })

    def test_named_icons_and_other_text_pass(self) -> None:
        """Icon names, CSS rules and text that only looks a little like a drawing are fine."""
        self.write('window/tab_strip.rs', '\n'.join([
            'let close = icons::image(Icon::Dismiss16, CLOSE_GLYPH);',
            'image.set_icon_name(Some("ox-add-20-symbolic"));',
            'let chip = "M3 Pro";',
            'let d = "Music";',
            'let d = distance(start, end);',
            'assert_eq!(label, "Media (M:)");',
            'let label = concat!("Media", " (M:)");',
            'include_str!("../../resources/skin/base.css"),',
            'gio::resources_register_include!("icons.gresource")',
        ]))
        self.write('skin/title-bar.css', '\n'.join([
            '.tab > .tab-icon { margin-left: 2px; }',
            "/* GTK's cairo renderer drops a shadow this faint. */",
        ]))
        self.write('resources/ui/address-bar.ui', '<object class="GtkImage" id="icon"/>')
        self.write('resources/icons/icons.gresource.xml',
                   '<file>scalable/actions/ox-add-20-symbolic.svg</file>')
        self.assertEqual(check.drawn_icon_lines(self.root), [])

    def test_the_vendored_svg_files_are_not_read(self) -> None:
        """The bundled icons are SVG files with path data of their own."""
        self.write('ox-app/resources/icons/hicolor/scalable/actions/ox-add-20-symbolic.svg',
                   '<svg><path d="M10 2.5v15"/></svg>')
        self.assertEqual(check.drawn_icon_lines(self.root), [])
        self.assertEqual(check.stray_images(self.root), [])

    def test_images_outside_the_bundled_icons_are_found(self) -> None:
        """A hand-made picture cannot hide as a file next to the code or the icons."""
        self.write('ox-app/resources/art/folder.svg', '<svg/>')
        self.write('ox-app/resources/icons/folder.svg', '<svg/>')
        self.write('ox-app/src/folder.PNG', 'a picture')
        outside = 'an image outside ox-app/resources/icons/hicolor'
        self.assertEqual(check.stray_images(self.root), [
            f'ox-app/resources/art/folder.svg: {outside}',
            f'ox-app/resources/icons/folder.svg: {outside}',
            f'ox-app/src/folder.PNG: {outside}',
        ])

    def test_the_check_fails_and_lists_every_drawn_icon(self) -> None:
        """The failure names each file and line, and each stray image, so they can be found."""
        self.write('crates/ox-app/src/glyphs.rs', 'let path = gsk::Path::parse("M5 12h14");')
        self.write('crates/ox-app/resources/art/folder.svg', '<svg/>')
        with (patch.object(check, 'NATIVE', self.root),
              contextlib.redirect_stdout(io.StringIO())):
            with self.assertRaises(check.CheckError) as raised:
                check.check_no_drawn_icons()
        report = str(raised.exception)
        self.assertRegex(report, r'ox-app/src/glyphs\.rs:1: ')
        self.assertRegex(report, r'ox-app/resources/art/folder\.svg: an image outside')

    def test_the_native_crates_draw_no_icons(self) -> None:
        """The real sources pass the guard."""
        crates = check.NATIVE / 'crates'
        self.assertEqual(check.drawn_icon_lines(crates), [])
        self.assertEqual(check.stray_images(crates), [])



class LeftoverDesktopTests(unittest.TestCase):
    """An updated clone is told to delete the retired app's leftover folder."""

    def test_a_leftover_desktop_folder_is_named(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repository = Path(directory)
            self.assertIsNone(check.leftover_desktop_note(repository))
            (repository / 'desktop/__pycache__').mkdir(parents=True)
            note = check.leftover_desktop_note(repository)
            self.assertIn(str(repository / 'desktop'), note)
            self.assertIn('delete it', note)

if __name__ == '__main__':
    unittest.main()
