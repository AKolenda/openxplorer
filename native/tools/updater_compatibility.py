# SPDX-License-Identifier: AGPL-3.0-only
"""Prove that the in-app updater of OpenXplorer 1.1.x installs a native package.

Runs Updater.check and Updater.install from desktop/updater.py, the code
every 1.1.x user runs, against a built package. A simulated GitHub answer
publishes the package's file under its own name; the updater must find it
under the asset name it expects, download it with the size and SHA-256
checks, read its fields with the real dpkg-deb and hand it to APT. Only
the network, the administrator prompt and the package database are
simulated, so nothing is downloaded or installed.
"""
from __future__ import annotations

import hashlib
import importlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import ModuleType
from typing import Any

from package_data import REPOSITORY

DESKTOP = REPOSITORY / 'desktop'
# The folder the Python package runs from, which the updater requires.
PYTHON_PACKAGE_ROOT = '/opt/openxplorer'
PROMPT = '/usr/bin/pkexec'
PACKAGE_QUERY = '/usr/bin/dpkg-query'


class UpdaterRefused(Exception):
    """The 1.1.x updater would not offer or install the package; the message says why."""


def python_updater() -> ModuleType:
    """Import desktop/updater.py as the Python app does, with desktop/ on the path."""
    if str(DESKTOP) not in sys.path:
        sys.path.insert(0, str(DESKTOP))
    return importlib.import_module('updater')


def release_answer(updater: ModuleType, package: Path, version: str) -> dict[str, Any]:
    """Return GitHub's latest-release answer for a release that publishes package."""
    data = package.read_bytes()
    tag = f'v{version}'
    return {
        'tag_name': tag, 'draft': False, 'prerelease': False,
        'body': 'Fictional release notes.',
        'assets': [{
            'name': package.name,
            'browser_download_url': f'{updater.REPOSITORY}/releases/download/{tag}/{package.name}',
            'digest': 'sha256:' + hashlib.sha256(data).hexdigest(),
            'size': len(data),
        }],
    }


class SimulatedSystem:
    """GitHub, the administrator prompt and dpkg's database, as the updater sees them.

    dpkg-deb runs for real, so the fields it reads are the package's own.
    """

    def __init__(self, updater: ModuleType, package: Path, version: str) -> None:
        self.answer = release_answer(updater, package, version)
        self.latest_url: str = updater.LATEST
        self.package = package
        self.version = version
        self.commands: list[list[str]] = []

    def open_url(self, url: str) -> io.BytesIO:
        """Answer the updater's two requests: the latest release, then the asset."""
        if url == self.latest_url:
            return io.BytesIO(json.dumps(self.answer).encode())
        if url == self.answer['assets'][0]['browser_download_url']:
            return io.BytesIO(self.package.read_bytes())
        raise UpdaterRefused(f'The updater requested an unexpected address: {url}')

    def run(self, argv: list[str], **options: Any) -> subprocess.CompletedProcess[str]:
        """Run dpkg-deb for real; accept the prompt and report the new version installed."""
        self.commands.append(list(argv))
        if argv[0] == PROMPT:
            return subprocess.CompletedProcess(argv, 0, '', '')
        if argv[0] == PACKAGE_QUERY:
            return subprocess.CompletedProcess(argv, 0, f'install ok installed\n{self.version}', '')
        result: subprocess.CompletedProcess[str] = subprocess.run(argv, **options)
        return result


def check_install(package: Path, version: str) -> None:
    """Raise UpdaterRefused unless the 1.1.x updater offers and installs package."""
    updater = python_updater()
    system = SimulatedSystem(updater, package, version)

    class PackagedUpdater(updater.Updater):  # type: ignore[name-defined, misc]
        """The updater as it runs from the installed Python package."""

        def can_install(self) -> bool:
            return True

    with tempfile.TemporaryDirectory(prefix='openxplorer-updater-') as temporary:
        instance = PackagedUpdater(current=updater.VERSION, root=PYTHON_PACKAGE_ROOT,
                                   directory=Path(temporary) / 'updates',
                                   opener=system.open_url, run=system.run)
        try:
            status = instance.check()
            if not status['available']:
                raise UpdaterRefused(f'{version} is not newer than {updater.VERSION}.')
            result = instance.install(version, True)
        except ValueError as error:
            raise UpdaterRefused(f'The 1.1.x updater refused the package: {error}') from error
    if result != {'installed': True, 'version': version}:
        raise UpdaterRefused(f'The 1.1.x updater reported {result!r}.')
    installs = [argv for argv in system.commands if argv[0] == PROMPT]
    if len(installs) != 1 or not installs[0][-1].endswith(package.name):
        raise UpdaterRefused(f'The updater did not install the package once: {installs!r}')
