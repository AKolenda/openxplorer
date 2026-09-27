# SPDX-License-Identifier: AGPL-3.0-only
"""Print what desktop/core.py answers for the location parity inputs.

Usage, from the repository root:

    python3 native/crates/ox-core/tests/location_fixtures/generate_python.py desktop

The JSON document on standard output is python.json. location_python.rs and
location_external.rs compare the Rust port with it, and
location_fixture_drift.rs runs this script again to prove that python.json
still matches core.py.

Every answer is recorded as one of three outcomes:

- {"value": ...}: core.py returned this value.
- {"error": message}: a `raise` statement in core.py refused the input with
  its own message. The Rust port must show exactly this message.
- {"rejected": message}: Python's standard library refused the input with its
  own wording. The Rust port must refuse the input too, in its own words.
"""
import json
import os
from pathlib import Path
import sys
import traceback

# Importing core must not leave a __pycache__ folder in desktop/.
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).parent))

from inputs import (  # noqa: E402
    BASES, COPY_NAMES, EXTERNAL_HOME, EXTERNAL_LOCATIONS, HOME, ITEMS, LABELS, LOCATIONS, NAMES, SERVERS,
    SHARES, SPLITS,
)

# require_item_uri, require_share and is_smb_server resolve relative input
# against Path.home(), which reads HOME.
os.environ['HOME'] = HOME
sys.path.insert(0, sys.argv[1])
import core  # noqa: E402

CORE_SOURCE = Path(core.__file__).resolve()


def raised_by_core(error: ValueError) -> bool:
    """True when a `raise` statement in core.py wrote the message itself."""
    frame = traceback.extract_tb(error.__traceback__)[-1]
    in_core = Path(frame.filename).resolve() == CORE_SOURCE
    return in_core and (frame.line or '').startswith('raise ')


def outcome(function, *args) -> dict:
    """The value, core error or standard-library rejection of one call."""
    try:
        return {'value': function(*args)}
    except ValueError as error:  # UnicodeDecodeError is a ValueError too.
        if raised_by_core(error):
            return {'error': str(error)}
        return {'rejected': str(error)}


def split_parts(value: str) -> dict:
    """core.split_location as named fields."""
    parts = core.split_location(value)
    return {
        'scheme': parts.scheme,
        'netloc': parts.netloc,
        'path': parts.path,
        'query': parts.query,
        'fragment': parts.fragment,
    }


def is_server(value: str) -> bool:
    """core.is_smb_server, where an address core.py refuses is not a server.

    Its callers in desktop/ only ask about locations that were already
    normalised; the Rust port answers false instead of failing.
    """
    try:
        return core.is_smb_server(value)
    except ValueError:
        return False


def input_cases(function, values) -> list:
    """One {input, outcome} case per value."""
    return [{'input': value, 'outcome': outcome(function, value)} for value in values]


def external_cases() -> dict:
    """The location_external.rs tables, captured with EXTERNAL_HOME."""
    home = Path(EXTERNAL_HOME)

    def normalise(value):
        return core.normalise_location(value, None, home)

    def normalised_item(value):
        return core.require_item_uri(normalise(value))

    return {
        'home': EXTERNAL_HOME,
        'normalise': input_cases(normalise, EXTERNAL_LOCATIONS),
        'items': input_cases(normalised_item, EXTERNAL_LOCATIONS),
    }


def capture() -> dict:
    """Every table in python.json."""
    home = Path(HOME)
    return {
        'home': HOME,
        'normalise': input_cases(lambda value: core.normalise_location(value, None, home), LOCATIONS),
        'relative': [
            {'input': value, 'base': base, 'outcome': outcome(core.normalise_location, value, base, home)}
            for value, base in BASES
        ],
        'names': input_cases(core.validate_name, NAMES),
        'copies': [
            {'name': name, 'number': number, 'is_dir': is_dir,
             'outcome': outcome(core.new_copy_name, name, number, is_dir)}
            for name, number, is_dir in COPY_NAMES
        ],
        'labels': [
            {'input': value, 'fallback': fallback, 'outcome': outcome(core.safe_label, value, fallback)}
            for value, fallback in LABELS
        ],
        'items': input_cases(core.require_item_uri, ITEMS),
        'shares': input_cases(core.require_share, SHARES),
        'servers': [{'input': value, 'is_server': is_server(value)} for value in SERVERS],
        'devices': [{'input': value, 'is_device': core.is_device_location(value)} for value in SPLITS + LOCATIONS],
        'splits': input_cases(split_parts, SPLITS),
        'external': external_cases(),
    }


def main() -> None:
    document = json.dumps(capture(), ensure_ascii=False, indent=2) + '\n'
    sys.stdout.buffer.write(document.encode('utf-8'))


if __name__ == '__main__':
    main()
