# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for the native Debian package: its name, fields, scripts and updater contract.

The package build itself needs Cargo and a release build, so these tests
check the parts that decide whether existing users can move to the native
app: the file name and control fields the 1.1.x updater accepts, the
dependency grouping and the maintainer scripts' safety rules.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import build_deb
from build_deb import BuildError, Channel
import updater_compatibility
import verify_deb
from verify_layout import Report

DEBIAN_LIBRARIES = ['libc6 (>= 2.34)', 'libglib2.0-0t64 (>= 2.54.0)', 'libgtk-4-1 (>= 4.6.0)']


def control_of(channel: Channel, version: str = '2.0.0') -> dict[str, str]:
    """Return the control fields build_deb writes for channel, as a dictionary."""
    identity = build_deb.package_identity(channel, version, 'amd64')
    text = build_deb.control_text(identity, channel, build_deb.depends_field(DEBIAN_LIBRARIES),
                                  1024)
    fields = {}
    for line in text.splitlines():
        if not line.startswith(' '):
            name, _, value = line.partition(': ')
            fields[name] = value
    return fields


class TemporaryFolderTestCase(unittest.TestCase):
    """Gives each test a temporary folder."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-build-deb-test-')
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)


class PackageIdentityTests(unittest.TestCase):
    """The stable package is named and labelled as the 1.1.x updater requires."""

    # parity: UPD-017
    def test_the_stable_package_is_the_installer_the_updater_downloads(self) -> None:
        identity = build_deb.package_identity(Channel.STABLE, '2.0.0', 'amd64')

        self.assertEqual(identity.file_name, 'openxplorer_2.0.0_all.deb')

    def test_the_preview_package_names_its_processor(self) -> None:
        identity = build_deb.package_identity(Channel.PREVIEW, '0.1.0', 'arm64')

        self.assertEqual(identity.file_name, 'openxplorer-native_0.1.0_arm64.deb')

    def test_the_python_updater_accepts_the_stable_asset_name(self) -> None:
        updater = updater_compatibility.python_updater()
        name = build_deb.package_identity(Channel.STABLE, '2.0.0', 'amd64').file_name
        release = {
            'tag_name': 'v2.0.0', 'draft': False, 'prerelease': False, 'body': '',
            'assets': [{'name': name, 'digest': 'sha256:' + 'a' * 64, 'size': 1024,
                        'browser_download_url':
                            f'{updater.REPOSITORY}/releases/download/v2.0.0/{name}'}],
        }

        metadata = updater.release_metadata(release, '1.1.4')

        self.assertTrue(metadata['available'])
        self.assertEqual(metadata['name'], name)

    def test_a_stable_version_must_be_newer_than_the_python_app(self) -> None:
        for version in ('1.1.4', '1.1.3', '0.9.0'):
            with self.subTest(version=version):
                with self.assertRaisesRegex(BuildError, 'must be newer'):
                    build_deb.check_stable_version(version, '1.1.4')
        build_deb.check_stable_version('1.2.0', '1.1.4')

    def test_a_version_the_updater_cannot_read_is_refused(self) -> None:
        for version in ('2.0', '2.0.0-rc.1', '02.0.0'):
            with self.subTest(version=version):
                with self.assertRaisesRegex(BuildError, 'MAJOR.MINOR.PATCH'):
                    build_deb.version_numbers(version)


class ControlFieldTests(unittest.TestCase):
    """Dependencies follow the grouping in native/packaging/README.md."""

    def test_gtk_is_raised_to_the_oldest_supported_version(self) -> None:
        depends = build_deb.depends_field(DEBIAN_LIBRARIES)

        self.assertEqual(depends, 'libc6 (>= 2.34), libglib2.0-0t64 (>= 2.54.0), '
                                  'libgtk-4-1 (>= 4.14), hicolor-icon-theme')

    # parity: INT-029, UPD-017
    def test_the_stable_package_takes_over_the_python_package(self) -> None:
        fields = control_of(Channel.STABLE)

        self.assertEqual(fields['Package'], 'openxplorer')
        self.assertEqual(fields['Architecture'], 'all')
        self.assertEqual(fields['Replaces'], 'winspace-explorer (<< 0.8.0)')
        self.assertEqual(fields['Conflicts'], 'winspace-explorer (<< 0.8.0)')
        self.assertEqual(fields['Provides'], 'winspace-explorer')

    def test_only_the_stable_package_recommends_updates_and_the_mount_helper(self) -> None:
        stable = control_of(Channel.STABLE)['Recommends']
        preview = control_of(Channel.PREVIEW)['Recommends']

        for name in ('pkexec', 'cifs-utils'):
            with self.subTest(name=name):
                self.assertIn(name, stable)
                self.assertNotIn(name, preview)
        self.assertNotIn('python', stable + preview)

    def test_the_preview_installs_beside_the_python_package(self) -> None:
        fields = control_of(Channel.PREVIEW)

        self.assertEqual(fields['Package'], 'openxplorer-native')
        self.assertNotIn('Replaces', fields)
        self.assertNotIn('Conflicts', fields)


class MaintainerScriptTests(TemporaryFolderTestCase):
    """The maintainer scripts refresh caches and refuse a foreign processor, nothing else."""

    def write_scripts(self, channel: Channel) -> Path:
        """Write the channel's maintainer scripts into a DEBIAN folder and return it."""
        control = self.folder / 'DEBIAN'
        control.mkdir()
        build_deb.write_maintainer_scripts(control, channel, 'amd64')
        return control

    def run_preinst(self, control: Path, machine: str) -> subprocess.CompletedProcess[str]:
        """Run the preinst on a computer whose dpkg reports machine."""
        fake_bin = self.folder / 'bin'
        fake_bin.mkdir(exist_ok=True)
        dpkg = fake_bin / 'dpkg'
        dpkg.write_text(f'#!/bin/sh\necho {machine}\n', encoding='utf-8')
        dpkg.chmod(0o755)
        environment = dict(os.environ, PATH=f'{fake_bin}:{os.environ["PATH"]}')
        return subprocess.run(['sh', str(control / 'preinst'), 'upgrade', '1.1.4'],
                              capture_output=True, text=True, env=environment)

    # parity: UPD-018
    def test_both_channels_pass_the_script_checks(self) -> None:
        for channel in Channel:
            with self.subTest(channel=channel.name):
                control = self.write_scripts(channel)
                report = Report()

                verify_deb.check_maintainer_scripts(report, channel, control)

                self.assertIn('postinst and postrm only refresh caches', report.passed)
                shutil.rmtree(control)

    def test_the_stable_preinst_refuses_a_foreign_processor(self) -> None:
        control = self.write_scripts(Channel.STABLE)

        result = self.run_preinst(control, 'arm64')

        self.assertEqual(result.returncode, 1)
        self.assertIn('runs on amd64 computers, not arm64', result.stderr)
        self.assertIn('Flatpak', result.stderr)

    def test_the_stable_preinst_accepts_the_build_processor(self) -> None:
        control = self.write_scripts(Channel.STABLE)

        result = self.run_preinst(control, 'amd64')

        self.assertEqual(result.returncode, 0, result.stderr)

    # parity: UPD-018
    def test_a_script_that_changes_defaults_is_refused(self) -> None:
        control = self.write_scripts(Channel.PREVIEW)
        postinst = control / 'postinst'
        postinst.write_text('#!/bin/sh\nxdg-mime default x.desktop inode/directory\n',
                            encoding='utf-8')

        with self.assertRaises(verify_deb.VerificationError):
            verify_deb.check_maintainer_scripts(Report(), Channel.PREVIEW, control)


class ReproducibilityTests(TemporaryFolderTestCase):
    """The staged package has the same modes, times and checksums on every build."""

    # parity: UPD-019
    def test_staging_is_normalised_and_every_file_is_checksummed(self) -> None:
        stage = self.folder / 'stage'
        (stage / 'DEBIAN').mkdir(parents=True)
        docs = stage / 'usr/share/doc/openxplorer'
        docs.mkdir(parents=True, mode=0o700)
        (docs / 'b.txt').write_text('b\n', encoding='utf-8')
        (docs / 'a.txt').write_text('a\n', encoding='utf-8')

        build_deb.write_md5sums(stage)
        build_deb.normalise(stage, 1_700_000_000)

        self.assertEqual(docs.stat().st_mode & 0o777, 0o755)
        self.assertEqual({path.stat().st_mtime for path in stage.rglob('*')}, {1_700_000_000})
        md5sums = (stage / 'DEBIAN/md5sums').read_text(encoding='utf-8').splitlines()
        self.assertEqual([line.split('  ')[1] for line in md5sums],
                         ['usr/share/doc/openxplorer/a.txt', 'usr/share/doc/openxplorer/b.txt'])


@unittest.skipUnless(shutil.which('dpkg-deb'), 'needs dpkg-deb')
class UpdaterCompatibilityTests(TemporaryFolderTestCase):
    """The real 1.1.x updater installs a package built with the stable identity."""

    def build_package(self, architecture: str) -> Path:
        """Build a minimal openxplorer_9.9.9_all.deb whose control names architecture."""
        stage = self.folder / 'stage'
        (stage / 'DEBIAN').mkdir(parents=True)
        control = (f'Package: openxplorer\nVersion: 9.9.9\nArchitecture: {architecture}\n'
                   'Maintainer: Tests <tests@example.invalid>\nDescription: test\n')
        (stage / 'DEBIAN' / 'control').write_text(control, encoding='utf-8')
        package = self.folder / 'openxplorer_9.9.9_all.deb'
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(stage), str(package)],
                       check=True, capture_output=True)
        return package

    def test_the_updater_installs_a_package_labelled_all(self) -> None:
        package = self.build_package('all')

        updater_compatibility.check_install(package, '9.9.9')

    def test_the_updater_refuses_a_package_labelled_with_a_processor(self) -> None:
        package = self.build_package('amd64')

        with self.assertRaisesRegex(updater_compatibility.UpdaterRefused, 'metadata'):
            updater_compatibility.check_install(package, '9.9.9')


if __name__ == '__main__':
    unittest.main()
