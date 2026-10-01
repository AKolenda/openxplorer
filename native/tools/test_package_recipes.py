# SPDX-License-Identifier: AGPL-3.0-only
"""Tests that the RPM spec, the PKGBUILD and the source archives agree with the app.

The package recipes are built only in CI containers, so these tests catch
the mistakes that would break them there: a version that differs from Cargo's,
an RPM file list that misses a file package_data.py installs, or a source
archive that differs between two builds.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import tarfile
import tempfile
import unittest

import build_deb
import package_data
from package_data import Channel, InstallRequest, Layout
import source_archive

PACKAGING = package_data.NATIVE / 'packaging'
SPEC = PACKAGING / 'rpm' / 'openxplorer.spec'
PKGBUILD = PACKAGING / 'arch' / 'PKGBUILD'
PREVIEW_METAINFO = package_data.PACKAGING_DATA / 'io.winspace.Development.Native.metainfo.xml'
STABLE_CONDITION = '%if "%{app_id}" == "io.winspace.Development"'


@dataclass(frozen=True)
class RpmMacros:
    """The spec macros the %files section uses, for one channel."""

    channel: Channel

    def expand(self, line: str) -> str:
        """Return line with the macros replaced by their values."""
        values = {'%{_bindir}': '/usr/bin', '%{_datadir}': '/usr/share',
                  '%{name}': self.channel.package, '%{app_id}': self.channel.app_id}
        for macro, value in values.items():
            line = line.replace(macro, value)
        return line


def rpm_file_entries(channel: Channel) -> list[str]:
    """Return the %files entries of the spec for channel, macros expanded."""
    lines = SPEC.read_text(encoding='utf-8').split('%files\n', 1)[1].split('%changelog', 1)[0]
    macros = RpmMacros(channel)
    entries = []
    in_stable_block = False
    for line in lines.splitlines():
        stripped = line.strip()
        if stripped == STABLE_CONDITION:
            in_stable_block = True
        elif stripped == '%endif':
            in_stable_block = False
        elif stripped and not stripped.startswith('#'):
            if channel is Channel.STABLE or not in_stable_block:
                entries.append(macros.expand(stripped))
    return entries


def is_listed(installed: str, entries: list[str]) -> bool:
    """Return whether an installed path is an entry or inside a listed folder."""
    return any(installed == entry or (entry.endswith('/') and installed.startswith(entry))
               for entry in entries)


class RecipeVersionTests(unittest.TestCase):
    """Every recipe builds the version the program reports."""

    def test_the_rpm_and_arch_versions_are_cargos(self) -> None:
        version = build_deb.cargo_version()
        spec_version = re.search(r'^Version:\s+(\S+)$', SPEC.read_text(encoding='utf-8'), re.M)
        pkgbuild_version = re.search(r'^pkgver=(\S+)$', PKGBUILD.read_text(encoding='utf-8'),
                                     re.M)

        self.assertIsNotNone(spec_version)
        self.assertIsNotNone(pkgbuild_version)
        assert spec_version is not None and pkgbuild_version is not None
        self.assertEqual(spec_version.group(1), version)
        self.assertEqual(pkgbuild_version.group(1), version)

    def test_the_preview_metainfo_announces_the_cargo_version(self) -> None:
        text = PREVIEW_METAINFO.read_text(encoding='utf-8')

        newest = re.search(r'<release version="([^"]+)"', text)

        assert newest is not None
        self.assertEqual(newest.group(1), build_deb.cargo_version())


class RpmFileListTests(unittest.TestCase):
    """The spec's %files lists exactly what package_data.py installs."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-recipe-test-')
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)
        self.program = self.folder / 'openxplorer-native'
        self.program.write_bytes(b'\x7fELF fake program')
        self.mount_helper = self.folder / 'openxplorer-mount-share'
        self.mount_helper.write_bytes(b'\x7fELF fake helper')

    def installed_files(self, channel: Channel) -> list[str]:
        """Install the channel's FHS layout and return its files as installed paths."""
        staging = self.folder / channel.name
        request = InstallRequest(channel, Layout.FHS, self.program, staging, self.mount_helper)
        package_data.install(request, [])
        return sorted('/' + path.relative_to(staging).as_posix()
                      for path in staging.rglob('*') if path.is_file() or path.is_symlink())

    def test_every_installed_file_is_in_the_file_list(self) -> None:
        for channel in Channel:
            with self.subTest(channel=channel.name):
                entries = rpm_file_entries(channel)

                unlisted = [path for path in self.installed_files(channel)
                            if not is_listed(path, entries)]

                self.assertEqual(unlisted, [])

    def test_every_file_list_entry_is_installed(self) -> None:
        for channel in Channel:
            with self.subTest(channel=channel.name):
                installed = self.installed_files(channel)

                missing = [entry for entry in rpm_file_entries(channel)
                           if not any(is_listed(path, [entry]) for path in installed)]

                self.assertEqual(missing, [])


class RecipeBuildTests(unittest.TestCase):
    """Every recipe builds and installs the mount helper, and none needs Python to run."""

    def test_the_recipes_build_the_mount_helper(self) -> None:
        for recipe in (SPEC, PKGBUILD):
            with self.subTest(recipe=recipe.name):
                text = recipe.read_text(encoding='utf-8')

                self.assertIn('--package ox-core --bin openxplorer-mount-share', text)
                self.assertIn('--mount-helper', text)

    def test_python_is_only_a_build_tool(self) -> None:
        spec = SPEC.read_text(encoding='utf-8')
        pkgbuild = PKGBUILD.read_text(encoding='utf-8')

        self.assertNotRegex(spec, r'(?m)^(Requires|Recommends):\s+python')
        self.assertNotRegex(pkgbuild, r"optdepends\+?=\([^)]*'python")


class SourceArchiveTests(unittest.TestCase):
    """The vendored-crate archive is the same bytes on every build."""

    # parity: UPD-019
    def test_the_archive_is_reproducible_and_root_owned(self) -> None:
        with tempfile.TemporaryDirectory(prefix='openxplorer-archive-test-') as temporary:
            folder = Path(temporary)
            vendor = folder / 'vendor' / 'glib-0.22.0'
            vendor.mkdir(parents=True)
            (vendor / 'lib.rs').write_text('// crate\n', encoding='utf-8')
            first, second = folder / 'first.tar.gz', folder / 'second.tar.gz'

            for output in (first, second):
                source_archive.write_reproducible_tar(folder / 'vendor', 'openxplorer-1/vendor',
                                                      output, 1_790_000_000)

            self.assertEqual(first.read_bytes(), second.read_bytes())
            with tarfile.open(first) as archive:
                members = archive.getmembers()
            self.assertEqual([member.name for member in members],
                             ['openxplorer-1/vendor/glib-0.22.0',
                              'openxplorer-1/vendor/glib-0.22.0/lib.rs'])
            self.assertTrue(all(member.uid == 0 and member.mtime == 1_790_000_000
                                for member in members))


if __name__ == '__main__':
    unittest.main()
