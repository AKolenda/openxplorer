# SPDX-License-Identifier: AGPL-3.0-only
"""Runs desktop/core.py over the parity inputs and prints JSON results."""
import json
import os
os.environ["HOME"] = "/home/test"
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
sys.path.insert(0, str(Path(__file__).parent))
import core  # noqa: E402
from inputs import (LOCATIONS, BASES, NAMES, COPY_NAMES, LABELS, ITEMS, SHARES, SERVERS, SPLITS)  # noqa: E402

HOME = Path('/home/test')


def run(function, *args):
    try:
        value = function(*args)
        return {'ok': value}
    except UnicodeDecodeError:
        return {'err': None}
    except ValueError as exc:
        return {'err': str(exc)}


def normalise(value, base=None):
    return core.normalise_location(value, base, HOME)


def server(value):
    try:
        return core.is_smb_server(value)
    except ValueError:
        return False


def split(value):
    parts = core.split_location(value)
    return [parts.scheme, parts.netloc, parts.path, parts.query, parts.fragment]


result = {
    'normalise': [[v, None, run(normalise, v)] for v in LOCATIONS],
    'relative': [[v, b, run(normalise, v, b)] for v, b in BASES],
    'names': [[n, run(core.validate_name, n)] for n in NAMES],
    'copies': [[n, c, d, run(core.new_copy_name, n, c, d)] for n, c, d in COPY_NAMES],
    'labels': [[v, f, run(core.safe_label, v, f)] for v, f in LABELS],
    'items': [[v, run(core.require_item_uri, v)] for v in ITEMS],
    'shares': [[v, run(core.require_share, v)] for v in SHARES],
    'servers': [[v, server(v)] for v in SERVERS],
    'devices': [[v, core.is_device_location(v)] for v in SPLITS + LOCATIONS],
    'splits': [[v, run(split, v)] for v in SPLITS],
}
with open(sys.argv[2], "w", encoding="utf-8") as out:
    json.dump(result, out, ensure_ascii=False, indent=0)
