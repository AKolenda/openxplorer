# SPDX-License-Identifier: AGPL-3.0-only
"""Load and validate bridge.json, the Python bridge operation inventory.

Every operation that desktop/winspace.py dispatches must appear exactly
once, with a status and a note. A tested status must cite the Rust tests
that prove it, each as ``path/to/file.rs::test_name`` relative to the
repository root. README.md in this directory explains the statuses.
"""
from __future__ import annotations

from collections.abc import Iterable
import json
from pathlib import Path
import re
from typing import Any, TypeAlias

from dispatch import bridge_methods

BRIDGE = 'native/parity/bridge.json'
LEGACY_DISPATCHER = 'desktop/winspace.py'
STATUSES = frozenset({'pending', 'core-tested', 'native-tested'})
CITATION = re.compile(r'(?P<path>[^:]+\.rs)::(?P<test>[A-Za-z_]\w*)')
RUST_FUNCTION = re.compile(
    r'(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>\w+)')

Inventory: TypeAlias = dict[str, Any]


def load(root: Path) -> Inventory:
    """Return the parsed bridge.json of a repository.

    Raise ValueError if the file is not a JSON object;
    json.JSONDecodeError is itself a ValueError.
    """
    data = json.loads((root / BRIDGE).read_text(encoding='utf-8'))
    if not isinstance(data, dict):
        raise ValueError('expected a JSON object.')
    return data


def validate(root: Path, inventory: Inventory) -> list[str]:
    """Return every problem with the inventory, or an empty list.

    The inventory must list exactly the operations that the legacy
    dispatcher handles, so none can be dropped or forgotten while
    porting.
    """
    methods = inventory.get('methods')
    if inventory.get('schema') != 1 or not isinstance(methods, dict):
        return ['Expected schema 1 with a methods object.']
    errors = coverage_problems(root, methods.keys())
    for name, entry in methods.items():
        errors += [f'{name}: {problem}'
                   for problem in entry_problems(root, entry)]
    return errors


def coverage_problems(root: Path, listed: Iterable[str]) -> list[str]:
    """Return operations dispatched but not listed, and listed but gone.

    An unreadable dispatcher is reported as a problem rather than
    raised, so the message names the file to fix.
    """
    source = (root / LEGACY_DISPATCHER).read_text(encoding='utf-8')
    try:
        actual = bridge_methods(source)
    except ValueError as error:
        return [f'{LEGACY_DISPATCHER}: {error}']
    listed = set(listed)
    errors = [f'Untracked bridge operation: {name}'
              for name in sorted(actual - listed)]
    errors += [f'Stale bridge operation: {name}'
               for name in sorted(listed - actual)]
    return errors


def entry_problems(root: Path, entry: object) -> list[str]:
    """Return the problems with one entry: status, note and tests."""
    if not isinstance(entry, dict):
        return ['expected an object']
    problems = []
    if entry.get('status') not in STATUSES:
        problems.append('invalid status')
    note = entry.get('note')
    if not isinstance(note, str) or not note.strip():
        problems.append('explain the remaining work or verified behavior')
    return problems + evidence_problems(root, entry)


def evidence_problems(root: Path, entry: dict[str, Any]) -> list[str]:
    """Return the problems with the tests that an entry cites.

    A pending entry may cite none. A tested status must cite at least
    one test, because the status alone proves nothing.
    """
    evidence = entry.get('evidence', [])
    if not isinstance(evidence, list) or not all(
            isinstance(citation, str) for citation in evidence):
        return ['evidence must be a list of "path.rs::test_name" strings']
    if entry.get('status') != 'pending' and not evidence:
        return ['tested status requires evidence']
    problems = []
    for citation in evidence:
        problem = citation_problem(root, citation)
        if problem:
            problems.append(problem)
    return problems


def citation_problem(root: Path, citation: str) -> str | None:
    """Return why a cited test cannot be found, or None if it exists.

    The file must be inside the repository, so evidence cannot point
    at a test that the native checks never run.
    """
    match = CITATION.fullmatch(citation)
    if not match:
        return ('evidence must name a Rust test as path.rs::test_name: '
                f'{citation}')
    path = (root / match['path']).resolve()
    if not path.is_relative_to(root.resolve()) or not path.is_file():
        return f'missing or nonrepository evidence: {citation}'
    if match['test'] not in rust_tests(path.read_text(encoding='utf-8')):
        return f'no #[test] fn {match["test"]} in {match["path"]}'
    return None


def rust_tests(source: str) -> set[str]:
    """Return the names of the functions that run as tests in Rust code.

    Other attributes and comments may stand between ``#[test]`` and the
    ``fn`` line; anything else ends the attribute block. A test marked
    ``#[ignore]`` is left out, because ``cargo test`` skips it.
    """
    tests = set()
    attributes: list[str] = []
    for line in source.splitlines():
        text = line.strip()
        function = RUST_FUNCTION.match(text)
        if text.startswith('#['):
            attributes.append(text)
        elif function and is_running_test(attributes):
            tests.add(function['name'])
        if not text.startswith(('#[', '//')):
            attributes = []
    return tests


def is_running_test(attributes: list[str]) -> bool:
    """Return whether attributes make a function a test that runs."""
    ignored = any(attribute.startswith('#[ignore') for attribute in attributes)
    return '#[test]' in attributes and not ignored


def replacement_blockers(inventory: Inventory) -> list[str]:
    """Return the operations that are not yet proven in the native app.

    Core-only coverage cannot authorize replacing a working desktop
    feature, so everything short of native-tested blocks replacement.
    """
    return sorted(name for name, entry in inventory['methods'].items()
                  if entry['status'] != 'native-tested')
