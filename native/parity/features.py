# SPDX-License-Identifier: AGPL-3.0-only
"""Load and validate features.toml, the inventory of behaviours the native app must provide.

README.md in this directory explains the fields and the regression process.
"""
from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import tomllib

from desktop_tests import Catalog

FEATURE_ID = re.compile(r'([A-Z]{2,5})-\d{3}')
KEYS = ('id', 'area', 'title', 'behaviour', 'origin', 'priority', 'openxplorer',
        'sources', 'python_tests', 'bridge', 'dolphin', 'gnome', 'native', 'native_note')
OPTIONAL_KEYS = frozenset({'dolphin', 'gnome', 'native_note'})
TEXT_KEYS = ('id', 'area', 'title', 'behaviour', 'dolphin', 'gnome', 'native_note')
LIST_KEYS = ('origin', 'sources', 'python_tests', 'bridge')
ORIGINS = ('openxplorer', 'dolphin', 'gnome')
CHOICES = {
    'priority': ('must', 'should', 'could'),
    'openxplorer': ('has', 'partial', 'missing'),
    'native': ('todo', 'partial', 'done', 'n-a'),
}
NATIVE_STATUSES = CHOICES['native']
FINISHED = frozenset({'done', 'n-a'})


def load(path: Path) -> list[dict]:
    """The [[feature]] tables of an inventory file."""
    data = tomllib.loads(path.read_text())
    if data.get('schema') != 1 or not isinstance(data.get('feature'), list):
        raise ValueError(f'{path.name}: expected "schema = 1" and [[feature]] tables.')
    return data['feature']


def validate(features: list[dict], bridge_operations: set[str], catalog: Catalog,
             markers: dict[str, list[str]]) -> list[str]:
    """Every problem in the inventory, its bridge coverage and its parity markers."""
    errors = []
    seen = set()
    for number, feature in enumerate(features, start=1):
        name = feature.get('id') if isinstance(feature.get('id'), str) else f'feature #{number}'
        if name in seen:
            errors.append(f'{name}: duplicate id')
        seen.add(name)
        errors.extend(f'{name}: {problem}'
                      for problem in feature_problems(feature, bridge_operations, catalog))
    cited = {operation for feature in features
             for operation in string_list(feature.get('bridge')) or []}
    errors.extend(f'Bridge operation {operation} is not cited by any feature'
                  for operation in sorted(bridge_operations - cited))
    errors.extend(marker_problems(features, markers))
    return errors


def feature_problems(feature: dict, bridge_operations: set[str], catalog: Catalog) -> list[str]:
    """Problems with one [[feature]] table; structure first, then its claims."""
    problems = structure_problems(feature)
    if problems:
        return problems  # Later checks assume the documented types.
    return (identity_problems(feature) + status_problems(feature)
            + citation_problems(feature, bridge_operations, catalog))


def structure_problems(feature: dict) -> list[str]:
    """Missing, unknown or misordered keys, wrong types and unknown choices."""
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
            problems.append(f'{key} must be a list of distinct non-empty strings')
    for key, choices in CHOICES.items():
        if key in feature and feature[key] not in choices:
            problems.append(f'{key} must be one of {", ".join(choices)}')
    return problems


def identity_problems(feature: dict) -> list[str]:
    """The id shape, its area prefix and the origin list."""
    problems = []
    match = FEATURE_ID.fullmatch(feature['id'])
    if not match:
        problems.append('id must look like AREA-001 (2-5 capital letters and 3 digits)')
    elif match.group(1) != feature['area']:
        problems.append(f'id prefix must equal its area {feature["area"]!r}')
    if not feature['origin'] or not set(feature['origin']) <= set(ORIGINS):
        problems.append(f'origin must list some of {", ".join(ORIGINS)}')
    if not feature['sources']:
        problems.append('sources must name at least one reference')
    return problems


def status_problems(feature: dict) -> list[str]:
    """Rules tying priority, origin and native status together."""
    problems = []
    existing = feature['openxplorer'] in ('has', 'partial')
    if feature['openxplorer'] == 'has' and 'openxplorer' not in feature['origin']:
        problems.append('a behaviour OpenXplorer has must list "openxplorer" in origin')
    if existing and 'openxplorer' in feature['origin'] and feature['priority'] != 'must':
        problems.append('a behaviour OpenXplorer has must have priority "must"')
    if feature['native'] == 'n-a' and 'native_note' not in feature:
        problems.append('native = "n-a" requires a native_note explaining why')
    return problems


def citation_problems(feature: dict, bridge_operations: set[str], catalog: Catalog) -> list[str]:
    """Bridge operations and desktop tests that do not exist."""
    problems = [f'unknown bridge operation {operation!r}'
                for operation in feature['bridge'] if operation not in bridge_operations]
    problems += [f'python test not found: {reference}'
                 for reference in feature['python_tests'] if not catalog.find(reference)]
    return problems


def marker_problems(features: list[dict], markers: dict[str, list[str]]) -> list[str]:
    """Markers must name real features, and only tested features may be recorded as done."""
    statuses = {feature.get('id'): feature.get('native') for feature in features}
    problems = []
    for name, places in sorted(markers.items()):
        if name not in statuses:
            problems.append(f'{places[0]}: parity marker names unknown feature {name}')
        elif statuses[name] in ('todo', 'n-a'):
            problems.append(f'{name}: has a parity marker ({places[0]}) but native = '
                            f'"{statuses[name]}"; record "partial" or "done"')
    problems += [f'{name}: native = "done" requires a parity marker on a native test'
                 for name, status in statuses.items() if status == 'done' and name not in markers]
    return problems


def replace_gate_blockers(features: list[dict]) -> list[str]:
    """Existing OpenXplorer behaviours the native app does not provide yet."""
    return [feature['id'] for feature in features
            if 'openxplorer' in feature['origin'] and feature['openxplorer'] in ('has', 'partial')
            and feature['native'] not in FINISHED]


def dolphin_gate_blockers(features: list[dict]) -> list[str]:
    """Dolphin baseline behaviours marked "must" that the native app does not provide yet."""
    return [feature['id'] for feature in features
            if 'dolphin' in feature['origin'] and feature['priority'] == 'must'
            and feature['native'] not in FINISHED]


GATES = {'replace': replace_gate_blockers, 'dolphin': dolphin_gate_blockers}


def summary(features: list[dict]) -> list[str]:
    """A table of native status counts per area, in file order, with totals."""
    areas = list(dict.fromkeys(feature['area'] for feature in features))
    counts = Counter((feature['area'], feature['native']) for feature in features)
    header = f'{"Area":<6}{"total":>7}' + ''.join(f'{status:>9}' for status in NATIVE_STATUSES)
    lines = [header]
    for area in areas + ['All']:
        row = [sum(count for (name, status), count in counts.items()
                   if status == wanted and area in (name, 'All')) for wanted in NATIVE_STATUSES]
        lines.append(f'{area:<6}{sum(row):>7}' + ''.join(f'{value:>9}' for value in row))
    return lines


def is_text(value: object) -> bool:
    """A string with visible content."""
    return isinstance(value, str) and bool(value.strip())


def string_list(value: object) -> list[str] | None:
    """The value if it is a list of distinct non-empty strings, otherwise None."""
    if not isinstance(value, list) or not all(is_text(item) for item in value):
        return None
    return value if len(set(value)) == len(value) else None
