#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Install the native app's files into a package's staging folder.

The Debian package, the RPM, the Arch package and the Flatpak all install
through this tool, so the program, the desktop entry, the AppStream
metainfo, the icon, the licences and the mount helper land in the same
places with the same modes whichever format builds them. It ports the file
layout of desktop/tools/build_deb.py; native/packaging/README.md explains
each decision.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterator
from dataclasses import dataclass
import enum
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
from typing import Any

REPOSITORY = Path(__file__).resolve().parents[2]
NATIVE = REPOSITORY / 'native'
PACKAGING_DATA = NATIVE / 'packaging' / 'data'
CARGO_MANIFEST = NATIVE / 'Cargo.toml'

# The launcher icon is the Fluent Emoji file folder the app shows for
# folders, byte for byte (crates/ox-app/resources/icons/SOURCES.md): the
# owner's rule is that every icon is an unmodified Fluent file.
APP_ICON = NATIVE / 'crates/ox-app/resources/icons/hicolor/scalable/places/ox-file-folder-flat.svg'

# The administrator mount helper and the modules it imports, in import
# order: mount_share.py -> mount_support.py -> core.py -> private_storage.py.
# All four use only the standard library.
MOUNT_HELPER_MODULES = ('mount_share.py', 'mount_support.py', 'core.py', 'private_storage.py')
MOUNT_HELPER_LAUNCHER = PACKAGING_DATA / 'openxplorer-mount-share.in'
MOUNT_HELPER_COMMAND = 'openxplorer-mount-share'

# The Python package's older command names, which the stable package keeps.
# /usr/bin/winspace is also the Exec line of the opt-in "Show in folder"
# service file (SERVICE_FILE in crates/ox-core/src/integration/reveal.rs).
LEGACY_COMMAND = 'winspace'
LEGACY_MOUNT_HELPER_COMMAND = 'winspace-mount-share'

# The licence texts every package installs.
LICENCE_FILES = (REPOSITORY / 'LICENSE', REPOSITORY / 'THIRD_PARTY_NOTICES.md')
LICENCE_FOLDER = REPOSITORY / 'licenses'

# How the files a Rust crate ships its licence in start, compared without
# regard to case (LICENSE-MIT, LICENCE, COPYING, NOTICE and so on).
CRATE_LICENCE_PREFIXES = ('license', 'licence', 'copying', 'copyright', 'notice', 'unlicense')

PROGRAM_MODE = 0o755
DATA_MODE = 0o644


class Channel(enum.Enum):
    """Which app a package installs, named by its application ID."""

    PREVIEW = 'io.winspace.Development.Native'
    STABLE = 'io.winspace.Development'

    @property
    def app_id(self) -> str:
        """The application ID of the binary, desktop entry, metainfo and Flatpak."""
        return self.value

    @property
    def package(self) -> str:
        """The package name, which is also the command.

        The stable package takes over the Python package's name, so upgrading
        replaces it; the preview's own name installs it beside that package.
        """
        return 'openxplorer' if self is Channel.STABLE else 'openxplorer-native'

    @property
    def is_python_replacement(self) -> bool:
        """True for the stable app, which also takes over the Python package's commands."""
        return self is Channel.STABLE


class Layout(enum.Enum):
    """Where a package format installs the program and its helpers."""

    # /opt/<package>/bin, because the in-app updater recognises the Debian
    # package by that folder (Installation::detect in ox-core's update).
    DEBIAN = 'debian'
    # /usr/bin, as Fedora, openSUSE and Arch package programs.
    FHS = 'fhs'
    # /app, the Flatpak's own prefix.
    FLATPAK = 'flatpak'


@dataclass(frozen=True)
class InstalledPaths:
    """Where one package's files are on the installed system."""

    program: PurePosixPath
    commands: PurePosixPath
    share: PurePosixPath
    licences: PurePosixPath
    # The mount helper's folder where the package takes over the Python
    # package's commands (winspace and the mount helper), else None. Only
    # the stable host packages do: the preview installs beside the Python
    # package, and a Flatpak cannot add commands to the host.
    mount_helper: PurePosixPath | None


def installed_paths(channel: Channel, layout: Layout) -> InstalledPaths:
    """Return where a package of this channel and layout installs its files."""
    package = channel.package
    if layout is Layout.FLATPAK:
        return InstalledPaths(
            program=PurePosixPath('/app/bin', package),
            commands=PurePosixPath('/app/bin'),
            share=PurePosixPath('/app/share'),
            licences=PurePosixPath('/app/share/licenses', channel.app_id),
            mount_helper=None)
    has_helper = channel.is_python_replacement
    if layout is Layout.DEBIAN:
        home = PurePosixPath('/opt', package)
        return InstalledPaths(
            program=home / 'bin' / package,
            commands=PurePosixPath('/usr/bin'),
            share=PurePosixPath('/usr/share'),
            licences=PurePosixPath('/usr/share/doc', package),
            mount_helper=home / 'mount-share' if has_helper else None)
    return InstalledPaths(
        program=PurePosixPath('/usr/bin', package),
        commands=PurePosixPath('/usr/bin'),
        share=PurePosixPath('/usr/share'),
        licences=PurePosixPath('/usr/share/licenses', package),
        mount_helper=PurePosixPath('/usr/share', package, 'mount-share') if has_helper else None)


@dataclass(frozen=True)
class Staging:
    """A package's staging folder, which stands for '/' of the installed system."""

    root: Path

    def path_of(self, installed: PurePosixPath) -> Path:
        """Return the staging path of an installed path."""
        return self.root / installed.relative_to('/')

    def copy(self, source: Path, installed: PurePosixPath, mode: int) -> None:
        """Copy a file's bytes to an installed path and give it mode."""
        target = self.path_of(installed)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(mode)

    def write(self, text: str, installed: PurePosixPath, mode: int) -> None:
        """Write text to an installed path and give it mode."""
        target = self.path_of(installed)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding='utf-8')
        target.chmod(mode)

    def link(self, installed: PurePosixPath, points_to: PurePosixPath) -> None:
        """Create a relative symbolic link, so it resolves inside the staging folder too."""
        target = self.path_of(installed)
        target.parent.mkdir(parents=True, exist_ok=True)
        relative = os.path.relpath(points_to, installed.parent)
        target.symlink_to(relative)


@dataclass(frozen=True)
class InstallRequest:
    """What to install, and where to put it."""

    channel: Channel
    layout: Layout
    # The executable Cargo built (target/release/openxplorer-native).
    program: Path
    staging: Path


def install(request: InstallRequest) -> None:
    """Install every file of one package into its staging folder.

    Raises OSError when a source file is missing or the staging folder is not
    writable, and subprocess.CalledProcessError when Cargo cannot list the
    crates.
    """
    paths = installed_paths(request.channel, request.layout)
    staging = Staging(request.staging)
    install_program(staging, request.channel, request.program, paths)
    install_desktop_data(staging, request.channel, paths.share)
    install_licences(staging, paths.licences)
    install_crate_licences(staging, paths.licences / 'rust-crates')
    if paths.mount_helper is not None:
        install_python_commands(staging, request.channel, paths.commands, paths.mount_helper)


def install_program(staging: Staging, channel: Channel, program: Path,
                    paths: InstalledPaths) -> None:
    """Install the program and, where it lives outside PATH, its command."""
    staging.copy(program, paths.program, PROGRAM_MODE)
    command = paths.commands / channel.package
    if command != paths.program:
        staging.link(command, paths.program)


def install_desktop_data(staging: Staging, channel: Channel, share: PurePosixPath) -> None:
    """Install the desktop entry, the AppStream metainfo and the launcher icon."""
    app_id = channel.app_id
    staging.copy(PACKAGING_DATA / f'{app_id}.desktop',
                 share / 'applications' / f'{app_id}.desktop', DATA_MODE)
    staging.copy(PACKAGING_DATA / f'{app_id}.metainfo.xml',
                 share / 'metainfo' / f'{app_id}.metainfo.xml', DATA_MODE)
    staging.copy(APP_ICON, share / 'icons/hicolor/scalable/apps' / f'{app_id}.svg', DATA_MODE)


def install_licences(staging: Staging, licences: PurePosixPath) -> None:
    """Install the AGPL, the third-party notices and every text in licenses/."""
    for source in LICENCE_FILES:
        staging.copy(source, licences / source.name, DATA_MODE)
    for source in sorted(LICENCE_FOLDER.iterdir()):
        staging.copy(source, licences / 'licenses' / source.name, DATA_MODE)


def install_python_commands(staging: Staging, channel: Channel, commands: PurePosixPath,
                            helper: PurePosixPath) -> None:
    """Install winspace, the Python mount helper, its launcher and its legacy name."""
    staging.link(commands / LEGACY_COMMAND, commands / channel.package)
    for module in MOUNT_HELPER_MODULES:
        staging.copy(REPOSITORY / 'desktop' / module, helper / module, DATA_MODE)
    template = MOUNT_HELPER_LAUNCHER.read_text(encoding='utf-8')
    launcher = template.replace('@HELPER_DIRECTORY@', str(helper))
    command = commands / MOUNT_HELPER_COMMAND
    staging.write(launcher, command, PROGRAM_MODE)
    staging.link(commands / LEGACY_MOUNT_HELPER_COMMAND, command)


@dataclass(frozen=True)
class Crate:
    """A third-party Rust crate compiled into the program."""

    name: str
    version: str
    licence: str
    folder: Path


def install_crate_licences(staging: Staging, destination: PurePosixPath) -> None:
    """Install each compiled crate's licence files and an index of their licences.

    MIT and Apache-2.0 require their notices to travel with the binary.
    """
    crates = program_crates(cargo_metadata())
    index = [f'{crate.name} {crate.version}: {crate.licence}' for crate in crates]
    staging.write('\n'.join(index) + '\n', destination / 'INDEX.txt', DATA_MODE)
    for crate in crates:
        folder = destination / f'{crate.name}-{crate.version}'
        for source in licence_files(crate.folder):
            staging.copy(source, folder / source.name, DATA_MODE)


def cargo_metadata() -> dict[str, Any]:
    """Return Cargo's description of the workspace, resolved for this computer.

    --offline works in the Flatpak and RPM builds, whose sources are vendored,
    and after any build, which has downloaded every crate.
    """
    command = ['cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
               '--filter-platform', host_target(), '--manifest-path', str(CARGO_MANIFEST)]
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    metadata: dict[str, Any] = json.loads(result.stdout)
    return metadata


def host_target() -> str:
    """Return the target triple rustc builds for on this computer."""
    result = subprocess.run(['rustc', '-vV'], capture_output=True, text=True, check=True)
    for line in result.stdout.splitlines():
        if line.startswith('host: '):
            return line.removeprefix('host: ')
    raise RuntimeError('rustc -vV did not name its host target.')


def program_crates(metadata: dict[str, Any]) -> list[Crate]:
    """Return the third-party crates the program links, sorted by name and version.

    Only normal dependencies reachable from ox-app count: build scripts and
    tests are not part of the installed program.
    """
    packages = {package['id']: package for package in metadata['packages']}
    reachable = normal_dependencies(metadata['resolve']['nodes'], app_package_id(packages))
    crates = [
        Crate(name=package['name'], version=package['version'],
              licence=package.get('license') or 'see its licence files',
              folder=Path(package['manifest_path']).parent)
        for package in (packages[package_id] for package_id in reachable)
        if package['source'] is not None
    ]
    return sorted(crates, key=lambda crate: (crate.name, crate.version))


def app_package_id(packages: dict[str, dict[str, Any]]) -> str:
    """Return the Cargo package id of ox-app."""
    for package_id, package in packages.items():
        if package['name'] == 'ox-app' and package['source'] is None:
            return package_id
    raise RuntimeError('cargo metadata does not list the ox-app package.')


def normal_dependencies(nodes: list[dict[str, Any]], root: str) -> set[str]:
    """Return every package reachable from root through normal dependencies."""
    edges = {node['id']: list(normal_edges(node)) for node in nodes}
    reachable = {root}
    pending = [root]
    while pending:
        for dependency in edges[pending.pop()]:
            if dependency not in reachable:
                reachable.add(dependency)
                pending.append(dependency)
    return reachable


def normal_edges(node: dict[str, Any]) -> Iterator[str]:
    """Yield the package ids a resolve node depends on as a normal dependency."""
    for dependency in node['deps']:
        kinds = {kind['kind'] for kind in dependency['dep_kinds']}
        if None in kinds:
            yield dependency['pkg']


def licence_files(folder: Path) -> list[Path]:
    """Return the licence files at the top of a crate's folder, sorted by name."""
    return sorted(
        path for path in folder.iterdir()
        if path.is_file() and path.name.casefold().startswith(CRATE_LICENCE_PREFIXES))


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    """Read the command-line options."""
    parser = argparse.ArgumentParser(
        description="Install the native app's files for one package format into a staging "
                    'folder, as DESTDIR installs do.')
    parser.add_argument('--app-id', required=True, choices=[channel.value for channel in Channel],
                        help='the application ID the program was built with (OX_APP_ID)')
    parser.add_argument('--layout', required=True, choices=[layout.value for layout in Layout],
                        help='debian for the .deb, fhs for RPM and Arch, flatpak for the Flatpak')
    parser.add_argument('--program', required=True, type=Path,
                        help='the openxplorer-native executable Cargo built')
    parser.add_argument('--destdir', required=True, type=Path,
                        help='the staging folder that stands for / (use / inside Flatpak)')
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    """Install one package's files and return the exit status."""
    arguments = parse_arguments(argv)
    request = InstallRequest(channel=Channel(arguments.app_id), layout=Layout(arguments.layout),
                             program=arguments.program, staging=arguments.destdir)
    try:
        install(request)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'Installing the package files failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
