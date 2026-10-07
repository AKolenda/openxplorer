# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for the shared install layout (package_data.py) and its verifier.

Each test installs a fake program into a temporary staging folder, as a
package build does, and checks the result with verify_layout.py, the same
checks the release packages must pass.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path, PurePosixPath
import shutil
import tempfile
import unittest
from xml.etree import ElementTree

import package_data
from package_data import Channel, Crate, InstallRequest, Layout
import verify_layout
from verify_layout import InstalledTree, Report, VerificationError

# The version at the top of each channel's metainfo, which a verified tree
# must carry.
METAINFO_VERSIONS = {Channel.PREVIEW: '2.0.3', Channel.STABLE: '2.0.3'}


@dataclass(frozen=True)
class LayoutCase:
    """Where one channel and layout put the program, its command and the mount helper."""

    channel: Channel
    layout: Layout
    program: str
    command: str
    mount_helper: str | None


LAYOUT_CASES = (
    LayoutCase(Channel.STABLE, Layout.DEBIAN, '/opt/openxplorer/bin/openxplorer',
               '/usr/bin/openxplorer', '/opt/openxplorer/bin/openxplorer-mount-share'),
    LayoutCase(Channel.PREVIEW, Layout.DEBIAN, '/opt/openxplorer-native/bin/openxplorer-native',
               '/usr/bin/openxplorer-native', None),
    LayoutCase(Channel.STABLE, Layout.FHS, '/usr/bin/openxplorer', '/usr/bin/openxplorer',
               '/usr/bin/openxplorer-mount-share'),
    LayoutCase(Channel.PREVIEW, Layout.FHS, '/usr/bin/openxplorer-native',
               '/usr/bin/openxplorer-native', None),
    LayoutCase(Channel.STABLE, Layout.FLATPAK, '/app/bin/openxplorer', '/app/bin/openxplorer',
               None),
    LayoutCase(Channel.PREVIEW, Layout.FLATPAK, '/app/bin/openxplorer-native',
               '/app/bin/openxplorer-native', None),
)


class StagingTestCase(unittest.TestCase):
    """Gives each test a temporary folder with a fake program and a fake crate."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-package-data-test-')
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)
        self.program = self.folder / 'openxplorer-native'
        self.program.write_bytes(b'\x7fELF fake program')
        # A real compiled program that accepts --help stands for the helper.
        self.mount_helper = self.folder / 'openxplorer-mount-share'
        shutil.copyfile(shutil.which('true') or '/bin/true', self.mount_helper)
        crate_folder = self.folder / 'crates' / 'glib-0.22.0'
        crate_folder.mkdir(parents=True)
        (crate_folder / 'LICENSE-MIT').write_text('MIT licence text\n', encoding='utf-8')
        (crate_folder / 'src.rs').write_text('// not a licence\n', encoding='utf-8')
        self.crates = [Crate('glib', '0.22.0', 'MIT', crate_folder)]

    def install(self, channel: Channel, layout: Layout) -> InstalledTree:
        """Install one package into a fresh staging folder and return it as a tree."""
        staging = self.folder / f'{channel.name}-{layout.name}'
        request = InstallRequest(channel, layout, self.program, staging, self.mount_helper)
        package_data.install(request, self.crates)
        root = staging / 'app' if layout is Layout.FLATPAK else staging
        return InstalledTree(root, channel, layout, METAINFO_VERSIONS[channel])


class InstalledPathsTests(unittest.TestCase):
    """Each package format puts the program where its updater and users expect it."""

    def test_each_channel_and_layout_installs_where_its_format_expects(self) -> None:
        for case in LAYOUT_CASES:
            with self.subTest(channel=case.channel.name, layout=case.layout.name):
                paths = package_data.installed_paths(case.channel, case.layout)

                self.assertEqual(paths.program, PurePosixPath(case.program))
                self.assertEqual(paths.command, PurePosixPath(case.command))
                expected_helper = case.mount_helper and PurePosixPath(case.mount_helper)
                self.assertEqual(paths.mount_helper, expected_helper)

    def test_the_debian_program_is_where_the_updater_recognises_the_package(self) -> None:
        # Installation::detect in ox-core's update allows in-app updates only
        # for a program in /opt/openxplorer or /opt/openxplorer/bin.
        paths = package_data.installed_paths(Channel.STABLE, Layout.DEBIAN)

        self.assertEqual(paths.program.parent.parent, PurePosixPath('/opt/openxplorer'))


class InstallTests(StagingTestCase):
    """Every package format installs the same verified files."""

    def verify_without_validators(self, tree: InstalledTree) -> Report:
        """Run the layout checks that need no desktop validator programs."""
        report = Report()
        verify_layout.check_promised_files(report, tree)
        verify_layout.check_no_user_state(report, tree)
        verify_layout.check_desktop_entry(report, tree)
        verify_layout.check_service_file(report, tree)
        verify_layout.check_metainfo(report, tree)
        return report

    # parity: INT-003, UPD-017
    def test_every_layout_installs_the_promised_files(self) -> None:
        for case in LAYOUT_CASES:
            with self.subTest(channel=case.channel.name, layout=case.layout.name):
                tree = self.install(case.channel, case.layout)

                report = self.verify_without_validators(tree)

                self.assertIn('Every promised file is installed', report.passed)
                self.assertIn('The D-Bus service starts the app as a GApplication service',
                              report.passed)

    # parity: INT-029, UPD-017
    def test_the_stable_host_packages_keep_the_python_commands(self) -> None:
        for layout in (Layout.DEBIAN, Layout.FHS):
            with self.subTest(layout=layout.name):
                tree = self.install(Channel.STABLE, layout)
                helper = tree.paths.mount_helper
                assert helper is not None

                report = Report()
                verify_layout.check_mount_helper(report, tree, helper)
                verify_layout.check_no_python(report, tree)

                winspace = tree.path_of(tree.paths.commands / package_data.LEGACY_COMMAND)
                self.assertEqual(winspace.resolve(), tree.path_of(tree.paths.program).resolve())
                self.assertIn('The mount helper commands run the helper program', report.passed)
                self.assertIn('The mount helper starts', report.passed)
                self.assertIn('No Python module or script is installed', report.passed)

    def test_the_stable_host_packages_need_the_mount_helper(self) -> None:
        request = InstallRequest(Channel.STABLE, Layout.FHS, self.program, self.folder / 'stage')

        with self.assertRaisesRegex(RuntimeError, 'openxplorer-mount-share'):
            package_data.install(request, self.crates)

    def test_files_get_program_and_data_modes(self) -> None:
        tree = self.install(Channel.PREVIEW, Layout.FHS)
        app_id = Channel.PREVIEW.app_id

        program = tree.path_of(tree.paths.program)
        desktop_entry = tree.path_of(tree.paths.share / 'applications' / f'{app_id}.desktop')

        self.assertEqual(program.stat().st_mode & 0o777, package_data.PROGRAM_MODE)
        self.assertEqual(desktop_entry.stat().st_mode & 0o777, package_data.DATA_MODE)

    def test_the_command_link_resolves_inside_the_staging_folder(self) -> None:
        tree = self.install(Channel.PREVIEW, Layout.DEBIAN)

        command = tree.path_of(tree.paths.command)

        self.assertTrue(command.is_symlink())
        self.assertFalse(Path(command.readlink()).is_absolute())
        self.assertEqual(command.resolve(), tree.path_of(tree.paths.program).resolve())

    def test_crate_licences_travel_with_the_program(self) -> None:
        tree = self.install(Channel.PREVIEW, Layout.FLATPAK)
        crates = tree.path_of(tree.paths.licences / 'rust-crates')

        index = (crates / 'INDEX.txt').read_text(encoding='utf-8')

        self.assertEqual(index, 'glib 0.22.0: MIT\n')
        self.assertTrue((crates / 'glib-0.22.0' / 'LICENSE-MIT').is_file())
        self.assertFalse((crates / 'glib-0.22.0' / 'src.rs').exists())


class VerifierTests(StagingTestCase):
    """The layout verifier refuses the mistakes that would break an installation."""

    # parity: UPD-018
    def test_a_system_wide_file_manager_service_is_refused(self) -> None:
        tree = self.install(Channel.STABLE, Layout.FHS)
        share = tree.path_of(tree.paths.share)
        service = share / verify_layout.SYSTEM_FILE_MANAGER_SERVICE
        service.write_text('[D-BUS Service]\nName=org.freedesktop.FileManager1\n',
                           encoding='utf-8')

        with self.assertRaisesRegex(VerificationError, 'FileManager1'):
            verify_layout.check_no_user_state(Report(), tree)

    # parity: UPD-020
    def test_a_metainfo_without_the_package_version_is_refused(self) -> None:
        tree = self.install(Channel.STABLE, Layout.DEBIAN)
        newer = InstalledTree(tree.root, tree.channel, tree.layout, '99.0.0')

        with self.assertRaisesRegex(VerificationError, 'newest release'):
            verify_layout.check_metainfo(Report(), newer)

    # parity: INT-032
    def test_host_packages_install_an_opt_in_file_dialog_portal(self) -> None:
        for layout in (Layout.DEBIAN, Layout.FHS):
            tree = self.install(Channel.STABLE, layout)
            app_id = Channel.STABLE.app_id
            portal = tree.path_of(tree.paths.share / 'xdg-desktop-portal/portals' / f'{app_id}.portal')
            section = verify_layout.key_file(portal)['portal']
            self.assertEqual(section.get('DBusName'), app_id)
            self.assertEqual(section.get('Interfaces'), 'org.freedesktop.impl.portal.FileChooser;')
            self.assertNotIn('UseIn', section, 'the backend must never volunteer for a desktop')
            verify_layout.check_portal_file(Report(), tree)

    # parity: INT-032
    def test_the_flatpak_installs_no_portal_file(self) -> None:
        tree = self.install(Channel.STABLE, Layout.FLATPAK)
        self.assertIsNone(package_data.portal_file_path(tree.paths, Channel.STABLE.app_id))
        self.assertFalse(tree.path_of(PurePosixPath('/app/share/xdg-desktop-portal')).exists())

    # parity: INT-032
    def test_a_portal_file_that_volunteers_is_refused(self) -> None:
        tree = self.install(Channel.STABLE, Layout.FHS)
        portal = tree.path_of(package_data.portal_file_path(tree.paths, Channel.STABLE.app_id))
        portal.write_text(portal.read_text(encoding='utf-8') + 'UseIn=KDE\n', encoding='utf-8')
        with self.assertRaisesRegex(VerificationError, 'portal file'):
            verify_layout.check_portal_file(Report(), tree)

    def test_a_python_program_is_refused(self) -> None:
        tree = self.install(Channel.STABLE, Layout.DEBIAN)
        script = tree.path_of(tree.paths.commands / 'openxplorer-helper')
        script.write_text('#!/usr/bin/python3\nprint()\n', encoding='utf-8')

        with self.assertRaisesRegex(VerificationError, 'usr/bin/openxplorer-helper'):
            verify_layout.check_no_python(Report(), tree)

    def test_a_missing_promised_file_is_named(self) -> None:
        tree = self.install(Channel.PREVIEW, Layout.FHS)
        app_id = Channel.PREVIEW.app_id
        tree.path_of(tree.paths.share / 'dbus-1/services' / f'{app_id}.service').unlink()

        with self.assertRaisesRegex(VerificationError, f'{app_id}.service'):
            verify_layout.check_promised_files(Report(), tree)

    @unittest.skipUnless(shutil.which('desktop-file-validate') and shutil.which('appstreamcli'),
                         'needs desktop-file-utils and appstream')
    # parity: INT-003, UPD-020
    def test_the_desktop_validators_accept_both_channels(self) -> None:
        for channel in Channel:
            with self.subTest(channel=channel.name):
                tree = self.install(channel, Layout.FHS)
                report = Report()

                verify_layout.check_with_desktop_validators(report, tree)

                self.assertEqual(len(report.passed), 1)


class ContentRatingTests(unittest.TestCase):
    """Software centres show what the app shares, from the metainfo's OARS rating."""

    def test_each_channel_rates_the_update_check_as_mild_social_info(self) -> None:
        # The update check asks GitHub's API for the latest release; OARS
        # names "checking for the latest application version" social-info mild.
        for channel in Channel:
            with self.subTest(channel=channel.name):
                metainfo = package_data.PACKAGING_DATA / f'{channel.app_id}.metainfo.xml'
                rating = ElementTree.parse(metainfo).getroot().find('content_rating')

                self.assertIsNotNone(rating)
                self.assertEqual(rating.get('type'), 'oars-1.1')
                attributes = {item.get('id'): item.text for item in rating.iter('content_attribute')}
                self.assertEqual(attributes, {'social-info': 'mild'})


class CrateSelectionTests(unittest.TestCase):
    """Only the crates linked into the program are credited with its licences."""

    def test_build_and_test_dependencies_are_not_linked_crates(self) -> None:
        def package(name: str, source: str | None) -> dict[str, object]:
            return {'id': name, 'name': name, 'version': '1.0.0', 'license': 'MIT',
                    'source': source, 'manifest_path': f'/crates/{name}/Cargo.toml'}

        def dependency(name: str, kind: str | None) -> dict[str, object]:
            return {'pkg': name, 'dep_kinds': [{'kind': kind}]}

        registry = 'registry+https://github.com/rust-lang/crates.io-index'
        metadata = {
            'packages': [package('ox-app', None), package('ox-core', None),
                         package('gtk4', registry), package('glib-build-tools', registry),
                         package('tempfile', registry)],
            'resolve': {'nodes': [
                {'id': 'ox-app', 'deps': [dependency('ox-core', None), dependency('gtk4', None),
                                          dependency('glib-build-tools', 'build'),
                                          dependency('tempfile', 'dev')]},
                {'id': 'ox-core', 'deps': []},
                {'id': 'gtk4', 'deps': []},
                {'id': 'glib-build-tools', 'deps': []},
                {'id': 'tempfile', 'deps': []},
            ]},
        }

        crates = package_data.program_crates(metadata)

        self.assertEqual([crate.name for crate in crates], ['gtk4'])


if __name__ == '__main__':
    unittest.main()
