#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Capture the website's screenshots from the native app.

Each screenshot is a picture of the real native app, taken by
tools/native_capture.py in an isolated session with the fictional demo tree
(docs/PRIVACY.md): no personal file, keyring, network or live desktop is
involved. Writes apps/web/public/assets/screenshots/*.png and manifest.json,
which tools/audit-public-data.py checks. Review every picture before
committing it.

    python3 tools/capture-screenshots.py [--program target/release/openxplorer-native]
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

import native_capture
from native_capture import DEMO_HOME, Picture

OUTPUT = native_capture.ROOT / 'apps' / 'web' / 'public' / 'assets' / 'screenshots'
DOCUMENTS = f'{DEMO_HOME}/Documents'
LAUNCH_PLANNING = f'{DOCUMENTS}/Launch planning'
SNAPSHOT = f'{LAUNCH_PLANNING}/.snapshot/{native_capture.DEMO_SNAPSHOTS[-1]}'

# Screenshot name -> what it shows. The names are the ones the website and
# the documentation (apps/web/lib/docs.json) use.
SCREENSHOTS = {
    'explorer-light': Picture(start=DOCUMENTS, scene=('select=Launch planning',)),
    'explorer-dark': Picture(start=DOCUMENTS, scene=('select=Launch planning',), theme='dark'),
    'network-path': Picture(start='network:', with_share=True),
    'pinned-sidebar': Picture(start=LAUNCH_PLANNING, with_share=True),
    # Launch planning is pinned, so the search cache holds it; the pause lets
    # the cache finish before the picture.
    'cached-search': Picture(start=DEMO_HOME, search='budget', scene=('wait=3000',)),
    'previous-versions': Picture(start=LAUNCH_PLANNING,
                                 scene=('select=Budget.ods', 'action=previous-versions')),
    'snapshot-tab': Picture(start=SNAPSHOT),
    # Split panes (F3): Documents in both panes, with the left one's selection.
    'split-view': Picture(start=DOCUMENTS, scene=('select=Launch planning', 'action=split-view')),
}
FIXTURE_POLICY = ('Entirely fictional sample names, addresses and paths (docs/PRIVACY.md); '
                  'a fresh isolated session with no network. No user screenshots.')


def main() -> int:
    """Build the app, capture every screenshot and write the manifest."""
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--program', type=Path,
                        help='a built openxplorer-native to use instead of building one')
    args = parser.parse_args()
    try:
        program = args.program or native_capture.build_app()
        with native_capture.demo_session() as workspace:
            for name, picture in SCREENSHOTS.items():
                print(f'Capturing {name}.png', flush=True)
                native_capture.capture(program, workspace, picture, OUTPUT / f'{name}.png')
    except native_capture.CaptureError as error:
        print(error, file=sys.stderr)
        return 1
    pictures = sorted(OUTPUT.glob('*.png'))
    manifest = {
        'fixturePolicy': FIXTURE_POLICY,
        'source': 'tools/capture-screenshots.py: the native app\'s snapshot hook '
                  '(OPENXPLORER_SNAPSHOT) with tools/native_capture.py\'s demo tree',
        'renderer': 'GTK 4',
        'nativeRuntime': True,
        'screenshots': [picture.name for picture in pictures],
        'sha256': {picture.name: hashlib.sha256(picture.read_bytes()).hexdigest()
                   for picture in pictures},
    }
    (OUTPUT / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
