# SPDX-License-Identifier: AGPL-3.0-only
"""Regression checks for inventory drift, parity markers and fail-closed gates."""
import json
from pathlib import Path
import tempfile
import textwrap
import unittest

import check
from desktop_tests import Catalog, discover, node_tests, python_tests
import features
import markers


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


def valid_feature(**changes) -> dict:
    """A feature that passes validation; tests change one field at a time."""
    feature = {
        'id': 'NAV-001', 'area': 'NAV', 'title': 'Back returns',
        'behaviour': 'Back opens the previous location of the active tab.',
        'origin': ['openxplorer', 'dolphin'], 'priority': 'must', 'openxplorer': 'has',
        'sources': ['desktop/ui/app.js:351'], 'python_tests': ['desktop/tests/ui_a.py::Back works'],
        'bridge': ['list'], 'native': 'todo',
    }
    feature.update(changes)
    ordered = {key: feature[key] for key in features.KEYS if key in feature}
    return ordered | feature  # Unknown keys, if any, follow the documented ones.


class DesktopTestDiscoveryTests(unittest.TestCase):
    def test_unittest_methods_and_computed_check_labels_are_found(self):
        tests = python_tests(textwrap.dedent('''
            class CoreTests:
                def test_unc(self): pass
                def helper(self): pass
            def check(name, value=True): pass
            check('Back works')
            check('Menu includes ' + label)
            check(f'Moved to {window} safely')
        '''), 'desktop/tests/ui_a.py')
        self.assertEqual([test.name for test in tests], ['CoreTests::test_unc', 'Back works',
                                                         'Menu includes …', 'Moved to … safely'])
        self.assertTrue(tests[2].matches('Menu includes Open with…'))
        self.assertFalse(tests[2].matches('Menu includes '))
        self.assertFalse(tests[1].matches('Back works twice'))

    def test_node_labels_skip_the_helper_definition(self):
        tests = node_tests(textwrap.dedent('''
            function check(label, value) {assert.ok(value, label);}
            check('Enter opens it', true);
            for (const key of keys) test('Ctrl '+key, () => ok(key));
            test(`fits at ${size}%`, () => {});
            if (/x/.test(name)) {}
        '''), 'desktop/tests/a.cjs')
        self.assertEqual([test.name for test in tests], ['Enter opens it', 'Ctrl …', 'fits at …%'])

    def test_catalog_resolves_citations_and_reports_uncited_tests(self):
        tests = python_tests("class A:\n    def test_x(self): pass\ncheck('Label ' + n)\n",
                             'desktop/tests/t.py')
        catalog = Catalog(tests)
        self.assertEqual(len(catalog.find('desktop/tests/t.py::A::test_x')), 1)
        self.assertEqual(catalog.find('desktop/tests/t.py::test_x'), [])
        self.assertEqual(catalog.find('desktop/tests/other.py::A::test_x'), [])
        self.assertEqual([test.name for test in catalog.unreferenced({'desktop/tests/t.py::Label 3'})],
                         ['A::test_x'])


class FeatureValidationTests(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog(python_tests("check('Back works')\n", 'desktop/tests/ui_a.py'))

    def problems(self, *items, bridge=frozenset({'list'}), found_markers=None):
        return features.validate(list(items), set(bridge), self.catalog, found_markers or {})

    def test_a_complete_feature_passes(self):
        self.assertEqual(self.problems(valid_feature()), [])
        self.assertEqual(self.problems(valid_feature(dolphin='Alt+Left', native_note='Planned.')), [])

    def test_ids_are_unique_well_formed_and_match_their_area(self):
        self.assertIn('NAV-001: duplicate id', self.problems(valid_feature(), valid_feature()))
        self.assertTrue(any('id must look like' in problem
                            for problem in self.problems(valid_feature(id='NAV-1'))))
        self.assertTrue(any('id prefix must equal' in problem
                            for problem in self.problems(valid_feature(id='TAB-001'))))

    def test_keys_are_required_known_and_ordered(self):
        feature = valid_feature()
        del feature['native']
        self.assertIn("NAV-001: missing key 'native'", self.problems(feature))
        self.assertIn("NAV-001: unknown key 'notes'", self.problems(valid_feature(notes='x')))
        reordered = dict(reversed(list(valid_feature().items())))
        self.assertIn('NAV-001: keys are not in the documented order', self.problems(reordered))

    def test_choices_and_types_are_enforced(self):
        for key, value in [('priority', 'high'), ('openxplorer', 'yes'), ('native', 'finished')]:
            self.assertTrue(any(problem.startswith(f'NAV-001: {key} must be one of')
                                for problem in self.problems(valid_feature(**{key: value}))), key)
        self.assertTrue(self.problems(valid_feature(origin=['kde'])))
        self.assertTrue(self.problems(valid_feature(origin=[])))
        self.assertTrue(self.problems(valid_feature(bridge=['list', 'list'])))
        self.assertTrue(self.problems(valid_feature(title='  ')))

    def test_existing_behaviour_is_a_must_from_openxplorer(self):
        self.assertIn('NAV-001: a behaviour OpenXplorer has must have priority "must"',
                      self.problems(valid_feature(priority='could')))
        self.assertIn('NAV-001: a behaviour OpenXplorer has must list "openxplorer" in origin',
                      self.problems(valid_feature(origin=['dolphin'])))
        self.assertEqual(self.problems(valid_feature(openxplorer='missing', priority='could')), [])

    def test_not_applicable_needs_a_reason(self):
        self.assertIn('NAV-001: native = "n-a" requires a native_note explaining why',
                      self.problems(valid_feature(native='n-a')))
        self.assertEqual(self.problems(valid_feature(native='n-a', native_note='WebKit only.')), [])

    def test_citations_must_exist_and_every_bridge_operation_is_cited(self):
        self.assertIn("NAV-001: unknown bridge operation 'lst'",
                      self.problems(valid_feature(bridge=['lst'])))
        self.assertIn('NAV-001: python test not found: desktop/tests/ui_a.py::Back broke',
                      self.problems(valid_feature(python_tests=['desktop/tests/ui_a.py::Back broke'])))
        self.assertIn('Bridge operation open is not cited by any feature',
                      self.problems(valid_feature(), bridge={'list', 'open'}))

    def test_done_requires_a_marker_and_markers_require_a_tested_status(self):
        place = {'NAV-001': ['native/a.rs:3']}
        self.assertIn('NAV-001: native = "done" requires a parity marker on a native test',
                      self.problems(valid_feature(native='done')))
        self.assertEqual(self.problems(valid_feature(native='done'), found_markers=place), [])
        self.assertEqual(self.problems(valid_feature(native='partial'), found_markers=place), [])
        self.assertTrue(any('has a parity marker' in problem
                            for problem in self.problems(valid_feature(), found_markers=place)))
        self.assertIn('native/a.rs:3: parity marker names unknown feature TAB-009',
                      self.problems(valid_feature(), found_markers={'TAB-009': ['native/a.rs:3']}))

    def test_gates_block_until_done_or_not_applicable(self):
        inventory = [
            valid_feature(id='NAV-001'),
            valid_feature(id='NAV-002', native='partial', native_note='History model only.'),
            valid_feature(id='NAV-003', native='n-a', native_note='WebKit only.'),
            valid_feature(id='NAV-004', origin=['dolphin'], openxplorer='missing', native='todo'),
            valid_feature(id='NAV-005', origin=['dolphin'], openxplorer='missing', priority='should'),
        ]
        self.assertEqual(features.replace_gate_blockers(inventory), ['NAV-001', 'NAV-002'])
        self.assertEqual(features.dolphin_gate_blockers(inventory), ['NAV-001', 'NAV-002', 'NAV-004'])

    def test_summary_counts_native_status_per_area(self):
        inventory = [valid_feature(), valid_feature(id='NAV-002', native='partial'),
                     valid_feature(id='TAB-001', area='TAB')]
        lines = features.summary(inventory)
        self.assertEqual(lines[1].split(), ['NAV', '2', '1', '1', '0', '0'])
        self.assertEqual(lines[-1].split(), ['All', '3', '2', '1', '0', '0'])


class MarkerTests(unittest.TestCase):
    def test_markers_are_collected_from_native_sources_but_not_build_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'native/crates/src').mkdir(parents=True)
            (root / 'native/target/debug').mkdir(parents=True)
            (root / 'native/ui-tests').mkdir()
            (root / 'native/crates/src/a.rs').write_text(
                '#[test]\n// parity: NAV-001, TAB-002\nfn back() {}\n/* parity: nav-3 */\n')
            (root / 'native/target/debug/b.rs').write_text('// parity: SEL-001\n')
            (root / 'native/ui-tests/flow.py').write_text('# parity: SEL-002\n')
            found, errors = markers.scan(root)
        self.assertEqual(found, {'NAV-001': ['native/crates/src/a.rs:2'],
                                 'TAB-002': ['native/crates/src/a.rs:2'],
                                 'SEL-002': ['native/ui-tests/flow.py:1']})
        self.assertEqual(errors, ["native/crates/src/a.rs:4: malformed parity marker entry 'nav-3'"])


class RepositoryTests(unittest.TestCase):
    def test_checked_in_feature_inventory_is_valid(self):
        inventory, feature_list, catalog, errors = check.check_inventories(check.ROOT)
        self.assertEqual(errors, [])
        self.assertTrue(feature_list)
        self.assertEqual(len(catalog.tests), len(discover(check.ROOT)))

    def test_a_broken_feature_file_is_reported_not_raised(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'desktop/tests').mkdir(parents=True)
            (root / 'native/parity').mkdir(parents=True)
            (root / 'desktop/winspace.py').write_text(
                "def dispatch(method):\n    if method == 'list': pass\n")
            (root / 'native/parity/bridge.json').write_text(json.dumps({'schema': 1, 'methods': {
                'list': {'status': 'pending', 'note': 'Not implemented.', 'evidence': []}}}))
            (root / 'native/parity/features.toml').write_text('schema = 1\n[[feature]\n')
            _, _, _, errors = check.check_inventories(root)
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('native/parity/features.toml: '))


if __name__ == '__main__':
    unittest.main()
