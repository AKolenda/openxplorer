# SPDX-License-Identifier: AGPL-3.0-only
"""Publication privacy checks using fictional identifiers and temporary files."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


AUDIT_PATH = Path(__file__).resolve().parents[1] / 'tools/audit-public-data.py'


def load_audit():
    spec = importlib.util.spec_from_file_location('public_data_audit', AUDIT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PublicDataAuditTests(unittest.TestCase):
    def setUp(self):
        self.audit = load_audit()
        self.audit.DENIED.clear()
        self.temporary = tempfile.TemporaryDirectory(prefix='openxplorer-public-data-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.audit.ROOT = self.root
        # Supply isolated, valid provenance so unrelated repository captures do
        # not determine whether these privacy regression checks pass.
        source = self.put('desktop/ui/app.js', '// Synthetic fixture only.\n')
        pixel = base64.b64decode(
            'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF1kAAAAASUVORK5CYII=')
        screenshots = {}
        for number in range(7):
            name = f'fixture-{number}.png'
            self.put('apps/web/public/assets/screenshots/' + name, pixel)
            screenshots[name] = self.audit.digest(pixel)
        self.put('apps/web/public/assets/screenshots/manifest.json', json.dumps({
            'fixturePolicy': 'Fictional one-pixel test fixtures only.',
            'fixtureSourceSha256': self.audit.digest(source.read_bytes()),
            'sha256': screenshots,
        }))

    def put(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content if isinstance(content, bytes) else content.encode())
        return path

    def test_full_names_addresses_and_encoded_or_wrapped_text_are_rejected(self):
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

    def test_whitespace_only_and_duplicate_rules_are_ignored(self):
        self.audit.deny_terms([' ', '', 'Fictional Customer', ' fictional\t customer '])
        self.assertEqual(len(self.audit.DENIED), 1)

    def test_partial_words_do_not_match_but_punctuation_and_underscores_do(self):
        self.audit.deny_terms(['Fictional Customer', 'studio-nas'])
        safe = self.put('inputs/safe.txt', 'Nonfictional Customer and studio-nascent.')
        self.assertTrue(self.audit.audit([safe])['passed'])
        denied = self.put('inputs/denied.txt', 'prefix_Fictional Customer_suffix; smb://studio-nas/share')
        self.assertFalse(self.audit.audit([denied])['passed'])

    def test_unicode_identifiers_are_supported(self):
        self.audit.deny_terms(['Fictício Cliente'])
        path = self.put('inputs/example.txt', 'Example: FICTÍCIO CLIENTE.')
        self.assertFalse(self.audit.audit([path])['passed'])

    def test_each_filename_is_checked_even_when_payloads_match(self):
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

    def test_duplicate_payloads_in_nested_archives_still_check_private_names(self):
        self.audit.deny_terms(['Fictional Private Customer'])
        inner = self.root / 'inner.zip'
        with zipfile.ZipFile(inner, 'w') as archive:
            archive.writestr('ordinary.txt', 'Identical fictional payload.\n')
            archive.writestr('Fictional%20Private%20Customer.txt', 'Identical fictional payload.\n')
        outer = self.root / 'outer.zip'
        with zipfile.ZipFile(outer, 'w') as archive:
            archive.write(inner, 'inner.zip')
        result = self.audit.audit([outer])
        self.assertFalse(result['passed'])
        self.assertEqual(result['uniqueArchives'], 2)
        self.assertEqual(result['issues'], [
            'outer.zip!/inner.zip!/Fictional%20Private%20Customer.txt: rejected filename fingerprint'])

    def test_environment_rules_include_multiword_identifiers(self):
        with patch.dict(os.environ, {'OX_PRIVATE_TERMS': 'Fictional Private Customer,studio-nas'}):
            audit = load_audit()
        self.assertTrue(audit.contains_private_term('FICTIONAL PRIVATE CUSTOMER'))
        self.assertTrue(audit.contains_private_term('smb://studio-nas/share'))
        self.assertEqual(len(audit.DENIED), 2)

    def test_cli_loads_external_multiword_rules_and_rejects_the_file(self):
        script = self.put('tools/audit-public-data.py', AUDIT_PATH.read_bytes())
        path = self.put('inputs/example.txt', 'Contains Fictional Private Customer.')
        with tempfile.TemporaryDirectory(prefix='openxplorer-denylist-test-') as directory:
            denylist = Path(directory) / 'private-terms.json'
            denylist.write_text(json.dumps(['Fictional Private Customer']))
            result = subprocess.run(
                [sys.executable, str(script), '--private-terms', str(denylist), str(path)],
                capture_output=True, text=True,
                env=dict(os.environ, OX_PRIVATE_TERMS=''))
        self.assertEqual(result.returncode, 1, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['privateIdentifierRules'], 1)
        self.assertEqual(report['issues'], [
            'inputs/example.txt: rejected private-data fingerprint'])


if __name__ == '__main__':
    unittest.main()
