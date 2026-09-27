# SPDX-License-Identifier: AGPL-3.0-only
"""Find parity markers: comments that tie a native test to the features it proves.

A marker is a comment such as ``// parity: NAV-001, TAB-004`` placed next to a
test in a Rust source under native/, or in any file under native/ui-tests/.
Only a feature named by a marker may be recorded as natively done.
"""
from __future__ import annotations

import os
from pathlib import Path
import re

FEATURE_ID = re.compile(r'[A-Z]{2,5}-\d{3}')
MARKER = re.compile(r'(?://|#|/\*|\*)\s*parity:(.*)')
SKIPPED_DIRECTORIES = {'target', '__pycache__'}


def marked_files(root: Path) -> list[Path]:
    """Rust sources under native/ and every file under native/ui-tests/."""
    native = root / 'native'
    ui_tests = native / 'ui-tests'
    files = []
    for directory, subdirectories, names in os.walk(native):
        subdirectories[:] = sorted(name for name in subdirectories
                                   if name not in SKIPPED_DIRECTORIES and not name.startswith('.'))
        for name in sorted(names):
            path = Path(directory, name)
            if path.suffix == '.rs' or path.is_relative_to(ui_tests):
                files.append(path)
    return files


def scan(root: Path) -> tuple[dict[str, list[str]], list[str]]:
    """Map each marked feature id to its 'path:line' locations; report bad markers."""
    locations: dict[str, list[str]] = {}
    errors = []
    for path in marked_files(root):
        try:
            lines = path.read_text().splitlines()
        except UnicodeDecodeError:
            continue  # Binary fixtures cannot hold markers.
        for number, line in enumerate(lines, start=1):
            match = MARKER.search(line)
            if not match:
                continue
            place = f'{path.relative_to(root).as_posix()}:{number}'
            names = [name.strip() for name in match.group(1).removesuffix('*/').split(',')]
            for name in names:
                if FEATURE_ID.fullmatch(name):
                    locations.setdefault(name, []).append(place)
                else:
                    errors.append(f'{place}: malformed parity marker entry {name!r}')
    return locations, errors
