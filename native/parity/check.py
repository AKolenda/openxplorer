#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check the parity inventories and gate replacement of the Python app.

Two inventories live in native/parity. bridge.json tracks every
operation of the Python bridge. features.toml lists every behaviour the
native app must provide: the current app's behaviours, the Dolphin
baseline and GNOME integration. This script validates both, checks that
parity markers in native tests name real features, and applies the
replacement gates. README.md explains the process.

Cited tests are checked for existence only; the native test runner in
native/tools/check.py executes them.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import bridge
import features as feature_inventory
import legacy
from legacy import Catalog
import markers as parity_markers

ROOT = Path(__file__).resolve().parents[2]
FEATURES = 'native/parity/features.toml'
LIMITS = ('"done" requires a parity marker on a native test. Statuses do '
          'not certify SMB, phone hardware or assistive-technology '
          'behaviour.')


@dataclass(frozen=True)
class Inventories:
    """Both inventories as loaded, and every problem found in them."""

    bridge_inventory: bridge.Inventory
    features: list[dict[str, Any]]
    catalog: Catalog
    errors: list[str]


def parse_arguments() -> argparse.Namespace:
    """Parse the options that add reports and replacement gates."""
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        '--require-replacement', action='store_true',
        help='also fail if any legacy bridge workflow lacks native '
             'verification')
    parser.add_argument(
        '--gate', action='append', default=[],
        choices=sorted(feature_inventory.GATES),
        help='fail unless every feature this gate covers is done or n-a: '
             '"replace" covers existing OpenXplorer behaviour, "dolphin" '
             'covers the Dolphin must-haves (repeatable)')
    parser.add_argument(
        '--python-tests', action='store_true',
        help='list the Python app\'s tests that no feature cites')
    return parser.parse_args()


def check_inventories(root: Path) -> Inventories:
    """Load both inventories of a repository and every problem in them.

    A file that cannot be parsed is reported as a problem, not raised,
    so the output names the file to fix.
    """
    try:
        python_app = legacy.load(root)
    except ValueError as error:  # json.JSONDecodeError is a ValueError.
        return Inventories({}, [], Catalog([]), [f'{legacy.LEGACY}: {error}'])
    try:
        bridge_inventory = bridge.load(root)
    except ValueError as error:
        return Inventories({}, [], Catalog([]), [f'{bridge.BRIDGE}: {error}'])
    errors = bridge.validate(root, bridge_inventory, python_app.bridge_operations)
    catalog = python_app.catalog
    try:
        features = feature_inventory.load(root / FEATURES)
    except ValueError as error:  # tomllib.TOMLDecodeError is a ValueError.
        errors.append(f'{FEATURES}: {error}')
        return Inventories(bridge_inventory, [], catalog, errors)
    found_markers, marker_errors = parity_markers.scan(root)
    errors += marker_errors
    operations = set(bridge_inventory.get('methods', {}))
    errors += feature_inventory.validate(
        features, operations, catalog, found_markers)
    return Inventories(bridge_inventory, features, catalog, errors)


def print_status(inventories: Inventories) -> None:
    """Print how much of each inventory the native app covers."""
    operations = len(inventories.bridge_inventory['methods'])
    blockers = bridge.replacement_blockers(inventories.bridge_inventory)
    print(f'Legacy bridge inventory: {operations} operations accounted for.')
    print(f'{len(blockers)} still require native workflow verification '
          'before replacement.')
    print(f'Feature inventory: {len(inventories.features)} behaviours, '
          'native status by area:')
    for line in feature_inventory.summary(inventories.features):
        print('  ' + line)
    print(LIMITS)


def report_unreferenced_tests(features: list[dict[str, Any]],
                              catalog: Catalog) -> None:
    """Print the Python app's tests that no feature cites, in file order."""
    cited = {reference
             for feature in features
             for reference in feature['python_tests']}
    unreferenced = catalog.unreferenced(cited)
    print(f'{len(unreferenced)} of {len(catalog.tests)} Python app tests are '
          'not cited by any feature:')
    for test in unreferenced:
        print(f'  {test.reference}')


def replacement_blocked(inventory: bridge.Inventory) -> bool:
    """Print and return whether bridge operations block replacement."""
    blockers = bridge.replacement_blockers(inventory)
    if blockers:
        print('Replacement blocked: ' + ', '.join(blockers))
    return bool(blockers)


def gate_blocked(gate: str, features: list[dict[str, Any]]) -> bool:
    """Print and return whether any feature blocks the named gate."""
    blockers = feature_inventory.GATES[gate](features)
    print(f'Gate "{gate}": {len(blockers)} features are neither done nor '
          'n-a.')
    if blockers:
        print('  ' + ', '.join(blockers))
    return bool(blockers)


def main() -> int:
    """Validate the inventories, print reports and apply the gates."""
    args = parse_arguments()
    inventories = check_inventories(ROOT)
    if inventories.errors:
        print('\n'.join(inventories.errors))
        return 1
    print_status(inventories)
    if args.python_tests:
        report_unreferenced_tests(inventories.features, inventories.catalog)
    failed = False
    if args.require_replacement:
        failed = replacement_blocked(inventories.bridge_inventory)
    for gate in args.gate:
        failed = gate_blocked(gate, inventories.features) or failed
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
