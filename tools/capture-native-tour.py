#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Capture the website's click-through tour of the native app.

The website cannot run the GTK app, so the tour is a series of pictures of the
real app, taken by tools/native_capture.py in an isolated session with the
fictional demo tree, in the light and the dark theme. Each scene's clickable
areas are the rectangles of real controls, which the app reports for the
picture (OPENXPLORER_HOTSPOTS); a hotspot names the scene it leads to.

Writes apps/web/public/tour/<scene>-<theme>.png and
apps/web/public/tour/scenes.json, which apps/web/public/tour/index.html shows.
Run it after a change to the app's look:

    python3 tools/capture-native-tour.py        (or: pnpm capture:tour)

then review every picture before committing it (docs/PRIVACY.md) and run
python3 tools/audit-public-data.py.
"""
from __future__ import annotations

import argparse
import hashlib
from dataclasses import dataclass, replace
import json
from pathlib import Path
import sys
import tomllib
from typing import Any

import native_capture
from native_capture import DEMO_HOME, Picture

OUTPUT = native_capture.ROOT / 'apps' / 'web' / 'public' / 'tour'
THEMES = ('light', 'dark')
DOCUMENTS = f'{DEMO_HOME}/Documents'
FOLDER = 'Launch planning'
FOLDER_PATH = f'{DOCUMENTS}/{FOLDER}'
FILE = 'Budget.ods'
SNAPSHOT_PATH = f'{FOLDER_PATH}/.snapshot/{native_capture.DEMO_SNAPSHOTS[-1]}'


@dataclass(frozen=True)
class Target:
    """A control in a picture that leads to another scene.

    The control is the hotspot whose text (and tooltip and kind, when given)
    equal these; label is what the tour says the click does.
    """

    text: str
    goes_to: str
    label: str
    tooltip: str | None = None
    kind: str | None = None

    def matches(self, hotspot: dict[str, Any]) -> bool:
        """Return whether a reported hotspot is this control."""
        return (hotspot['text'] == self.text
                and (self.tooltip is None or hotspot['tooltip'] == self.tooltip)
                and (self.kind is None or hotspot['kind'] == self.kind))


@dataclass(frozen=True)
class Scene:
    """One picture of the tour and where its controls lead."""

    name: str
    title: str
    caption: str
    picture: Picture
    targets: tuple[Target, ...]


def sidebar(label: str, folder: str, goes_to: str) -> Target:
    """Return the sidebar row of a folder in the demo home."""
    path = f'{DEMO_HOME}/{folder}' if folder else DEMO_HOME
    return Target(label, goes_to, f'Open {label}', tooltip=path, kind='row')


def listed(name: str, goes_to: str, label: str) -> Target:
    """Return the row of an item in the file list (it has no tooltip)."""
    return Target(name, goes_to, label, tooltip='', kind='row')


HOME = sidebar('Home', '', 'home')
THIS_PC = Target('This PC', 'this-pc', 'Open This PC', tooltip='This PC', kind='row')
NETWORK = Target('Network', 'network', 'Open Network', tooltip='Network', kind='row')

SCENES = (
    Scene('home', 'Home', 'Your home folder, with the sidebar on the left and the details pane '
          'on the right.',
          Picture(start=DEMO_HOME),
          (listed('Documents', 'documents', 'Open Documents'),
           sidebar('Pictures', 'Pictures', 'pictures'), THIS_PC, NETWORK,
           Target('Search Home', 'search', 'Search for “budget”', kind='entry'),
           Target('New tab (Ctrl+T)', 'tabs', 'Open more tabs', kind='button'),
           Target('Settings (Ctrl+,)', 'settings', 'Open Settings', kind='button'))),
    Scene('documents', 'Documents', 'Documents in the Details view, with folders first. The '
          'details pane describes the folder shown.',
          Picture(start=DOCUMENTS),
          (listed(FOLDER, 'folder', f'Open {FOLDER}'),
           Target('Home', 'home', 'Back to Home', tooltip=DEMO_HOME, kind='button'),
           sidebar('Pictures', 'Pictures', 'pictures'), THIS_PC)),
    Scene('folder', 'A selected file', f'{FOLDER}, with {FILE} selected. The details pane shows '
          'its type, size and dates.',
          Picture(start=FOLDER_PATH, scene=(f'select={FILE}',)),
          (listed(FILE, 'context-menu', f'Right-click {FILE}'),
           Target('Documents', 'documents', 'Back to Documents', tooltip=DOCUMENTS, kind='button'),
           Target('Home', 'home', 'Back to Home', tooltip=DEMO_HOME, kind='button'))),
    Scene('context-menu', 'Context menu', 'The right-click menu of a file, in the classic '
          'Windows 10 style. Settings also offer the compact Windows 11 menu.',
          Picture(start=FOLDER_PATH, scene=(f'select={FILE}', 'action=context-menu')),
          (Target('Previous versions', 'versions', 'Show previous versions', kind='row'),
           Target('Properties', 'properties', 'Open Properties', kind='row'))),
    Scene('properties', 'Properties', 'Properties of a file: its type, size, location, dates '
          'and attributes, on tabs.',
          Picture(start=FOLDER_PATH, scene=(f'select={FILE}', 'action=properties')),
          (Target('Previous versions', 'versions', 'Show previous versions', kind='button'),
           Target('Close', 'folder', 'Close Properties', kind='button'))),
    Scene('versions', 'Previous versions', 'Snapshots the storage already exposes, newest '
          'first, with the date read from each snapshot’s name. Restore a copy never '
          'overwrites the file.',
          Picture(start=FOLDER_PATH, scene=(f'select={FILE}', 'action=previous-versions')),
          (Target('General', 'properties', 'Show the General tab', kind='button'),
           Target('Close', 'folder', 'Close Properties', kind='button'))),
    Scene('pictures', 'Large icons', 'Pictures in the Large icons view. The buttons at the bottom '
          'right switch views.',
          Picture(start=f'{DEMO_HOME}/Pictures', view='large'),
          (sidebar('Documents', 'Documents', 'documents'), HOME, THIS_PC)),
    Scene('this-pc', 'This PC', 'Quick access folders, drives and saved network locations.',
          Picture(start='pc:'),
          (NETWORK, HOME, sidebar('Documents', 'Documents', 'documents'))),
    Scene('network', 'Network', 'Find servers, type a \\\\server\\share address, or open a '
          'saved share. The sample share is shown offline: this tour has no network.',
          Picture(start='network:', with_share=True),
          (THIS_PC, HOME)),
    Scene('search', 'Search', 'Search finds names in this folder and its subfolders, here from '
          'the search cache of the pinned folder, and shows each result’s folder.',
          Picture(start=DEMO_HOME, search='budget', scene=('wait=3000',)),
          (Target('Home', 'home', 'Clear the search', tooltip=DEMO_HOME, kind='button'),
           sidebar('Documents', 'Documents', 'documents'))),
    Scene('tabs', 'Tabs', 'Tabs keep several places open in one window. The amber tab shows a '
          'previous version of the folder.',
          Picture(start=FOLDER_PATH, scene=(f'action=open-tab-background:file://{SNAPSHOT_PATH}',
                                            'action=open-tab-background:pc:')),
          (Target(native_capture.DEMO_SNAPSHOTS[-1], 'snapshot', 'Show the previous version',
                  kind='box'),
           Target('This PC', 'this-pc', 'Show the This PC tab', kind='box'),
           listed(FILE, 'context-menu', f'Right-click {FILE}'))),
    Scene('snapshot', 'A previous version', 'A snapshot folder opens in a tab marked Previous '
          'version, with a read-only banner.',
          Picture(start=SNAPSHOT_PATH),
          (Target('Documents', 'documents', 'Back to Documents', tooltip=DOCUMENTS, kind='button'),
           HOME)),
    Scene('settings', 'Settings', 'Settings open in a tab: appearance, search and indexing, '
          'default apps, windows and tabs.',
          Picture(settings='appearance'),
          (Target('Back to files', 'home', 'Back to files', kind='button'),)),
)


def resolve_targets(scene: Scene, hotspots: dict[str, Any]) -> list[dict[str, Any]]:
    """Return the scene's hotspots: each target's rectangle in the picture.

    A target the app did not report fails the capture, so a renamed control
    cannot leave a dead hotspot behind.
    """
    resolved = []
    for target in scene.targets:
        found = [hotspot for hotspot in hotspots['hotspots'] if target.matches(hotspot)]
        if not found:
            raise native_capture.CaptureError(
                f'{scene.name}: the app reported no control “{target.text}”.')
        area = found[0]
        resolved.append({'label': target.label, 'goesTo': target.goes_to,
                         **{key: area[key] for key in ('x', 'y', 'width', 'height')}})
    return resolved


def check_scenes() -> None:
    """Fail when a target leads to a scene that does not exist."""
    names = {scene.name for scene in SCENES}
    for scene in SCENES:
        for target in scene.targets:
            if target.goes_to not in names:
                raise native_capture.CaptureError(f'{scene.name}: no scene {target.goes_to!r}.')


def app_version() -> str:
    """Return the native app's version from native/Cargo.toml."""
    manifest = tomllib.loads((native_capture.NATIVE / 'Cargo.toml').read_text(encoding='utf-8'))
    version: str = manifest['workspace']['package']['version']
    return version


def capture_tour(program: Path, output: Path, only: set[str]) -> None:
    """Capture every scene in both themes and write scenes.json."""
    check_scenes()
    scenes_file = output / 'scenes.json'
    previous = {}
    if scenes_file.is_file():
        previous = {scene['id']: scene for scene in json.loads(scenes_file.read_text())['scenes']}
    written = []
    with native_capture.demo_session() as workspace:
        for scene in SCENES:
            if only and scene.name not in only and scene.name in previous:
                written.append(previous[scene.name])
                continue
            images = {}
            hotspots: list[dict[str, Any]] = []
            for theme in THEMES:
                picture = replace(scene.picture, theme=theme)
                image = f'{scene.name}-{theme}.png'
                print(f'Capturing {image}', flush=True)
                reported = native_capture.capture(program, workspace, picture, output / image,
                                                  with_hotspots=True)
                assert reported is not None
                if theme == THEMES[0]:
                    hotspots = resolve_targets(scene, reported)
                images[theme] = image
            written.append({'id': scene.name, 'title': scene.title, 'caption': scene.caption,
                            'images': images, 'hotspots': hotspots})
    pictures = sorted(image for scene in written for image in scene['images'].values())
    tour = {
        'generator': 'tools/capture-native-tour.py',
        'app': f'OpenXplorer {app_version()}, the native GTK 4 app',
        'width': 1440, 'height': 900,
        'start': SCENES[0].name,
        'scenes': written,
        # tools/audit-public-data.py accepts only pictures registered here.
        'sha256': {image: hashlib.sha256((output / image).read_bytes()).hexdigest()
                   for image in pictures},
    }
    scenes_file.write_text(json.dumps(tour, indent=1, ensure_ascii=False) + '\n', encoding='utf-8')
    print(f'Wrote {len(written)} scenes to {scenes_file.relative_to(native_capture.ROOT)}.')


def main() -> int:
    """Build the app, capture the tour and report a failure plainly."""
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--program', type=Path,
                        help='a built openxplorer-native to use instead of building one')
    parser.add_argument('--scene', action='append', default=[],
                        help='recapture only this scene (repeatable); the others are kept')
    args = parser.parse_args()
    try:
        program = args.program or native_capture.build_app()
        capture_tour(program, OUTPUT, set(args.scene))
    except native_capture.CaptureError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
