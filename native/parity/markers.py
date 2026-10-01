# SPDX-License-Identifier: AGPL-3.0-only
"""Find parity markers: comments that tie a native test to its features.

A marker is a comment such as ``/// parity: NAV-001, TAB-004`` placed
in a test's doc comment in a Rust source under native/, in a packaging test
(native/tools/test_*.py, which native/tools/check.py runs), or in any file
under native/ui-tests/ once UI tests exist there. Only a feature named by a
marker may be recorded as natively done.
"""
from __future__ import annotations

from pathlib import Path
import re

FEATURE_ID = re.compile(r'[A-Z]{2,5}-\d{3}')
MARKER = re.compile(r'(?://|#|/\*|\*)\s*parity:(.*)')
SKIPPED_DIRECTORIES = frozenset({'target', '__pycache__'})


def marked_files(root: Path) -> list[Path]:
    """Return the files that may hold markers, in a stable order.

    These are the Rust sources under native/, the packaging tests in
    native/tools/ and every file under native/ui-tests/. Build output and
    hidden directories are skipped: they hold copies, not the tests that
    run.
    """
    native = root / 'native'
    ui_tests = native / 'ui-tests'
    tools = native / 'tools'
    files = []
    for directory, subdirectories, names in native.walk():
        subdirectories[:] = sorted(
            name for name in subdirectories
            if name not in SKIPPED_DIRECTORIES and not name.startswith('.'))
        for name in sorted(names):
            path = directory / name
            if (path.suffix == '.rs' or path.is_relative_to(ui_tests)
                    or is_packaging_test(path, tools)):
                files.append(path)
    return files


def is_packaging_test(path: Path, tools: Path) -> bool:
    """Return whether path is a packaging test module in native/tools."""
    return (path.parent == tools and path.name.startswith('test_')
            and path.suffix == '.py')


def scan(root: Path) -> tuple[dict[str, list[str]], list[str]]:
    """Return where each feature is marked, and the malformed markers.

    Locations are ``path:line`` strings relative to the repository root.
    """
    locations: dict[str, list[str]] = {}
    errors = []
    for path in marked_files(root):
        try:
            lines = path.read_text(encoding='utf-8').splitlines()
        except UnicodeDecodeError:
            continue  # Binary fixtures cannot hold markers.
        relative = path.relative_to(root).as_posix()
        for number, line in enumerate(lines, start=1):
            match = MARKER.search(line)
            if not match:
                continue
            place = f'{relative}:{number}'
            for name in marker_entries(match.group(1)):
                if FEATURE_ID.fullmatch(name):
                    locations.setdefault(name, []).append(place)
                else:
                    errors.append(
                        f'{place}: malformed parity marker entry {name!r}')
    return locations, errors


def marker_entries(text: str) -> list[str]:
    """Return the comma-separated entries after ``parity:`` in a marker.

    A closing ``*/`` of a block comment is not part of the last entry.
    """
    return [entry.strip() for entry in text.removesuffix('*/').split(',')]
