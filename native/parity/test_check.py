# SPDX-License-Identifier: AGPL-3.0-only
"""Regression checks for inventory drift and a fail-closed replacement gate."""
import json
from pathlib import Path
import tempfile
import unittest

import check


class InventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / 'desktop').mkdir()
        (self.root / 'desktop/winspace.py').write_text('''
def dispatch(method):
    if method not in ('list', 'normalise'):
        return
    if method == 'list':
        return 1
    elif method in ('normalise', 'open'):
        return 2
''')
        (self.root / 'evidence.rs').write_text('// test evidence fixture\n')
        self.inventory = {'schema': 1, 'methods': {
            name: {'status': 'pending', 'note': 'Not implemented.', 'evidence': []}
            for name in ('list', 'normalise', 'open')
        }}

    def test_grouped_dispatch_and_guards_are_understood(self):
        self.assertEqual(check.validate(self.root, self.inventory), [])

    def test_reversed_comparisons_and_match_cannot_bypass_inventory(self):
        methods = check.bridge_methods('''
def dispatch(method):
    if 'list' == method: return
    match method:
        case 'newWorkflow' | 'anotherWorkflow': return
        case _: return
''')
        self.assertEqual(methods, {'list', 'newWorkflow', 'anotherWorkflow'})

    def test_added_or_removed_operations_cannot_silently_disappear(self):
        del self.inventory['methods']['open']
        self.inventory['methods']['old'] = {'status': 'pending', 'note': 'Removed.'}
        self.assertEqual(check.validate(self.root, self.inventory), [
            'Untracked bridge operation: open', 'Stale bridge operation: old'])

    def test_claimed_tested_status_needs_existing_evidence(self):
        entry = self.inventory['methods']['list']
        entry['status'] = 'native-tested'
        self.assertIn('list: tested status requires evidence', check.validate(self.root, self.inventory))
        entry['evidence'] = ['missing.rs']
        self.assertIn('list: missing or nonrepository evidence: missing.rs',
                      check.validate(self.root, self.inventory))
        entry['evidence'] = ['evidence.rs']
        self.assertEqual(check.validate(self.root, self.inventory), [])

    def test_evidence_cannot_escape_the_repository(self):
        self.inventory['methods']['list']['evidence'] = ['../outside.rs']
        self.assertTrue(check.validate(self.root, self.inventory))

    def test_core_tests_do_not_pass_the_replacement_gate(self):
        for entry in self.inventory['methods'].values():
            entry.update(status='core-tested', evidence=['evidence.rs'])
        self.assertEqual(check.validate(self.root, self.inventory), [])
        self.assertEqual(check.replacement_blockers(self.inventory), ['list', 'normalise', 'open'])
        self.inventory['methods']['open']['status'] = 'native-tested'
        self.assertEqual(check.replacement_blockers(self.inventory), ['list', 'normalise'])

    def test_bad_schema_status_and_nonliteral_dispatch_fail(self):
        self.assertTrue(check.validate(self.root, {}))
        self.inventory['methods']['list']['status'] = 'done-ish'
        self.assertIn('list: invalid status', check.validate(self.root, self.inventory))
        with self.assertRaises(ValueError):
            check.bridge_methods('def dispatch(method):\n    if method == operation: pass\n')
        with self.assertRaises(ValueError):
            check.bridge_methods('def no_dispatch(): pass\n')

    def test_checked_in_inventory_matches_the_legacy_application(self):
        inventory = json.loads((check.ROOT / 'native/parity/bridge.json').read_text())
        self.assertEqual(check.validate(check.ROOT, inventory), [])


if __name__ == '__main__':
    unittest.main()
