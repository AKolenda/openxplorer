# SPDX-License-Identifier: AGPL-3.0-only
"""Regression tests for the parity tooling.

They cover inventory drift, the frozen record of the Python app, cited
Rust tests, Python test citations, parity markers and the gates.
"""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import textwrap
from typing import Any
import unittest

import bridge
import check
import features
import legacy
from legacy import Catalog, LegacyTest
import markers


def temporary_root(test: unittest.TestCase) -> Path:
    """Return an empty directory that is removed after the test."""
    directory = tempfile.TemporaryDirectory()
    test.addCleanup(directory.cleanup)
    return Path(directory.name)


def write(path: Path, text: str) -> None:
    """Write dedented text to a file, creating parent directories."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text).lstrip(), encoding='utf-8')


def legacy_record(operations: list[str], tests: list[list[Any]]) -> str:
    """Return the text of a legacy.json with these operations and tests."""
    return json.dumps({'tag': legacy.TAG, 'bridge_operations': operations,
                       'tests': tests})


CITED_FILE = 'v2.0.0:desktop/tests/ui_a.py'


def pending() -> dict[str, Any]:
    """Return a bridge.json entry for an operation not ported yet."""
    return {'status': 'pending', 'note': 'Not implemented.', 'evidence': []}


class BridgeInventoryTests(unittest.TestCase):
    """bridge.json must match the legacy operations and cite real tests."""

    def setUp(self) -> None:
        """Create a repository with three operations and a Rust test."""
        self.root = temporary_root(self)
        self.operations = frozenset({'list', 'normalise', 'open'})
        write(self.root / 'native/tests.rs', '''
            #[test]
            fn lists_a_folder() {}
        ''')
        self.inventory: dict[str, Any] = {'schema': 1, 'methods': {
            name: pending() for name in ('list', 'normalise', 'open')}}

    def problems(self) -> list[str]:
        """Return the problems bridge.validate finds in the fixture."""
        return bridge.validate(self.root, self.inventory, self.operations)

    def claim(self, name: str, *evidence: str) -> None:
        """Record an operation as core-tested with the given tests."""
        self.inventory['methods'][name].update(
            status='core-tested', evidence=list(evidence))

    def test_a_complete_inventory_passes(self) -> None:
        """Every legacy operation is listed exactly once."""
        self.assertEqual(self.problems(), [])

    def test_added_or_removed_operations_cannot_disappear(self) -> None:
        """Operations missing from either side are both reported."""
        del self.inventory['methods']['open']
        self.inventory['methods']['old'] = pending()
        self.assertEqual(self.problems(), ['Untracked bridge operation: open',
                                           'Stale bridge operation: old'])

    def test_a_tested_status_needs_a_cited_test(self) -> None:
        """A tested status without evidence is refused."""
        self.claim('list')
        self.assertEqual(self.problems(),
                         ['list: tested status requires evidence'])
        self.claim('list', 'native/tests.rs::lists_a_folder')
        self.assertEqual(self.problems(), [])

    def test_citations_must_name_an_existing_test(self) -> None:
        """Each citation names a #[test] fn in a file that exists."""
        cases = {
            'no test name': (
                'native/tests.rs',
                'list: evidence must name a Rust test as '
                'path.rs::test_name: native/tests.rs'),
            'missing file': (
                'native/missing.rs::lists_a_folder',
                'list: missing or nonrepository evidence: '
                'native/missing.rs::lists_a_folder'),
            'missing test': (
                'native/tests.rs::renames_a_file',
                'list: no #[test] fn renames_a_file in native/tests.rs'),
        }
        for name, (citation, problem) in cases.items():
            with self.subTest(name):
                self.claim('list', citation)
                self.assertEqual(self.problems(), [problem])

    def test_evidence_must_be_a_list_of_strings(self) -> None:
        """Evidence of the wrong type is reported, not iterated."""
        self.inventory['methods']['list']['evidence'] = 'native/tests.rs'
        self.assertEqual(self.problems(), [
            'list: evidence must be a list of "path.rs::test_name" strings'])

    def test_evidence_cannot_escape_the_repository(self) -> None:
        """A real test outside the repository is not evidence."""
        outside = temporary_root(self) / 'outside.rs'
        write(outside, '''
            #[test]
            fn lists_a_folder() {}
        ''')
        relative = outside.relative_to(self.root, walk_up=True).as_posix()
        citation = f'{relative}::lists_a_folder'
        self.claim('list', citation)
        self.assertEqual(self.problems(), [
            f'list: missing or nonrepository evidence: {citation}'])

    def test_core_tests_do_not_pass_the_replacement_gate(self) -> None:
        """Only native-tested operations unblock replacement."""
        for name in self.inventory['methods']:
            self.claim(name, 'native/tests.rs::lists_a_folder')
        self.assertEqual(self.problems(), [])
        self.assertEqual(bridge.replacement_blockers(self.inventory),
                         ['list', 'normalise', 'open'])
        self.inventory['methods']['open']['status'] = 'native-tested'
        self.assertEqual(bridge.replacement_blockers(self.inventory),
                         ['list', 'normalise'])

    def test_bad_schema_and_status_are_reported(self) -> None:
        """A missing schema and an unknown status are both refused."""
        self.assertEqual(bridge.validate(self.root, {}, self.operations),
                         ['Expected schema 1 with a methods object.'])
        self.inventory['methods']['list']['status'] = 'done-ish'
        self.assertIn('list: invalid status', self.problems())

class RustTestDiscoveryTests(unittest.TestCase):
    """Only functions that ``cargo test`` runs count as evidence."""

    def test_only_running_test_functions_are_found(self) -> None:
        """Comments and attributes may follow #[test]; #[ignore] not."""
        source = textwrap.dedent('''
            /// Ported from v2.0.0:desktop/tests/test_core.py::CoreTests::test_a
            #[test]
            // parity: NAV-001
            fn with_a_comment_between() {}

            #[test]
            #[cfg(unix)]
            fn with_another_attribute() {}

            /// parity: OPS-001
            #[gtk::test]
            fn a_window_test() {}

            #[test]
            #[ignore]
            fn ignored() {}

            #[ignore = "slow"]
            #[test]
            fn ignored_before_the_test_attribute() {}

            #[test]

            fn after_a_blank_line() {}

            fn helper() {}

            mod tests {
                #[test]
                pub fn indented_and_public() {}

                #[gtk::test]
                fn on_the_gtk_test_thread() {}
            }

            #[gtk::test]
            fn on_the_gtk_test_thread() {}
        ''')
        self.assertEqual(bridge.rust_tests(source), {
            'with_a_comment_between', 'with_another_attribute',
            'a_window_test', 'indented_and_public', 'on_the_gtk_test_thread'})


def valid_feature(**changes: Any) -> dict[str, Any]:
    """Return a feature that passes validation, with changed fields.

    Documented keys keep their documented order; unknown keys follow.
    """
    feature = {
        'id': 'NAV-001', 'area': 'NAV', 'title': 'Back returns',
        'behaviour': 'Back opens the previous location of the active tab.',
        'origin': ['openxplorer', 'dolphin'], 'priority': 'must',
        'openxplorer': 'has', 'sources': ['v2.0.0:desktop/ui/app.js:351'],
        'python_tests': [f'{CITED_FILE}::Back works'],
        'bridge': ['list'], 'native': 'todo',
    }
    feature.update(changes)
    ordered = {key: feature[key] for key in features.KEYS if key in feature}
    return ordered | feature


class LegacyRecordTests(unittest.TestCase):
    """The frozen record of the Python app resolves citations."""

    def test_computed_label_parts_match_any_text_but_not_nothing(self) -> None:
        """A null part stands for text only known at run time."""
        test = LegacyTest(CITED_FILE, ('Menu includes ', None))
        self.assertEqual(test.name, 'Menu includes …')
        self.assertTrue(test.matches('Menu includes Open with…'))
        self.assertFalse(test.matches('Menu includes '))
        self.assertFalse(LegacyTest(CITED_FILE, ('Back works',)).matches(
            'Back works twice'))

    def test_catalog_resolves_citations_and_reports_uncited(self) -> None:
        """Citations need the right file and the full test name."""
        file = 'v2.0.0:desktop/tests/t.py'
        catalog = Catalog([LegacyTest(file, ('A::test_x',)),
                           LegacyTest(file, ('Label ', None))])
        self.assertEqual(len(catalog.find(f'{file}::A::test_x')), 1)
        self.assertEqual(catalog.find(f'{file}::test_x'), [])
        self.assertEqual(catalog.find('desktop/tests/t.py::A::test_x'), [],
                         'an untagged path names no test')
        uncited = catalog.unreferenced({f'{file}::Label 3'})
        self.assertEqual([test.name for test in uncited], ['A::test_x'])

    def test_a_malformed_record_is_refused(self) -> None:
        """Rows and operations of the wrong shape raise ValueError."""
        root = temporary_root(self)
        cases = {
            'no operations': legacy_record([], []),
            'a test without parts': legacy_record(['list'], [[CITED_FILE, []]]),
            'a part of the wrong type': legacy_record(['list'], [[CITED_FILE, [3]]]),
            'not an object': '[]',
        }
        for name, text in cases.items():
            with self.subTest(name):
                write(root / legacy.LEGACY, text)
                with self.assertRaises(ValueError):
                    legacy.load(root)


class FeatureValidationTests(unittest.TestCase):
    """Features are complete, consistent and cite real tests."""

    def setUp(self) -> None:
        """Provide a catalog holding the test valid_feature cites."""
        self.catalog = Catalog([LegacyTest(CITED_FILE, ('Back works',))])

    def problems(self, *items: dict[str, Any],
                 bridge_operations: frozenset[str] = frozenset({'list'}),
                 found_markers: dict[str, list[str]] | None = None,
                 ) -> list[str]:
        """Return the problems features.validate finds in the items."""
        return features.validate(list(items), set(bridge_operations),
                                 self.catalog, found_markers or {})

    def test_a_complete_feature_passes(self) -> None:
        """Required keys suffice, and optional keys are accepted."""
        self.assertEqual(self.problems(valid_feature()), [])
        self.assertEqual(self.problems(
            valid_feature(dolphin='Alt+Left', native_note='Planned.')), [])

    def test_ids_are_unique_well_formed_and_match_the_area(self) -> None:
        """Ids are permanent references, so their shape is enforced."""
        self.assertIn('NAV-001: duplicate id',
                      self.problems(valid_feature(), valid_feature()))
        self.assertTrue(any(
            'id must look like' in problem
            for problem in self.problems(valid_feature(id='NAV-1'))))
        self.assertTrue(any(
            'id prefix must equal' in problem
            for problem in self.problems(valid_feature(id='TAB-001'))))

    def test_keys_are_required_known_and_ordered(self) -> None:
        """Missing, unknown and reordered keys are each reported."""
        feature = valid_feature()
        del feature['native']
        self.assertIn("NAV-001: missing key 'native'", self.problems(feature))
        self.assertIn("NAV-001: unknown key 'notes'",
                      self.problems(valid_feature(notes='x')))
        reordered = dict(reversed(list(valid_feature().items())))
        self.assertIn('NAV-001: keys are not in the documented order',
                      self.problems(reordered))

    def test_an_id_of_the_wrong_type_is_reported_not_raised(self) -> None:
        """A list id is a structure problem, not a crash."""
        self.assertEqual(self.problems(valid_feature(id=['NAV-001'])),
                         ['feature #1: id must be a non-empty string'])

    def test_choices_are_enforced(self) -> None:
        """Enumerated fields accept only their documented values."""
        for key, value in [('priority', 'high'), ('openxplorer', 'yes'),
                           ('native', 'finished')]:
            with self.subTest(key):
                problems = self.problems(valid_feature(**{key: value}))
                self.assertTrue(any(
                    problem.startswith(f'NAV-001: {key} must be one of')
                    for problem in problems))

    def test_types_are_enforced(self) -> None:
        """Malformed lists and blank text are refused."""
        cases = {
            'unknown origin': valid_feature(origin=['kde']),
            'empty origin': valid_feature(origin=[]),
            'repeated bridge operation': valid_feature(bridge=['list'] * 2),
            'blank title': valid_feature(title='  '),
        }
        for name, feature in cases.items():
            with self.subTest(name):
                self.assertTrue(self.problems(feature))

    def test_existing_behaviour_is_a_must_from_openxplorer(self) -> None:
        """Rule 1: what the current app has is a "must"."""
        self.assertIn(
            'NAV-001: a behaviour OpenXplorer has must have priority "must"',
            self.problems(valid_feature(priority='could')))
        self.assertIn(
            'NAV-001: a behaviour OpenXplorer has must list "openxplorer" '
            'in origin',
            self.problems(valid_feature(origin=['dolphin'])))
        self.assertEqual(self.problems(
            valid_feature(openxplorer='missing', priority='could')), [])

    def test_not_applicable_needs_a_reason(self) -> None:
        """"n-a" is a product decision and must say why."""
        self.assertIn(
            'NAV-001: native = "n-a" requires a native_note explaining why',
            self.problems(valid_feature(native='n-a')))
        self.assertEqual(self.problems(
            valid_feature(native='n-a', native_note='WebKit only.')), [])

    def test_citations_exist_and_every_operation_is_cited(self) -> None:
        """Unknown citations and uncited operations are reported."""
        self.assertIn("NAV-001: unknown bridge operation 'lst'",
                      self.problems(valid_feature(bridge=['lst'])))
        broken = f'{CITED_FILE}::Back broke'
        self.assertIn(f'NAV-001: python test not found: {broken}',
                      self.problems(valid_feature(python_tests=[broken])))
        self.assertIn('Bridge operation open is not cited by any feature',
                      self.problems(valid_feature(),
                                    bridge_operations=frozenset({'list',
                                                                 'open'})))

    def test_done_needs_a_marker_and_a_marker_needs_a_status(self) -> None:
        """Markers and native statuses must agree in both directions."""
        place = {'NAV-001': ['native/a.rs:3']}
        self.assertIn(
            'NAV-001: native = "done" requires a parity marker on a native '
            'test',
            self.problems(valid_feature(native='done')))
        self.assertEqual(self.problems(valid_feature(native='done'),
                                       found_markers=place), [])
        self.assertEqual(self.problems(valid_feature(native='partial'),
                                       found_markers=place), [])
        self.assertTrue(any(
            'has a parity marker' in problem
            for problem in self.problems(valid_feature(),
                                         found_markers=place)))
        self.assertIn(
            'native/a.rs:3: parity marker names unknown feature TAB-009',
            self.problems(valid_feature(),
                          found_markers={'TAB-009': ['native/a.rs:3']}))

    def test_gates_block_until_done_or_not_applicable(self) -> None:
        """Each gate lists exactly the features it still waits for."""
        inventory = [
            valid_feature(id='NAV-001'),
            valid_feature(id='NAV-002', native='partial',
                          native_note='History model only.'),
            valid_feature(id='NAV-003', native='n-a',
                          native_note='WebKit only.'),
            valid_feature(id='NAV-004', origin=['dolphin'],
                          openxplorer='missing', native='todo'),
            valid_feature(id='NAV-005', origin=['dolphin'],
                          openxplorer='missing', priority='should'),
        ]
        self.assertEqual(features.replace_gate_blockers(inventory),
                         ['NAV-001', 'NAV-002'])
        self.assertEqual(features.dolphin_gate_blockers(inventory),
                         ['NAV-001', 'NAV-002', 'NAV-004'])

    def test_summary_counts_native_status_per_area(self) -> None:
        """Rows follow file order and end with the totals."""
        inventory = [valid_feature(),
                     valid_feature(id='NAV-002', native='partial'),
                     valid_feature(id='TAB-001', area='TAB')]
        lines = features.summary(inventory)
        self.assertEqual(lines[1].split(), ['NAV', '2', '1', '1', '0', '0'])
        self.assertEqual(lines[-1].split(), ['All', '3', '2', '1', '0', '0'])


class MarkerTests(unittest.TestCase):
    """Parity markers are read from native tests only."""

    def test_markers_come_from_native_sources_not_build_output(self) -> None:
        """Build output is skipped; malformed entries are reported."""
        root = temporary_root(self)
        write(root / 'native/crates/src/a.rs', '''
            #[test]
            // parity: NAV-001, TAB-002
            fn back() {}
            /* parity: nav-3 */
        ''')
        write(root / 'native/target/debug/b.rs', '// parity: SEL-001\n')
        write(root / 'native/ui-tests/flow.py', '# parity: SEL-002\n')
        write(root / 'native/tools/test_package.py', '# parity: UPD-017\n')
        write(root / 'native/tools/build.py', '# parity: UPD-018\n')
        found, errors = markers.scan(root)
        self.assertEqual(found, {'NAV-001': ['native/crates/src/a.rs:2'],
                                 'TAB-002': ['native/crates/src/a.rs:2'],
                                 'UPD-017': ['native/tools/test_package.py:1'],
                                 'SEL-002': ['native/ui-tests/flow.py:1']})
        self.assertEqual(errors, [
            "native/crates/src/a.rs:4: malformed parity marker entry 'nav-3'"
        ])


class RepositoryTests(unittest.TestCase):
    """The checked-in inventories pass; broken files are reported."""

    def test_checked_in_bridge_inventory_matches_the_legacy_bridge(self) -> None:
        """bridge.json lists exactly what the Python app's bridge handled."""
        inventory = bridge.load(check.ROOT)
        operations = legacy.load(check.ROOT).bridge_operations
        self.assertEqual(bridge.validate(check.ROOT, inventory, operations), [])

    def test_checked_in_feature_inventory_is_valid(self) -> None:
        """features.toml, the markers and the citations all agree."""
        inventories = check.check_inventories(check.ROOT)
        self.assertEqual(inventories.errors, [])
        self.assertTrue(inventories.features)
        self.assertEqual(len(inventories.catalog.tests), 1249)
        self.assertNotIn('desktop', {path.name for path in check.ROOT.iterdir()},
                         'the checks must not need the retired Python app')

    def test_a_broken_feature_file_is_reported_not_raised(self) -> None:
        """A TOML syntax error becomes one message naming the file."""
        root = self.minimal_repository()
        write(root / 'native/parity/features.toml', '''
            schema = 1
            [[feature]
        ''')
        errors = check.check_inventories(root).errors
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('native/parity/features.toml: '))

    def test_a_broken_legacy_record_is_reported_not_raised(self) -> None:
        """A malformed legacy.json becomes one message naming the file."""
        root = self.minimal_repository()
        write(root / legacy.LEGACY, '{"tag": "v2.0.0"}')
        errors = check.check_inventories(root).errors
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('native/parity/legacy.json: '))

    def test_a_broken_bridge_file_is_reported_not_raised(self) -> None:
        """A JSON syntax error becomes one message naming the file."""
        root = self.minimal_repository()
        write(root / 'native/parity/bridge.json', '{"schema": 1,')
        errors = check.check_inventories(root).errors
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('native/parity/bridge.json: '))

    def minimal_repository(self) -> Path:
        """Return a repository with legacy.json and bridge.json but no features.toml."""
        root = temporary_root(self)
        write(root / legacy.LEGACY, legacy_record(['list'], []))
        inventory = {'schema': 1, 'methods': {'list': pending()}}
        write(root / 'native/parity/bridge.json', json.dumps(inventory))
        return root


if __name__ == '__main__':
    unittest.main()
