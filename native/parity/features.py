# SPDX-License-Identifier: AGPL-3.0-only
"""Load and validate features.toml, the behaviours the native app needs.

README.md in this directory explains the fields and the regression
process.
"""
from __future__ import annotations

from collections import Counter
from collections.abc import Callable, Iterable
from pathlib import Path
import re
import tomllib
from typing import Any, TypeAlias

from legacy import Catalog

Feature: TypeAlias = dict[str, Any]

FEATURE_ID = re.compile(r'([A-Z]{2,5})-\d{3}')
KEYS = ('id', 'area', 'title', 'behaviour', 'origin', 'priority',
        'openxplorer', 'sources', 'python_tests', 'bridge', 'dolphin',
        'gnome', 'native', 'native_note')
OPTIONAL_KEYS = frozenset({'dolphin', 'gnome', 'native_note'})
TEXT_KEYS = ('id', 'area', 'title', 'behaviour', 'dolphin', 'gnome',
             'native_note')
LIST_KEYS = ('origin', 'sources', 'python_tests', 'bridge')
ORIGINS = ('openxplorer', 'dolphin', 'gnome')
CHOICES = {
    'priority': ('must', 'should', 'could'),
    'openxplorer': ('has', 'partial', 'missing'),
    'native': ('todo', 'partial', 'done', 'n-a'),
}
NATIVE_STATUSES = CHOICES['native']
EXISTING = ('has', 'partial')
FINISHED = frozenset({'done', 'n-a'})


def load(path: Path) -> list[Feature]:
    """Return the ``[[feature]]`` tables of an inventory file.

    Raise ValueError if the file is not valid TOML or lacks the
    schema version and feature tables; tomllib.TOMLDecodeError is
    itself a ValueError.
    """
    data = tomllib.loads(path.read_text(encoding='utf-8'))
    if data.get('schema') != 1 or not isinstance(data.get('feature'), list):
        raise ValueError(
            f'{path.name}: expected "schema = 1" and [[feature]] tables.')
    features: list[Feature] = data['feature']
    return features


def validate(features: list[Feature], bridge_operations: set[str],
             catalog: Catalog, markers: dict[str, list[str]]) -> list[str]:
    """Return every problem in the inventory, or an empty list.

    Besides each feature on its own, this checks that ids are unique,
    that every bridge operation is cited by some feature, and that
    parity markers agree with the native statuses.
    """
    errors = []
    seen = set()
    for number, feature in enumerate(features, start=1):
        name = feature_name(feature, number)
        if name in seen:
            errors.append(f'{name}: duplicate id')
        seen.add(name)
        problems = feature_problems(feature, bridge_operations, catalog)
        errors += [f'{name}: {problem}' for problem in problems]
    uncited = bridge_operations - cited_operations(features)
    errors += [f'Bridge operation {operation} is not cited by any feature'
               for operation in sorted(uncited)]
    errors += marker_problems(features, markers)
    return errors


def feature_name(feature: Feature, number: int) -> str:
    """Return how errors name a feature: its id, or its position."""
    identifier = feature.get('id')
    if isinstance(identifier, str):
        return identifier
    return f'feature #{number}'


def cited_operations(features: list[Feature]) -> set[str]:
    """Return the bridge operations that well-formed features cite."""
    cited: set[str] = set()
    for feature in features:
        cited.update(string_list(feature.get('bridge')) or [])
    return cited


def feature_problems(feature: Feature, bridge_operations: set[str],
                     catalog: Catalog) -> list[str]:
    """Return the problems with one ``[[feature]]`` table.

    Structure is checked first, and the claims only when it is sound,
    because the later checks assume the documented types.
    """
    problems = structure_problems(feature)
    if problems:
        return problems
    return (identity_problems(feature)
            + status_problems(feature)
            + citation_problems(feature, bridge_operations, catalog))


def structure_problems(feature: Feature) -> list[str]:
    """Return missing, unknown or misordered keys and bad values."""
    problems = [f'unknown key {key!r}' for key in feature if key not in KEYS]
    problems += [f'missing key {key!r}' for key in KEYS
                 if key not in feature and key not in OPTIONAL_KEYS]
    present = [key for key in feature if key in KEYS]
    if present != [key for key in KEYS if key in feature]:
        problems.append('keys are not in the documented order')
    for key in TEXT_KEYS:
        if key in feature and not is_text(feature[key]):
            problems.append(f'{key} must be a non-empty string')
    for key in LIST_KEYS:
        if key in feature and string_list(feature[key]) is None:
            problems.append(
                f'{key} must be a list of distinct non-empty strings')
    for key, choices in CHOICES.items():
        if key in feature and feature[key] not in choices:
            problems.append(f'{key} must be one of {", ".join(choices)}')
    return problems


def identity_problems(feature: Feature) -> list[str]:
    """Return problems with the id, area prefix, origin and sources."""
    problems = []
    match = FEATURE_ID.fullmatch(feature['id'])
    if not match:
        problems.append('id must look like AREA-001 '
                        '(2-5 capital letters and 3 digits)')
    elif match.group(1) != feature['area']:
        problems.append(
            f'id prefix must equal its area {feature["area"]!r}')
    if not feature['origin'] or not set(feature['origin']) <= set(ORIGINS):
        problems.append(f'origin must list some of {", ".join(ORIGINS)}')
    if not feature['sources']:
        problems.append('sources must name at least one reference')
    return problems


def status_problems(feature: Feature) -> list[str]:
    """Return breaches of the rules tying priority, origin and status.

    Rule 1 of the README makes every behaviour the current app has a
    "must", and "n-a" is a product decision that needs a reason.
    """
    problems = []
    if feature['openxplorer'] == 'has' and not from_openxplorer(feature):
        problems.append(
            'a behaviour OpenXplorer has must list "openxplorer" in origin')
    if is_existing_behaviour(feature) and feature['priority'] != 'must':
        problems.append(
            'a behaviour OpenXplorer has must have priority "must"')
    if feature['native'] == 'n-a' and 'native_note' not in feature:
        problems.append(
            'native = "n-a" requires a native_note explaining why')
    return problems


def citation_problems(feature: Feature, bridge_operations: set[str],
                      catalog: Catalog) -> list[str]:
    """Return cited bridge operations and tests that do not exist."""
    problems = [f'unknown bridge operation {operation!r}'
                for operation in feature['bridge']
                if operation not in bridge_operations]
    problems += [f'python test not found: {reference}'
                 for reference in feature['python_tests']
                 if not catalog.find(reference)]
    return problems


def marker_problems(features: list[Feature],
                    markers: dict[str, list[str]]) -> list[str]:
    """Return disagreements between parity markers and native statuses.

    A marker must name a real feature, a marked feature cannot still be
    "todo" or "n-a", and "done" needs a marker, so every finished
    feature points at the native test that proves it.
    """
    # An id that is not a string is reported by structure_problems.
    statuses = {feature['id']: feature.get('native')
                for feature in features
                if isinstance(feature.get('id'), str)}
    problems = []
    for name, places in sorted(markers.items()):
        if name not in statuses:
            problems.append(
                f'{places[0]}: parity marker names unknown feature {name}')
        elif statuses[name] in ('todo', 'n-a'):
            problems.append(
                f'{name}: has a parity marker ({places[0]}) but native = '
                f'"{statuses[name]}"; record "partial" or "done"')
    problems += [
        f'{name}: native = "done" requires a parity marker on a native test'
        for name, status in statuses.items()
        if status == 'done' and name not in markers
    ]
    return problems


def replace_gate_blockers(features: list[Feature]) -> list[str]:
    """Return existing OpenXplorer behaviours that are not finished."""
    return [feature['id'] for feature in features
            if is_existing_behaviour(feature) and not is_finished(feature)]


def dolphin_gate_blockers(features: list[Feature]) -> list[str]:
    """Return the Dolphin "must" behaviours that are not finished."""
    return [feature['id'] for feature in features
            if 'dolphin' in feature['origin']
            and feature['priority'] == 'must'
            and not is_finished(feature)]


GATES: dict[str, Callable[[list[Feature]], list[str]]] = {
    'replace': replace_gate_blockers,
    'dolphin': dolphin_gate_blockers,
}


def summary(features: list[Feature]) -> list[str]:
    """Return a table of native status counts per area, with totals.

    Areas appear in file order, followed by an "All" row.
    """
    areas = list(dict.fromkeys(feature['area'] for feature in features))
    counts = Counter((feature['area'], feature['native'])
                     for feature in features)
    totals = Counter(feature['native'] for feature in features)
    lines = [f'{"Area":<6}{"total":>7}' + status_columns(NATIVE_STATUSES)]
    for area in areas:
        row = [counts[area, status] for status in NATIVE_STATUSES]
        lines.append(summary_row(area, row))
    lines.append(summary_row('All', [totals[status]
                                     for status in NATIVE_STATUSES]))
    return lines


def summary_row(label: str, row: list[int]) -> str:
    """Format one row of the summary table: label, total and counts."""
    return f'{label:<6}{sum(row):>7}' + status_columns(row)


def status_columns(values: Iterable[str | int]) -> str:
    """Right-align the per-status columns of the summary table."""
    return ''.join(f'{value:>9}' for value in values)


def from_openxplorer(feature: Feature) -> bool:
    """Return whether the current app is one of a feature's origins."""
    return 'openxplorer' in feature['origin']


def is_existing_behaviour(feature: Feature) -> bool:
    """Return whether the current app has a behaviour, even partly."""
    return from_openxplorer(feature) and feature['openxplorer'] in EXISTING


def is_finished(feature: Feature) -> bool:
    """Return whether the native app is done or the feature is n-a."""
    return feature['native'] in FINISHED


def is_text(value: object) -> bool:
    """Return whether a value is a string with visible content."""
    return isinstance(value, str) and bool(value.strip())


def string_list(value: object) -> list[str] | None:
    """Return the value if it lists distinct non-empty strings."""
    if not isinstance(value, list) or not all(is_text(item) for item in value):
        return None
    return value if len(set(value)) == len(value) else None
