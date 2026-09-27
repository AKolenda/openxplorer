#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Detect untracked legacy bridge behavior and prevent premature replacement claims.

This inventories Python bridge operations, not every UI interaction or Dolphin
feature. ROADMAP.md tracks those broader requirements. Evidence paths are checked
for staleness; the native test runner must execute the tests separately.
"""
from __future__ import annotations

import argparse
import ast
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATUSES = frozenset({'pending', 'core-tested', 'native-tested'})


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-replacement', action='store_true',
                        help='also fail if any legacy bridge workflow lacks native verification')
    args = parser.parse_args()
    inventory = json.loads((ROOT / 'native/parity/bridge.json').read_text())
    errors = validate(ROOT, inventory)
    if errors:
        print('\n'.join(errors))
        return 1
    print(f"Legacy bridge inventory: {len(inventory['methods'])} operations accounted for.")
    blockers = replacement_blockers(inventory)
    print(f'{len(blockers)} still require native workflow verification before replacement.')
    print('This does not certify complete UI parity, Dolphin parity, or device hardware behavior.')
    if args.require_replacement and blockers:
        print('Replacement blocked: ' + ', '.join(blockers))
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
