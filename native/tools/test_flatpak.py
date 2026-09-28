# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for the Flatpak's offline Cargo sources and its two manifests."""
from __future__ import annotations

import json
from pathlib import Path
import re
import unittest

import flatpak_cargo_sources
from flatpak_cargo_sources import LockedCrate, LockFileError

FLATPAK_FOLDER = flatpak_cargo_sources.NATIVE / 'packaging' / 'flatpak'
PREVIEW_MANIFEST = FLATPAK_FOLDER / 'io.winspace.Development.Native.yml'
STABLE_MANIFEST = FLATPAK_FOLDER / 'io.winspace.Development.yml'
PACKAGING_README = flatpak_cargo_sources.NATIVE / 'packaging' / 'README.md'
PERMISSIONS_HEADING = '### Flatpak permissions'

LOCK_WITH_A_MEMBER_AND_A_CRATE = '''
version = 4

[[package]]
name = "ox-core"
version = "0.1.0"

[[package]]
name = "glib"
version = "0.22.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
'''


def documented_permissions() -> list[str]:
    """Return the permissions the README's permission table explains, in its order."""
    section = PACKAGING_README.read_text(encoding='utf-8').split(PERMISSIONS_HEADING, 1)[1]
    rows = [line for line in section.split('\n### ', 1)[0].splitlines()
            if line.startswith('| `')]
    first_cells = [row.split('|')[1] for row in rows]
    return [permission for cell in first_cells for permission in re.findall(r'`(--[^`]+)`', cell)]


def manifest_body(path: Path) -> str:
    """Return a manifest without its leading comment, which describes the channel."""
    text = path.read_text(encoding='utf-8')
    return text[text.index('\nid: '):]


class CargoSourcesTests(unittest.TestCase):
    """Every locked crate becomes a checked download in the vendor folder."""

    def test_workspace_members_are_built_from_the_checkout(self) -> None:
        crates = flatpak_cargo_sources.locked_crates(LOCK_WITH_A_MEMBER_AND_A_CRATE)

        self.assertEqual([crate.name for crate in crates], ['glib'])

    def test_a_crate_is_downloaded_with_its_lock_file_checksum(self) -> None:
        crate = LockedCrate('glib', '0.22.0', 'ab' * 32)

        archive, checksum_file = flatpak_cargo_sources.crate_sources(crate)

        self.assertEqual(archive['url'],
                         'https://static.crates.io/crates/glib/glib-0.22.0.crate')
        self.assertEqual(archive['sha256'], 'ab' * 32)
        self.assertEqual(archive['dest'], 'cargo/vendor/glib-0.22.0')
        self.assertEqual(checksum_file['dest'], archive['dest'])
        self.assertEqual(json.loads(checksum_file['contents']),
                         {'package': 'ab' * 32, 'files': {}})

    def test_cargo_is_pointed_at_the_vendor_folder(self) -> None:
        sources = flatpak_cargo_sources.flatpak_sources([])

        self.assertEqual(sources[-1]['dest'], 'cargo')
        self.assertEqual(sources[-1]['dest-filename'], 'config.toml')
        self.assertIn('replace-with = "vendored-sources"', sources[-1]['contents'])
        self.assertIn('directory = "cargo/vendor"', sources[-1]['contents'])

    def test_a_git_dependency_is_refused(self) -> None:
        lock = LOCK_WITH_A_MEMBER_AND_A_CRATE.replace(
            'registry+https://github.com/rust-lang/crates.io-index',
            'git+https://example.invalid/glib#0123')

        with self.assertRaisesRegex(LockFileError, 'only crates.io'):
            flatpak_cargo_sources.locked_crates(lock)

    def test_a_crate_without_a_checksum_is_refused(self) -> None:
        lock = LOCK_WITH_A_MEMBER_AND_A_CRATE.replace('checksum = ', 'unchecked = ')

        with self.assertRaisesRegex(LockFileError, 'no checksum'):
            flatpak_cargo_sources.locked_crates(lock)

    def test_the_committed_sources_match_cargo_lock(self) -> None:
        lock = flatpak_cargo_sources.CARGO_LOCK.read_text(encoding='utf-8')
        committed = flatpak_cargo_sources.SOURCES_FILE.read_text(encoding='utf-8')

        self.assertEqual(committed, flatpak_cargo_sources.sources_text(lock),
                         'Run python3 native/tools/flatpak_cargo_sources.py after changing '
                         'Cargo.lock.')


class ManifestTests(unittest.TestCase):
    """The preview and stable manifests build the same app under their own IDs."""

    def test_the_stable_manifest_is_the_preview_under_the_stable_id(self) -> None:
        preview = manifest_body(PREVIEW_MANIFEST)
        expected = preview.replace('io.winspace.Development.Native', 'io.winspace.Development')
        expected = expected.replace('command: openxplorer-native', 'command: openxplorer')

        self.assertEqual(manifest_body(STABLE_MANIFEST), expected)

    def test_the_build_uses_the_vendored_crates_offline(self) -> None:
        manifest = PREVIEW_MANIFEST.read_text(encoding='utf-8')

        self.assertIn('- cargo-sources.json', manifest)
        self.assertIn('CARGO_HOME: /run/build/openxplorer/cargo', manifest)
        self.assertIn('cargo build --release --locked --offline', manifest)

    def test_the_flatpak_grants_only_the_permissions_the_readme_explains(self) -> None:
        lines = manifest_body(PREVIEW_MANIFEST).splitlines()
        block = lines[lines.index('finish-args:') + 1:lines.index('modules:')]
        permissions = [line.strip().removeprefix('- ') for line in block
                       if line.strip().startswith('- ')]

        self.assertEqual(permissions, documented_permissions())

if __name__ == '__main__':
    unittest.main()
