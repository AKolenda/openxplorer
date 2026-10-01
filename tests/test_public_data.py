# SPDX-License-Identifier: AGPL-3.0-only
"""Publication privacy checks, using fictional identifiers and temporary files.

tools/audit-public-data.py rejects publication inputs that contain private
identifiers, however they are encoded, cased, wrapped or nested in archives.
Each test points the audit at a temporary fixture tree with its own valid
screenshot provenance, so the repository's real captures cannot decide the
result.
"""
from __future__ import annotations

import base64
from collections.abc import Iterator
from contextlib import chdir
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
from typing import Any
import unittest
from unittest.mock import patch
import zipfile

AUDIT_PATH = Path(__file__).resolve().parents[1] / 'tools/audit-public-data.py'
SCREENSHOTS = 'apps/web/public/assets/screenshots'
# The audit expects exactly seven registered product screenshots.
SCREENSHOT_COUNT = 7
ONE_PIXEL_PNG = base64.b64decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF1kAAAAASUVORK5CYII=')


def load_audit() -> types.ModuleType:
    """Import the audit script by path; its file name is not a valid module name."""
    spec = importlib.util.spec_from_file_location('public_data_audit', AUDIT_PATH)
    if spec is None or spec.loader is None:
        raise ImportError(f'cannot load the public-data audit from {AUDIT_PATH}')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PublicDataAuditTests(unittest.TestCase):
    """audit() and the command line reject fictional private identifiers."""

    def setUp(self) -> None:
        self.audit = load_audit()
        self.audit.DENIED.clear()
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-public-data-test-')
        self.addCleanup(temporary.cleanup)
        # audit() resolves its inputs, so ROOT must be resolved too; otherwise
        # a TMPDIR reached through a symlink breaks every relative name.
        self.root = Path(temporary.name).resolve()
        self.audit.ROOT = self.root
        self.put_screenshot_provenance()

    def put_screenshot_provenance(self) -> None:
        """Give the fixture tree valid screenshots and a manifest that registers them.

        With isolated, valid provenance, unrelated repository captures do not
        determine whether these privacy regression checks pass.
        """
        screenshots = {}
        for number in range(SCREENSHOT_COUNT):
            name = f'fixture-{number}.png'
            self.put(f'{SCREENSHOTS}/{name}', ONE_PIXEL_PNG)
            screenshots[name] = self.audit.digest(ONE_PIXEL_PNG)
        self.put(f'{SCREENSHOTS}/manifest.json', json.dumps({
            'fixturePolicy': 'Fictional one-pixel test fixtures only.',
            'nativeRuntime': True,
            'sha256': screenshots,
        }))

    def put(self, relative: str, content: str | bytes) -> Path:
        """Create a file in the fixture tree, with its parent directories."""
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content if isinstance(content, bytes) else content.encode())
        return path

    def test_full_names_addresses_and_encoded_or_wrapped_text_are_rejected(self) -> None:
        """Casing, wrapping, URL and HTML encoding do not hide an identifier."""
        self.audit.deny_terms(['Fictional Private Customer', '123 Fictional Lane'])
        examples = [
            'Contains Fictional Private Customer in plain text.',
            'Contains FICTIONAL PRIVATE CUSTOMER with different casing.',
            'Contains Fictional\n  Private\tCustomer with wrapped whitespace.',
            'Contains Fictional%20Private%20Customer in URL-encoded text.',
            'Contains Fictional&nbsp;Private&#32;Customer in HTML-encoded text.',
            'Address: 123 Fictional Lane.',
        ]
        for content in examples:
            with self.subTest(content=content):
                path = self.put('inputs/example.txt', content)
                result = self.audit.audit([path])
                self.assertFalse(result['passed'])
                self.assertEqual(result['issues'], [
                    'inputs/example.txt: rejected private-data fingerprint'])

    def test_whitespace_only_and_duplicate_rules_are_ignored(self) -> None:
        """Blank terms add no rule, and spacing or case variants add one rule only."""
        self.audit.deny_terms([' ', '', 'Fictional Customer', ' fictional\t customer '])
        self.assertEqual(len(self.audit.DENIED), 1)

    def test_partial_words_do_not_match_but_punctuation_and_underscores_do(self) -> None:
        """Only complete identifiers match; punctuation and underscores separate words."""
        self.audit.deny_terms(['Fictional Customer', 'studio-nas'])
        safe = self.put('inputs/safe.txt', 'Nonfictional Customer and studio-nascent.')
        self.assertTrue(self.audit.audit([safe])['passed'])
        denied = self.put('inputs/denied.txt',
                          'prefix_Fictional Customer_suffix; smb://studio-nas/share')
        self.assertFalse(self.audit.audit([denied])['passed'])

    def test_unicode_identifiers_are_supported(self) -> None:
        """Accented identifiers match case-insensitively."""
        self.audit.deny_terms(['Fictício Cliente'])
        path = self.put('inputs/example.txt', 'Example: FICTÍCIO CLIENTE.')
        self.assertFalse(self.audit.audit([path])['passed'])

    def test_rust_sources_manifests_and_lockfiles_are_audited(self) -> None:
        """The native workspace's .rs, .toml and .lock files are read as text."""
        self.audit.deny_terms(['Fictional Private Customer'])
        for suffix in ('.rs', '.toml', '.lock'):
            with self.subTest(suffix=suffix):
                path = self.put('native/example' + suffix, 'Fictional Private Customer')
                result = self.audit.audit([path])
                self.assertFalse(result['passed'])
                self.assertEqual(result['uniqueTextFiles'], 1)

    def test_rust_build_outputs_are_pruned_before_reading(self) -> None:
        """native/target is never entered, so build output cannot fail the audit."""
        self.audit.deny_terms(['Fictional Private Customer'])
        self.put('native/Cargo.toml', '[workspace]')
        self.put('native/target/debug/build/output.rs', 'Fictional Private Customer')
        seen = []
        real_walk = os.walk

        def walk(*args: Any, **kwargs: Any) -> Iterator[tuple[str, list[str], list[str]]]:
            for directory, children, names in real_walk(*args, **kwargs):
                seen.append(Path(directory).relative_to(self.root).as_posix())
                yield directory, children, names

        with chdir(self.root), patch.object(self.audit.os, 'walk', side_effect=walk):
            result = self.audit.audit([Path('native')])
        self.assertTrue(result['passed'], result['issues'])
        self.assertEqual(result['uniqueTextFiles'], 1)
        self.assertNotIn('native/target', seen)

    def test_symlinked_input_paths_are_reported_relative_to_the_repository(self) -> None:
        """A path through a symlink still names the file by its repository path."""
        self.audit.deny_terms(['Fictional Private Customer'])
        self.put('inputs/example.txt', 'Contains Fictional Private Customer.')
        with tempfile.TemporaryDirectory(prefix='openxplorer-link-test-') as directory:
            link = Path(directory) / 'repository'
            link.symlink_to(self.root, target_is_directory=True)
            result = self.audit.audit([link / 'inputs/example.txt'])
        self.assertEqual(result['issues'], [
            'inputs/example.txt: rejected private-data fingerprint'])

    def test_each_filename_is_checked_even_when_payloads_match(self) -> None:
        """A file whose content was already audited still has its own name checked."""
        self.audit.deny_terms(['privatecustomer'])
        ordinary = self.put('inputs/ordinary.txt', 'Identical fictional payload.\n')
        private = self.put('inputs/privatecustomer.txt', ordinary.read_bytes())
        for paths in ([ordinary, private], [private, ordinary]):
            with self.subTest(paths=[path.name for path in paths]):
                result = self.audit.audit(paths)
                self.assertFalse(result['passed'])
                self.assertEqual(result['uniqueTextFiles'], 1)
                self.assertEqual(result['issues'], [
                    'inputs/privatecustomer.txt: rejected filename fingerprint'])

    def test_duplicate_payloads_in_nested_archives_still_check_private_names(self) -> None:
        """Entries in nested archives get their names checked even when payloads repeat."""
        self.audit.deny_terms(['Fictional Private Customer'])
        payload = 'Identical fictional payload.\n'
        inner = self.root / 'inner.zip'
        with zipfile.ZipFile(inner, 'w') as archive:
            archive.writestr('ordinary.txt', payload)
            archive.writestr('Fictional%20Private%20Customer.txt', payload)
        outer = self.root / 'outer.zip'
        with zipfile.ZipFile(outer, 'w') as archive:
            archive.write(inner, 'inner.zip')
        result = self.audit.audit([outer])
        self.assertFalse(result['passed'])
        self.assertEqual(result['uniqueArchives'], 2)
        self.assertEqual(result['issues'], [
            'outer.zip!/inner.zip!/Fictional%20Private%20Customer.txt: '
            'rejected filename fingerprint'])

    def test_environment_rules_include_multiword_identifiers(self) -> None:
        """OX_PRIVATE_TERMS is split on commas only, so names keep their spaces."""
        terms = {'OX_PRIVATE_TERMS': 'Fictional Private Customer,studio-nas'}
        with patch.dict(os.environ, terms):
            audit = load_audit()
        self.assertTrue(audit.contains_private_term('FICTIONAL PRIVATE CUSTOMER'))
        self.assertTrue(audit.contains_private_term('smb://studio-nas/share'))
        self.assertEqual(len(audit.DENIED), 2)

    def test_cli_loads_external_multiword_rules_and_rejects_the_file(self) -> None:
        """--private-terms reads a JSON list kept outside the repository."""
        script = self.put('tools/audit-public-data.py', AUDIT_PATH.read_bytes())
        path = self.put('inputs/example.txt', 'Contains Fictional Private Customer.')
        with tempfile.TemporaryDirectory(prefix='openxplorer-denylist-test-') as directory:
            denylist = Path(directory) / 'private-terms.json'
            denylist.write_text(json.dumps(['Fictional Private Customer']))
            result = subprocess.run(
                [sys.executable, str(script), '--private-terms', str(denylist), str(path)],
                capture_output=True, text=True, check=False,
                env=dict(os.environ, OX_PRIVATE_TERMS=''))
        self.assertEqual(result.returncode, 1, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['privateIdentifierRules'], 1)
        self.assertEqual(report['issues'], [
            'inputs/example.txt: rejected private-data fingerprint'])


if __name__ == '__main__':
    unittest.main()
