# SPDX-License-Identifier: AGPL-3.0-only
"""Discover the Python app's regression tests so features can cite them.

desktop/tests holds three kinds of test, each cited as
``<file>::<name>``:

- unittest methods: ``desktop/tests/test_core.py::CoreTests::test_unc``;
- labelled ``check(...)`` calls in the ui_*.py and native_*.py scripts:
  ``desktop/tests/ui_release.py::Back returns to the share``;
- labelled ``test(...)`` and ``check(...)`` calls in the Node suites
  (*.cjs).

Some labels are built at run time, such as ``'Menu includes ' + label``.
Their computed parts match any text, so a feature can cite the concrete
label it relies on ("Menu includes Open with…").
"""
from __future__ import annotations

import ast
from dataclasses import dataclass
from pathlib import Path
import re
from typing import Final, TypeAlias, TypeGuard

TEST_DIRECTORY = 'desktop/tests'
# Marks a part of a label that is only known at run time.
COMPUTED: Final = None

Part: TypeAlias = str | None

# A test() or check() call in a Node suite. Methods such as regex.test()
# and the "function check(label, value)" helper definition do not count.
NODE_CALL = re.compile(r'(?<![\w.])(?<!function )(?:test|check)\(')


@dataclass(frozen=True)
class DesktopTest:
    """One test: an exact name, or a label with computed parts."""

    file: str
    parts: tuple[Part, ...]

    @property
    def name(self) -> str:
        """Return a readable name; computed parts become an ellipsis."""
        return ''.join('…' if part is COMPUTED else part
                       for part in self.parts)

    @property
    def reference(self) -> str:
        """Return how a feature cites this test."""
        return f'{self.file}::{self.name}'

    def matches(self, name: str) -> bool:
        """Return whether a cited name denotes this test."""
        pattern = ''.join('.+' if part is COMPUTED else re.escape(part)
                          for part in self.parts)
        return re.fullmatch(pattern, name, re.DOTALL) is not None


def discover(root: Path) -> list[DesktopTest]:
    """Return every test in desktop/tests, in file order."""
    tests = []
    for path in sorted((root / TEST_DIRECTORY).iterdir()):
        relative = path.relative_to(root).as_posix()
        if path.suffix == '.py':
            source = path.read_text(encoding='utf-8')
            tests += python_tests(source, relative)
        elif path.suffix == '.cjs':
            source = path.read_text(encoding='utf-8')
            tests += node_tests(source, relative)
    return tests


def python_tests(source: str, file: str) -> list[DesktopTest]:
    """Return the unittest methods and check() calls in a script."""
    tests = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.ClassDef):
            tests += [DesktopTest(file, (f'{node.name}::{method}',))
                      for method in test_methods(node)]
        elif is_check_call(node):
            label = python_label(node.args[0])
            tests.append(DesktopTest(file, merge_parts(label)))
    return tests


def test_methods(class_definition: ast.ClassDef) -> list[str]:
    """Return the names of the methods unittest would run in a class."""
    return [item.name for item in class_definition.body
            if isinstance(item, ast.FunctionDef)
            and item.name.startswith('test')]


def is_check_call(node: ast.AST) -> TypeGuard[ast.Call]:
    """Return whether a node calls the check() helper with a label."""
    return (isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id == 'check'
            and bool(node.args))


def python_label(expression: ast.expr) -> list[Part]:
    """Return the literal and computed parts of a label expression."""
    match expression:
        case ast.Constant(value=str() as text):
            return [text]
        case ast.BinOp(left=left, op=ast.Add(), right=right):
            return python_label(left) + python_label(right)
        case ast.JoinedStr(values=values):
            parts: list[Part] = []
            for value in values:  # Literal text or a {substitution}.
                parts += python_label(value)
            return parts
        case _:
            return [COMPUTED]


def node_tests(source: str, file: str) -> list[DesktopTest]:
    """Return the labelled test() and check() calls in a Node suite."""
    return [DesktopTest(file, merge_parts(node_label(source, call.end())))
            for call in NODE_CALL.finditer(source)]


def node_label(source: str, start: int) -> list[Part]:
    """Return the parts of a JavaScript call's first argument.

    Scanning starts just after the opening parenthesis and stops at the
    comma or parenthesis that ends the argument. String and template
    literals become literal parts; any other code becomes a computed
    part, except whitespace and the ``+`` that joins the parts.
    """
    parts: list[Part] = []
    depth = 0
    index = start
    while index < len(source):
        character = source[index]
        if character in '\'"`':
            end = string_end(source, index)
            parts += string_parts(character, source[index + 1:end])
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


def string_parts(quote: str, body: str) -> list[Part]:
    """Return the parts of a JavaScript string or template literal.

    ``body`` is the text between the quotes. Escaped quotes of the
    literal's own kind are unescaped; other escapes are kept as written.
    """
    if quote == '`':
        return template_parts(body)
    return [body.replace('\\' + quote, quote)]


def string_end(source: str, start: int) -> int:
    """Return the index of the quote closing the literal at start."""
    quote = source[start]
    index = start + 1
    while index < len(source) and source[index] != quote:
        index += 2 if source[index] == '\\' else 1
    return index


def template_parts(body: str) -> list[Part]:
    """Return the literal text and ${...} parts of a template."""
    parts: list[Part] = []
    for index, text in enumerate(re.split(r'\$\{[^}]*\}', body)):
        if index:
            parts.append(COMPUTED)
        if text:
            parts.append(text)
    return parts


def merge_parts(parts: list[Part]) -> tuple[Part, ...]:
    """Join adjacent literals and collapse runs of computed parts.

    A label without any parts is treated as computed, so it matches any
    name rather than none.
    """
    merged: list[Part] = []
    for part in parts:
        if not merged:
            merged.append(part)
            continue
        previous = merged[-1]
        if part is COMPUTED and previous is COMPUTED:
            continue  # One computed part already matches any text.
        if part is not COMPUTED and previous is not COMPUTED:
            merged[-1] = previous + part
        else:
            merged.append(part)
    return tuple(merged) or (COMPUTED,)


class Catalog:
    """The discovered tests, indexed by file for resolving citations."""

    def __init__(self, tests: list[DesktopTest]) -> None:
        """Index the tests by the file that holds them."""
        self.tests = tests
        self.by_file: dict[str, list[DesktopTest]] = {}
        for test in tests:
            self.by_file.setdefault(test.file, []).append(test)

    def find(self, reference: str) -> list[DesktopTest]:
        """Return the tests a citation like 'file.py::Label' denotes."""
        file, separator, name = reference.partition('::')
        if not separator:
            return []
        return [test for test in self.by_file.get(file, [])
                if test.matches(name)]

    def unreferenced(self, references: set[str]) -> list[DesktopTest]:
        """Return the tests that no citation denotes, in file order."""
        cited = {test
                 for reference in references
                 for test in self.find(reference)}
        return [test for test in self.tests if test not in cited]
