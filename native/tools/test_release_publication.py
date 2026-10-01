# SPDX-License-Identifier: AGPL-3.0-only
"""Tests that the release workflow publishes every artifact once, after the audit.

The publication runs only in CI on a tagged version, so these tests read the
workflow and check its order and its never-overwrite guard, and check that
tools/release.py lists every artifact in SHA256SUMS.
"""
from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

import package_data

REPOSITORY = package_data.REPOSITORY
WORKFLOW = REPOSITORY / '.github' / 'workflows' / 'checks.yml'
RELEASE_SCRIPT = REPOSITORY / 'tools' / 'release.py'


def step_index(lines: list[str], text: str) -> int:
    """Return the index of the first workflow line, not a comment, that contains text."""
    for index, line in enumerate(lines):
        if text in line and not line.lstrip().startswith('#'):
            return index
    raise AssertionError(f'{text!r} is not in {WORKFLOW.name}')


def publish_step(lines: list[str]) -> str:
    """Return the run script of the step that publishes the release."""
    start = step_index(lines, '- name: Publish a new desktop version')
    end = next(index for index in range(start + 1, len(lines))
               if re.match(r'\s+- name: ', lines[index]))
    return '\n'.join(lines[start:end])


def load_release_script():
    """Import tools/release.py, which is not a package module."""
    spec = importlib.util.spec_from_file_location('release', RELEASE_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleasePublicationTest(unittest.TestCase):
    """The release keeps its assets together and never replaces published ones."""

    # parity: UPD-021
    def test_publication_follows_the_audit_and_never_overwrites(self) -> None:
        """Assets are staged, audited, then created once with SHA256SUMS."""
        lines = WORKFLOW.read_text(encoding='utf-8').splitlines()
        stage = step_index(lines, 'tools/release.py --packages')
        audit = step_index(lines, 'tools/audit-public-data.py')
        publish = step_index(lines, '- name: Publish a new desktop version')
        self.assertLess(stage, audit)
        self.assertLess(audit, publish)

        script = publish_step(lines)
        guard = script.index('gh release view "$tag"')
        create = script.index('gh release create "$tag"')
        self.assertLess(guard, create)
        self.assertIn('leaving its assets unchanged', script[guard:create])
        self.assertNotIn('--clobber', script)
        self.assertNotIn('gh release upload', script)
        self.assertIn("awk '{ print \"dist/\" $2 }' dist/SHA256SUMS", script)
        self.assertIn('"${assets[@]}" dist/SHA256SUMS', script)

        self.assertEqual(self.build_checksummed_release(), [
            ('openxplorer_2.0.0_all.deb', hashlib.sha256(b'deb').hexdigest()),
            (release_bundle_name(), hashlib.sha256(b'flatpak').hexdigest()),
            ('openxplorer-2.0.0-1.x86_64.rpm', hashlib.sha256(b'rpm').hexdigest()),
            ('openxplorer-2.0.0-source.zip', hashlib.sha256(b'source').hexdigest()),
        ])

    def build_checksummed_release(self) -> list[tuple[str, str]]:
        """Run build_release on fake packages in a temporary dist/ and read SHA256SUMS.

        The package verification, the source archive's contents and the preview are
        stubbed; the artifact list and the checksums are release.py's own.
        """
        release = load_release_script()
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            packages = folder / 'packages'
            packages.mkdir()
            (packages / 'openxplorer_2.0.0_all.deb').write_bytes(b'deb')
            (packages / 'openxplorer-2.0.0-1.x86_64.rpm').write_bytes(b'rpm')
            (packages / release.FLATPAK_BUNDLE).write_bytes(b'flatpak')
            (packages / 'notes.txt').write_bytes(b'not released')
            dist = folder / 'dist'
            stubs = {
                'DIST': dist,
                'TEST_RESULTS': folder / 'test-results',
                'DESIGNS': folder / 'designs',
                'WEBSITE_DOWNLOADS': (),
                'release_version': lambda: '2.0.0',
                'verify_debian_package': lambda package: None,
                'write_source_archive': lambda archive: archive.write_bytes(b'source'),
                'publish_preview': lambda: None,
            }
            with mock.patch.multiple(release, **stubs):
                release.build_release(release.parse_arguments(['--packages', str(packages)]))
            lines = (dist / 'SHA256SUMS').read_text().splitlines()
        return [tuple(reversed(line.split('  '))) for line in lines]


def release_bundle_name() -> str:
    """Return the Flatpak bundle's file name in the release."""
    return load_release_script().FLATPAK_BUNDLE


if __name__ == '__main__':
    unittest.main()
