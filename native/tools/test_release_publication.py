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

import package_data

REPOSITORY = package_data.REPOSITORY
WORKFLOW = REPOSITORY / '.github' / 'workflows' / 'checks.yml'
RELEASE_SCRIPT = REPOSITORY / 'tools' / 'release.py'


def step_index(lines: list[str], text: str) -> int:
    """Return the index of the first workflow line that contains text."""
    for index, line in enumerate(lines):
        if text in line:
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

        release = load_release_script()
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            package = folder / 'openxplorer_2.0.0_all.deb'
            source = folder / 'openxplorer-2.0.0-source.zip'
            package.write_bytes(b'package')
            source.write_bytes(b'source')
            sums = folder / 'SHA256SUMS'
            release.write_checksums([package, source], sums)
            self.assertEqual(sums.read_text().splitlines(), [
                f'{hashlib.sha256(b"package").hexdigest()}  {package.name}',
                f'{hashlib.sha256(b"source").hexdigest()}  {source.name}',
            ])


if __name__ == '__main__':
    unittest.main()
