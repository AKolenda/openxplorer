# SPDX-License-Identifier: AGPL-3.0-only
"""Discover the Python application's regression tests so features can cite them.

desktop/tests holds three kinds of test, each cited as ``<file>::<name>``:

- unittest methods: ``desktop/tests/test_core.py::CoreTests::test_unc``;
- labelled ``check(...)`` calls in the ui_*.py and native_*.py scripts:
  ``desktop/tests/ui_release.py::Back returns to the share``;
- labelled ``test(...)``/``check(...)`` calls in the Node suites (*.cjs).

Some labels are built at run time, such as ``'Menu includes ' + label``. Their
computed parts match any text, so a feature can cite the concrete label it
relies on ("Menu includes Open with…").
"""
from __future__ import annotations

import ast
from dataclasses import dataclass
from pathlib import Path
import re

TEST_DIRECTORY = 'desktop/tests'
COMPUTED = None  # Marks a label part that is only known at run time.
Part = str | None


@dataclass(frozen=True)
class DesktopTest:
    """One test: an exact name, or a label with computed parts."""

    file: str
    parts: tuple[Part, ...]

    @property
    def name(self) -> str:
        """Readable name; computed parts are shown as an ellipsis."""
        return ''.join('…' if part is COMPUTED else part for part in self.parts)

    @property
    def reference(self) -> str:
        """How a feature cites this test."""
        return f'{self.file}::{self.name}'

    def matches(self, name: str) -> bool:
        """Whether a cited name denotes this test."""
        pattern = ''.join('.+' if part is COMPUTED else re.escape(part) for part in self.parts)
        return re.fullmatch(pattern, name, re.DOTALL) is not None


def discover(root: Path) -> list[DesktopTest]:
    """Every test in desktop/tests, in file order."""
    tests = []
    for path in sorted((root / TEST_DIRECTORY).iterdir()):
        relative = path.relative_to(root).as_posix()
        if path.suffix == '.py':
            tests.extend(python_tests(path.read_text(), relative))
        elif path.suffix == '.cjs':
            tests.extend(node_tests(path.read_text(), relative))
    return tests


def python_tests(source: str, file: str) -> list[DesktopTest]:
    """unittest methods and labelled check() calls in one Python file."""
    tests = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.ClassDef):
            tests.extend(DesktopTest(file, (f'{node.name}::{item.name}',)) for item in node.body
                         if isinstance(item, ast.FunctionDef) and item.name.startswith('test'))
        elif (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
              and node.func.id == 'check' and node.args):
            tests.append(DesktopTest(file, merge_parts(python_label(node.args[0]))))
    return tests


def python_label(expression: ast.expr) -> list[Part]:
    """Literal and computed parts of a label expression."""
    if isinstance(expression, ast.Constant) and isinstance(expression.value, str):
        return [expression.value]
    if isinstance(expression, ast.BinOp) and isinstance(expression.op, ast.Add):
        return python_label(expression.left) + python_label(expression.right)
    if isinstance(expression, ast.JoinedStr):
        return [part.value if isinstance(part, ast.Constant) else COMPUTED
                for part in expression.values]
    return [COMPUTED]


# A test() or check() call; not a method such as regex.test(), nor the
# "function check(label, value)" definition of the helper itself.
NODE_CALL = re.compile(r'(?<![\w.])(?<!function )(?:test|check)\(')


def node_tests(source: str, file: str) -> list[DesktopTest]:
    """Labelled test() and check() calls in one Node suite."""
    return [DesktopTest(file, merge_parts(node_label(source, call.end())))
            for call in NODE_CALL.finditer(source)]


def node_label(source: str, start: int) -> list[Part]:
    """Parts of the first argument of a JavaScript call, up to its comma."""
    parts: list[Part] = []
    depth = 0
    index = start
    while index < len(source):
        character = source[index]
        if character in '\'"':
            end = string_end(source, index)
            parts.append(source[index + 1:end].replace('\\' + character, character))
            index = end + 1
            continue
        if character == '`':
            end = string_end(source, index)
            parts.extend(template_parts(source[index + 1:end]))
            index = end + 1
            continue
        if depth == 0 and character in ',)':
            break
        if character in '([{':
            depth += 1
        elif character in ')]}':
            depth -= 1
        if not character.isspace() and character != '+':
            parts.append(COMPUTED)
        index += 1
    return parts


def string_end(source: str, start: int) -> int:
    """Index of the quote closing the string literal that opens at start."""
    quote = source[start]
    index = start + 1
    while index < len(source) and source[index] != quote:
        index += 2 if source[index] == '\\' else 1
    return index


def template_parts(body: str) -> list[Part]:
    """Literal text and ${...} substitutions of a template literal."""
    parts: list[Part] = []
    for index, text in enumerate(re.split(r'\$\{[^}]*\}', body)):
        if index:
            parts.append(COMPUTED)
        if text:
            parts.append(text)
    return parts


def merge_parts(parts: list[Part]) -> tuple[Part, ...]:
    """Join adjacent literals and collapse runs of computed parts."""
    merged: list[Part] = []
    for part in parts:
        if merged and part is not COMPUTED and merged[-1] is not COMPUTED:
            merged[-1] += part
        elif not (merged and part is COMPUTED and merged[-1] is COMPUTED):
            merged.append(part)
    return tuple(merged) or (COMPUTED,)


class Catalog:
    """The discovered tests, indexed by file for resolving citations."""

    def __init__(self, tests: list[DesktopTest]):
        self.tests = tests
        self.by_file: dict[str, list[DesktopTest]] = {}
        for test in tests:
            self.by_file.setdefault(test.file, []).append(test)

    def find(self, reference: str) -> list[DesktopTest]:
        """Tests denoted by a citation such as 'desktop/tests/ui_v06.py::Label'."""
        file, separator, name = reference.partition('::')
        if not separator:
            return []
        return [test for test in self.by_file.get(file, []) if test.matches(name)]

    def unreferenced(self, references: set[str]) -> list[DesktopTest]:
        """Tests that no citation denotes."""
        cited = {id(test) for reference in references for test in self.find(reference)}
        return [test for test in self.tests if id(test) not in cited]
