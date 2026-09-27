#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check the parity inventories and gate replacement of the Python application.

Two inventories live here. bridge.json tracks every operation of the Python
bridge. features.toml lists every behaviour the native app must provide: the
current app's behaviours, the Dolphin baseline and GNOME integration. This
script validates both, checks that parity markers in native tests name real
features, and applies the replacement gates. README.md explains the process.
Evidence and tests are checked for existence; the native test runner executes
the tests separately.
"""
from __future__ import annotations

import argparse
import ast
import json
from pathlib import Path

from desktop_tests import Catalog, discover
import features as feature_inventory
import markers as parity_markers

ROOT = Path(__file__).resolve().parents[2]
STATUSES = frozenset({'pending', 'core-tested', 'native-tested'})
FEATURES = 'native/parity/features.toml'


def is_method(node: ast.AST) -> bool:
    """Whether an expression names the dispatcher argument."""
    return isinstance(node, ast.Name) and node.id == 'method'


def pattern_methods(pattern: ast.pattern) -> set[str]:
    """Collect literal match cases; unsupported dynamic patterns fail closed."""
    if isinstance(pattern, ast.MatchAs) and pattern.pattern is None:
        return set()  # Default/capture case does not name an operation.
    if isinstance(pattern, ast.MatchOr):
        return set().union(*(pattern_methods(part) for part in pattern.patterns))
    if (isinstance(pattern, ast.MatchValue) and isinstance(pattern.value, ast.Constant)
            and isinstance(pattern.value.value, str)):
        return {pattern.value.value}
    raise ValueError(f'Nonliteral bridge match pattern at line {pattern.lineno}.')


def bridge_methods(source: str) -> set[str]:
    """Find named operations in the real dispatch function, including grouped branches."""
    dispatchers = [node for node in ast.walk(ast.parse(source))
                   if isinstance(node, ast.FunctionDef) and node.name == 'dispatch']
    if len(dispatchers) != 1:
        raise ValueError('Expected exactly one legacy dispatch function.')
    methods = set()
    for node in ast.walk(dispatchers[0]):
        if isinstance(node, ast.Match) and is_method(node.subject):
            for case in node.cases:
                methods.update(pattern_methods(case.pattern))
            continue
        if not isinstance(node, ast.Compare):
            continue
        if not any(is_method(part) for part in [node.left, *node.comparators]):
            continue
        if len(node.ops) != 1:
            raise ValueError(f'Chained bridge dispatch comparison at line {node.lineno}.')
        value = node.comparators[0]
        if isinstance(node.ops[0], (ast.NotEq, ast.NotIn)):
            continue  # Guard exclusions do not introduce dispatch operations.
        if isinstance(node.ops[0], ast.Eq) and is_method(value):
            value = node.left
        if isinstance(node.ops[0], ast.Eq) and isinstance(value, ast.Constant):
            values = [value]
        elif (is_method(node.left) and isinstance(node.ops[0], ast.In)
              and isinstance(value, (ast.Tuple, ast.List, ast.Set))):
            values = value.elts
        else:
            raise ValueError(f'Unsupported bridge dispatch comparison at line {node.lineno}.')
        if not all(isinstance(item, ast.Constant) and isinstance(item.value, str) for item in values):
            raise ValueError(f'Nonliteral bridge method at line {node.lineno}.')
        methods.update(item.value for item in values)
    if not methods:
        raise ValueError('No legacy bridge operations were discovered.')
    return methods


def validate(root: Path, inventory: dict) -> list[str]:
    """Check inventory completeness, explicit statuses and repository-local evidence."""
    errors = []
    if inventory.get('schema') != 1 or not isinstance(inventory.get('methods'), dict):
        return ['Expected schema 1 with a methods object.']
    actual = bridge_methods((root / 'desktop/winspace.py').read_text())
    methods = inventory['methods']
    for name in sorted(actual - methods.keys()):
        errors.append(f'Untracked bridge operation: {name}')
    for name in sorted(methods.keys() - actual):
        errors.append(f'Stale bridge operation: {name}')
    for name, entry in methods.items():
        if not isinstance(entry, dict):
            errors.append(f'{name}: expected an object')
            continue
        if entry.get('status') not in STATUSES:
            errors.append(f'{name}: invalid status')
        if not isinstance(entry.get('note'), str) or not entry['note'].strip():
            errors.append(f'{name}: explain the remaining work or verified behavior')
        evidence = entry.get('evidence', [])
        if not isinstance(evidence, list) or not all(isinstance(path, str) for path in evidence):
            errors.append(f'{name}: evidence must be a list of paths')
            continue
        if entry.get('status') != 'pending' and not evidence:
            errors.append(f'{name}: tested status requires evidence')
        for value in evidence:
            path = (root / value).resolve()
            if not path.is_relative_to(root.resolve()) or not path.is_file():
                errors.append(f'{name}: missing or nonrepository evidence: {value}')
    return errors


def replacement_blockers(inventory: dict) -> list[str]:
    """Core-only coverage cannot authorize replacing a working desktop feature."""
    return sorted(name for name, entry in inventory['methods'].items()
                  if entry['status'] != 'native-tested')


def parse_arguments() -> argparse.Namespace:
    """Command-line options: extra reports and the replacement gates."""
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--require-replacement', action='store_true',
                        help='also fail if any legacy bridge workflow lacks native verification')
    parser.add_argument('--gate', action='append', default=[],
                        choices=sorted(feature_inventory.GATES),
                        help='fail unless every feature this gate covers is done or n-a: '
                             '"replace" covers existing OpenXplorer behaviour, '
                             '"dolphin" covers the Dolphin must-haves (repeatable)')
    parser.add_argument('--python-tests', action='store_true',
                        help='list desktop/tests tests that no feature cites')
    return parser.parse_args()


def check_inventories(root: Path) -> tuple[dict, list[dict], Catalog, list[str]]:
    """Load both inventories and every problem found in them."""
    inventory = json.loads((root / 'native/parity/bridge.json').read_text())
    errors = validate(root, inventory)
    catalog = Catalog(discover(root))
    try:
        features = feature_inventory.load(root / FEATURES)
    except ValueError as error:  # tomllib.TOMLDecodeError is a ValueError.
        return inventory, [], catalog, errors + [f'{FEATURES}: {error}']
    markers, errors_in_markers = parity_markers.scan(root)
    errors += errors_in_markers
    operations = set(inventory.get('methods', {}))
    errors += feature_inventory.validate(features, operations, catalog, markers)
    return inventory, features, catalog, errors


def report_unreferenced_tests(features: list[dict], catalog: Catalog) -> None:
    """Print the desktop tests that no feature cites, in file order."""
    cited = {reference for feature in features for reference in feature['python_tests']}
    unreferenced = catalog.unreferenced(cited)
    total = len(catalog.tests)
    print(f'{len(unreferenced)} of {total} desktop tests are not cited by any feature:')
    for test in unreferenced:
        print(f'  {test.reference}')


def main() -> int:
    """Validate the inventories, print the reports and apply the requested gates."""
    args = parse_arguments()
    inventory, features, catalog, errors = check_inventories(ROOT)
    if errors:
        print('\n'.join(errors))
        return 1
    print(f"Legacy bridge inventory: {len(inventory['methods'])} operations accounted for.")
    blockers = replacement_blockers(inventory)
    print(f'{len(blockers)} still require native workflow verification before replacement.')
    print(f'Feature inventory: {len(features)} behaviours, native status by area:')
    print('\n'.join('  ' + line for line in feature_inventory.summary(features)))
    print('"done" requires a parity marker on a native test. Statuses do not certify SMB, '
          'phone hardware or assistive-technology behaviour.')
    if args.python_tests:
        report_unreferenced_tests(features, catalog)
    failed = False
    if args.require_replacement and blockers:
        print('Replacement blocked: ' + ', '.join(blockers))
        failed = True
    for gate in args.gate:
        gate_blockers = feature_inventory.GATES[gate](features)
        print(f'Gate "{gate}": {len(gate_blockers)} features are neither done nor n-a.')
        if gate_blockers:
            print('  ' + ', '.join(gate_blockers))
            failed = True
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
