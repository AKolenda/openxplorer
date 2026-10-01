# SPDX-License-Identifier: AGPL-3.0-only
"""Print what v2.0.0:desktop/core.py answers for the location parity inputs.

Usage, from the repository root:

    python3 native/crates/ox-core/tests/location_fixtures/generate_python.py "$(python3 native/tools/python_app.py)"

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

# Importing core must not leave a __pycache__ folder in the Python app's sources.
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).parent))

from inputs import (  # noqa: E402
    BASES, COPY_NAMES, EXTERNAL_HOME, EXTERNAL_LOCATIONS, HOME, ITEMS, LABELS, LOCATIONS, NAMES,
    SERVERS, SHARES, SPLITS,
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


def split_parts(address: str) -> dict:
    """core.split_location as named fields."""
    parts = core.split_location(address)
    return {
        'scheme': parts.scheme,
        'netloc': parts.netloc,
        'path': parts.path,
        'query': parts.query,
        'fragment': parts.fragment,
    }


def is_server(address: str) -> bool:
    """core.is_smb_server, where an address core.py refuses is not a server.

    Its callers in v2.0.0:desktop/ only ask about locations that were already
    normalised; the Rust port answers false instead of failing.
    """
    try:
        return core.is_smb_server(address)
    except ValueError:
        return False


def input_cases(function, inputs) -> list:
    """One {input, outcome} case per text in inputs."""
    cases = []
    for text in inputs:
        result = outcome(function, text)
        cases.append({'input': text, 'outcome': result})
    return cases


def relative_cases(home: Path) -> list:
    """normalise_location(input, base, home) for each (input, base) pair."""
    cases = []
    for address, base in BASES:
        result = outcome(core.normalise_location, address, base, home)
        cases.append({'input': address, 'base': base, 'outcome': result})
    return cases


def copy_name_cases() -> list:
    """new_copy_name(name, number, is_dir) for each "Keep both" input."""
    cases = []
    for name, number, is_dir in COPY_NAMES:
        result = outcome(core.new_copy_name, name, number, is_dir)
        cases.append({'name': name, 'number': number, 'is_dir': is_dir, 'outcome': result})
    return cases


def label_cases() -> list:
    """safe_label(input, fallback) for each sidebar label."""
    cases = []
    for label, fallback in LABELS:
        result = outcome(core.safe_label, label, fallback)
        cases.append({'input': label, 'fallback': fallback, 'outcome': result})
    return cases


def server_cases() -> list:
    """is_server(input) for each address in SERVERS."""
    cases = []
    for address in SERVERS:
        cases.append({'input': address, 'is_server': is_server(address)})
    return cases


def device_cases() -> list:
    """is_device_location(input) for every split and normalise input."""
    cases = []
    for address in SPLITS + LOCATIONS:
        cases.append({'input': address, 'is_device': core.is_device_location(address)})
    return cases


def external_cases() -> dict:
    """The location_external.rs tables, captured with EXTERNAL_HOME."""
    home = Path(EXTERNAL_HOME)

    def normalise(address: str) -> str:
        """core.normalise_location of address, resolved against EXTERNAL_HOME."""
        return core.normalise_location(address, None, home)

    def normalised_item(address: str) -> str:
        """core.require_item_uri of the normalised address."""
        return core.require_item_uri(normalise(address))

    return {
        'home': EXTERNAL_HOME,
        'normalise': input_cases(normalise, EXTERNAL_LOCATIONS),
        'items': input_cases(normalised_item, EXTERNAL_LOCATIONS),
    }


def capture() -> dict:
    """Every table in python.json, in the order the file lists them."""
    home = Path(HOME)

    def normalise(address: str) -> str:
        """core.normalise_location of address, resolved against HOME."""
        return core.normalise_location(address, None, home)

    return {
        'home': HOME,
        'normalise': input_cases(normalise, LOCATIONS),
        'relative': relative_cases(home),
        'names': input_cases(core.validate_name, NAMES),
        'copies': copy_name_cases(),
        'labels': label_cases(),
        'items': input_cases(core.require_item_uri, ITEMS),
        'shares': input_cases(core.require_share, SHARES),
        'servers': server_cases(),
        'devices': device_cases(),
        'splits': input_cases(split_parts, SPLITS),
        'external': external_cases(),
    }


def main() -> None:
    """Print python.json as UTF-8, whatever the terminal's encoding."""
    document = json.dumps(capture(), ensure_ascii=False, indent=2) + '\n'
    sys.stdout.buffer.write(document.encode('utf-8'))


if __name__ == '__main__':
    main()
