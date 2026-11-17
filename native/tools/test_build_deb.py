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

    def test_only_the_stable_package_recommends_the_mount_helper(self) -> None:
        stable = control_of(Channel.STABLE)['Recommends']
        preview = control_of(Channel.PREVIEW)['Recommends']

        self.assertIn('cifs-utils', stable)
        self.assertNotIn('cifs-utils', preview)
        self.assertNotIn('python', stable + preview)

    # parity: CMD-037
    def test_both_packages_recommend_the_email_portal_backend_for_share(self) -> None:
        for channel in (Channel.STABLE, Channel.PREVIEW):
            with self.subTest(channel=channel):
                fields = control_of(channel)
                self.assertIn('xdg-desktop-portal-gtk', fields['Recommends'])
                self.assertNotIn('xdg-desktop-portal', fields['Depends'])

    def test_both_channels_recommend_pkexec(self) -> None:
        # In-app updates ask for it, and so does the permanent-delete
        # helper of admin:// locations, which both channels install.
        for channel in Channel:
            with self.subTest(channel=channel.name):
                self.assertIn('pkexec', control_of(channel)['Recommends'])

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

                self.assertIn("postinst and postrm refresh caches and change only the update "
                              "setting's files", report.passed)
                shutil.rmtree(control)

    RULE = 'etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules'
    RECORD = 'var/lib/openxplorer/install-updates-without-password'
    TEMPLATE_TEXT = 'polkit.addRule(function (action, subject) {});\n'

    def run_postinst(self, control: Path, root: Path,
                     *arguments: str) -> subprocess.CompletedProcess[str]:
        """Run the postinst with arguments, as dpkg --root=root would."""
        environment = dict(os.environ, DPKG_ROOT=str(root))
        return subprocess.run(['sh', str(control / 'postinst'), *arguments], capture_output=True,
                              text=True, env=environment)

    def system_with_template(self, name: str) -> Path:
        """Return a root with the package's rule template and polkit's rules folder."""
        root = self.folder / name
        template = root / 'usr/share/openxplorer/install-update.rules'
        template.parent.mkdir(parents=True)
        template.write_text(self.TEMPLATE_TEXT, encoding='utf-8')
        (root / self.RULE).parent.mkdir(parents=True)
        return root

    # parity: UPD-003
    def test_installing_the_stable_package_turns_the_setting_on(self) -> None:
        """On by default: a fresh installation, and an upgrade from a
        version that had no record, install the rule and record "on"."""
        control = self.write_scripts(Channel.STABLE)
        for name, arguments in (('fresh', ('configure',)),
                                ('upgrade', ('configure', '2.0.13'))):
            with self.subTest(name=name):
                root = self.system_with_template(name)

                result = self.run_postinst(control, root, *arguments)

                self.assertEqual(result.returncode, 0, result.stderr)
                rule, record = root / self.RULE, root / self.RECORD
                self.assertEqual(rule.read_text(encoding='utf-8'), self.TEMPLATE_TEXT)
                self.assertEqual(record.read_text(encoding='utf-8'), 'on\n')
                self.assertEqual(rule.stat().st_mode & 0o777, 0o644)
                self.assertEqual(record.stat().st_mode & 0o777, 0o644)
                self.assertEqual(sorted(path.name for path in rule.parent.iterdir()), [rule.name])

    # parity: UPD-003
    def test_an_upgrade_keeps_the_setting_off(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        root = self.system_with_template('off')
        record = root / self.RECORD
        record.parent.mkdir(parents=True)
        record.write_text('off\n', encoding='utf-8')

        result = self.run_postinst(control, root, 'configure', '2.0.14')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((root / self.RULE).exists())
        self.assertEqual(record.read_text(encoding='utf-8'), 'off\n')

    def test_an_upgrade_with_the_setting_on_installs_this_versions_rule(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        root = self.system_with_template('on')
        rule, record = root / self.RULE, root / self.RECORD
        rule.write_text('// the last version\'s rule\n', encoding='utf-8')
        record.parent.mkdir(parents=True)
        record.write_text('on\n', encoding='utf-8')

        result = self.run_postinst(control, root, 'configure', '2.0.14')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rule.read_text(encoding='utf-8'), self.TEMPLATE_TEXT)

    def test_installing_never_writes_through_a_link_at_the_rule(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        root = self.system_with_template('link')
        elsewhere = self.folder / 'elsewhere'
        elsewhere.write_text('kept\n', encoding='utf-8')
        (root / self.RULE).symlink_to(elsewhere)

        result = self.run_postinst(control, root, 'configure')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((root / self.RULE).is_symlink())
        self.assertEqual((root / self.RULE).read_text(encoding='utf-8'), self.TEMPLATE_TEXT)
        self.assertEqual(elsewhere.read_text(encoding='utf-8'), 'kept\n')

    def test_installing_without_polkits_rules_folder_still_succeeds(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        root = self.system_with_template('no-polkit')
        (root / self.RULE).parent.rmdir()

        result = self.run_postinst(control, root, 'configure')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((root / self.RULE).parent.exists())

    def test_the_preview_postinst_installs_no_rule(self) -> None:
        control = self.write_scripts(Channel.PREVIEW)
        root = self.system_with_template('preview')

        result = self.run_postinst(control, root, 'configure')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((root / self.RULE).exists())
        self.assertFalse((root / self.RECORD).exists())

    def run_postrm(self, control: Path, action: str, root: Path) -> subprocess.CompletedProcess[str]:
        """Run the postrm for action, as dpkg --root=root would."""
        environment = dict(os.environ, DPKG_ROOT=str(root))
        return subprocess.run(['sh', str(control / 'postrm'), action], capture_output=True,
                              text=True, env=environment)

    def turned_on(self, root: Path) -> tuple[Path, Path]:
        """Write what "Install updates without asking for a password" leaves under root."""
        rule = root / 'etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules'
        record = root / 'var/lib/openxplorer/install-updates-without-password'
        for path in (rule, record):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture\n', encoding='utf-8')
        return rule, record

    # parity: UPD-003
    def test_removing_the_stable_package_removes_the_passwordless_rule(self) -> None:
        """Uninstalling never leaves updates without a password behind,
        and an upgrade keeps the user's choice."""
        control = self.write_scripts(Channel.STABLE)
        for action, kept in (('remove', False), ('purge', False), ('upgrade', True),
                             ('failed-upgrade', True)):
            with self.subTest(action=action):
                root = self.folder / f'root-{action}'
                rule, record = self.turned_on(root)

                result = self.run_postrm(control, action, root)

                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(rule.exists(), kept)
                self.assertEqual(record.exists(), kept)
                self.assertTrue(rule.parent.is_dir())
                if not kept:
                    self.assertFalse(record.parent.exists())

    def test_removing_the_stable_package_with_the_setting_off_succeeds(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        for action in ('remove', 'purge'):
            with self.subTest(action=action):
                result = self.run_postrm(control, action, self.folder / f'empty-{action}')

                self.assertEqual(result.returncode, 0, result.stderr)

    def test_removing_the_stable_package_never_follows_a_link_at_the_rule(self) -> None:
        control = self.write_scripts(Channel.STABLE)
        root = self.folder / 'root'
        rule, _record = self.turned_on(root)
        elsewhere = self.folder / 'elsewhere.rules'
        elsewhere.write_text('kept\n', encoding='utf-8')
        rule.unlink()
        rule.symlink_to(elsewhere)

        result = self.run_postrm(control, 'purge', root)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(rule.is_symlink())
        self.assertEqual(elsewhere.read_text(encoding='utf-8'), 'kept\n')

    # parity: INT-033
    def test_removing_either_package_leaves_super_e_and_home_folders_alone(self) -> None:
        """Super+E lives only in each user's KDE shortcuts and settings
        folder, which the package that remains tidies up: neither package's
        scripts touch a home folder or KDE's shortcuts, and removing or
        purging, again and again, succeeds on a system without KDE."""
        for channel in Channel:
            control = self.write_scripts(channel)
            for script in sorted(control.iterdir()):
                text = script.read_text(encoding='utf-8')
                for word in ('kglobalshortcutsrc', 'kglobalaccel', 'launch-shortcut', '/home',
                             '$HOME', '~/', '.config', 'winspace/'):
                    with self.subTest(channel=channel.name, script=script.name, word=word):
                        self.assertNotIn(word, text)
            root = self.folder / f'no-kde-{channel.name}'
            home = root / 'home/someone/.config'
            home.mkdir(parents=True)
            shortcuts = home / 'kglobalshortcutsrc'
            shortcuts.write_text('[io.winspace.Development.desktop]\nNewWindow=Meta+E,none,New window\n',
                                 encoding='utf-8')
            before = sorted(path.relative_to(root) for path in root.rglob('*'))
            for action in ('remove', 'remove', 'purge', 'purge'):
                with self.subTest(channel=channel.name, action=action):
                    result = self.run_postrm(control, action, root)

                    self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(sorted(path.relative_to(root) for path in root.rglob('*')), before)
            self.assertIn('Meta+E', shortcuts.read_text(encoding='utf-8'))
            shutil.rmtree(control)

    def test_the_preview_postrm_leaves_the_stable_setting_alone(self) -> None:
        control = self.write_scripts(Channel.PREVIEW)
        root = self.folder / 'root'
        rule, record = self.turned_on(root)

        result = self.run_postrm(control, 'purge', root)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(rule.exists())
        self.assertTrue(record.exists())

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
