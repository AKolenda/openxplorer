# SPDX-License-Identifier: AGPL-3.0-only
"""The retired Python app's bridge operations and tests, as features cite them.

The Python/WebKit app of OpenXplorer 1.x was removed from the tree after
2.0.0; its last release is tag v1.1.4, and tag v2.0.0 holds its final
sources. legacy.json records what the parity checks need from those sources,
read from tag v2.0.0 when the app was removed:

- ``bridge_operations``: every operation ``dispatch`` in
  ``v2.0.0:desktop/winspace.py`` handled. bridge.json must list exactly
  these.
- ``tests``: every test in ``v2.0.0:desktop/tests``, as
  ``[file, parts]``. A part is literal label text, or null for a part of
  a label that was only known at run time, such as
  ``'Menu includes ' + label``; null matches any text, so a feature can cite
  the concrete label it relies on ("Menu includes Open with…"). Tests are
  unittest methods (``file::Class::test_method``) or the labels of
  ``check(...)`` and ``test(...)`` calls in the UI and Node suites
  (``file::Back returns to the share``).

The tag never changes, so neither does legacy.json. Features cite the
tests as ``v2.0.0:desktop/tests/<file>::<name>``.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import re
from typing import Final, TypeAlias

LEGACY = 'native/parity/legacy.json'
# The tag that holds the Python app's final sources.
TAG: Final = 'v2.0.0'

Part: TypeAlias = str | None


@dataclass(frozen=True)
class LegacyTest:
    """One test of the Python app: an exact name, or a label with computed parts."""

    file: str
    parts: tuple[Part, ...]

    @property
    def name(self) -> str:
        """Return a readable name; computed parts become an ellipsis."""
        return ''.join('…' if part is None else part for part in self.parts)

    @property
    def reference(self) -> str:
        """Return how a feature cites this test."""
        return f'{self.file}::{self.name}'

    def matches(self, name: str) -> bool:
        """Return whether a cited name denotes this test."""
        pattern = ''.join('.+' if part is None else re.escape(part)
                          for part in self.parts)
        return re.fullmatch(pattern, name, re.DOTALL) is not None


class Catalog:
    """The Python app's tests, indexed by file for resolving citations."""

    def __init__(self, tests: list[LegacyTest]) -> None:
        """Index the tests by the file that holds them."""
        self.tests = tests
        self.by_file: dict[str, list[LegacyTest]] = {}
        for test in tests:
            self.by_file.setdefault(test.file, []).append(test)

    def find(self, reference: str) -> list[LegacyTest]:
        """Return the tests a citation like 'file.py::Label' denotes."""
        file, separator, name = reference.partition('::')
        if not separator:
            return []
        return [test for test in self.by_file.get(file, [])
                if test.matches(name)]

    def unreferenced(self, references: set[str]) -> list[LegacyTest]:
        """Return the tests that no citation denotes, in file order."""
        cited = {test
                 for reference in references
                 for test in self.find(reference)}
        return [test for test in self.tests if test not in cited]


@dataclass(frozen=True)
class Legacy:
    """What the parity checks know of the Python app."""

    bridge_operations: frozenset[str]
    catalog: Catalog


def load(root: Path) -> Legacy:
    """Return legacy.json of a repository.

    Raise ValueError if it is not valid JSON in the documented layout;
    json.JSONDecodeError is itself a ValueError.
    """
    data = json.loads((root / LEGACY).read_text(encoding='utf-8'))
    operations = data.get('bridge_operations') if isinstance(data, dict) else None
    tests = data.get('tests') if isinstance(data, dict) else None
    if not isinstance(operations, list) or not isinstance(tests, list) or not operations:
        raise ValueError('expected bridge_operations and tests lists.')
    if not all(isinstance(operation, str) for operation in operations):
        raise ValueError('bridge_operations must be strings.')
    return Legacy(frozenset(operations), Catalog([parse_test(test) for test in tests]))


def parse_test(row: object) -> LegacyTest:
    """Return one ``[file, parts]`` row of legacy.json as a test."""
    match row:
        case [str() as file, list() as parts] if parts and all(
                part is None or isinstance(part, str) for part in parts):
            return LegacyTest(file, tuple(parts))
    raise ValueError(f'a test must be [file, parts]: {row!r}')
