#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Verify the files a native package installs, whatever its format.

A .deb, RPM or Arch package is unpacked into a folder, which stands for '/'
of the installed system; a Flatpak's files folder stands for /app.
This checks that every file package_data.py promises is there, that the
desktop entry, D-Bus service file and AppStream metainfo name the program,
icon and application ID the channel requires, that the desktop's own
validators accept them, that nothing touches user state, and that no
package ships Python. It ports the
layout checks of desktop/tools/verify_deb.py; verify_deb.py adds the Debian
ones.
"""
from __future__ import annotations

import argparse
import configparser
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ElementTree

import package_data
from package_data import Channel, InstalledPaths, Layout

# A system-wide FileManager1 service would take "Show in folder" requests
# from every user without asking (the opt-in rule in AGENTS.md).
SYSTEM_FILE_MANAGER_SERVICE = 'dbus-1/services/org.freedesktop.FileManager1.service'
# Installed paths that would hold personal settings, caches or databases.
PERSONAL_PREFIXES = ('home/', 'root/', 'etc/')
PERSONAL_SUFFIXES = ('.sqlite3', 'settings.json')
# The first bytes of a compiled Linux program.
ELF_MAGIC = b'\x7fELF'
# The launcher keys the Python package's desktop entry has, which the
# stable entry keeps (DESKTOP in desktop/tools/build_deb.py).
STABLE_DESKTOP_KEYS = {
    'Exec': 'openxplorer %U',
    'Icon': 'io.winspace.Development',
    'StartupWMClass': 'io.winspace.Development',
    'MimeType': ('inode/directory;x-scheme-handler/smb;application/zip;application/x-zip;'
                 'application/x-zip-compressed;'),
    'Actions': 'NewWindow;Windows;Settings;',
}


class VerificationError(Exception):
    """A check failed; the message names it."""


class Report:
    """The names of the checks that passed, in order."""

    def __init__(self) -> None:
        self.passed: list[str] = []

    def check(self, name: str, condition: bool) -> None:
        """Record a passed check, or raise VerificationError naming the failed one."""
        if not condition:
            raise VerificationError(name)
        self.passed.append(name)


@dataclass(frozen=True)
class InstalledTree:
    """An unpacked package: its folder, and what it should hold."""

    root: Path
    channel: Channel
    layout: Layout
    version: str

    @property
    def paths(self) -> InstalledPaths:
        """Where package_data.py installs this package's files."""
        return package_data.installed_paths(self.channel, self.layout)

    def path_of(self, installed: PurePosixPath) -> Path:
        """Return where an installed path is in the unpacked folder."""
        anchor = '/app' if self.layout is Layout.FLATPAK else '/'
        return self.root / installed.relative_to(anchor)


def verify_tree(report: Report, tree: InstalledTree) -> None:
    """Run every layout check on an unpacked package."""
    check_promised_files(report, tree)
    check_no_user_state(report, tree)
    check_no_python(report, tree)
    check_desktop_entry(report, tree)
    check_service_file(report, tree)
    check_metainfo(report, tree)
    check_with_desktop_validators(report, tree)
    helper = tree.paths.mount_helper
    if helper is not None:
        check_mount_helper(report, tree, helper)


def promised_files(tree: InstalledTree) -> list[PurePosixPath]:
    """Return the installed paths the channel's package must contain."""
    paths = tree.paths
    app_id = tree.channel.app_id
    share = paths.share
    files = [paths.program, paths.command,
             share / 'applications' / f'{app_id}.desktop',
             share / 'metainfo' / f'{app_id}.metainfo.xml',
             share / 'icons/hicolor/scalable/apps' / f'{app_id}.svg',
             share / 'dbus-1/services' / f'{app_id}.service',
             paths.licences / 'LICENSE', paths.licences / 'rust-crates/INDEX.txt']
    if paths.mount_helper is not None:
        legacy = (package_data.LEGACY_COMMAND, package_data.MOUNT_HELPER_COMMAND,
                  package_data.LEGACY_MOUNT_HELPER_COMMAND)
        files += [paths.commands / name for name in legacy]
        files.append(paths.mount_helper)
    return files


def check_promised_files(report: Report, tree: InstalledTree) -> None:
    """Check that every promised file is installed and the commands are executable."""
    missing = [str(path) for path in promised_files(tree) if not tree.path_of(path).is_file()]
    if missing:
        raise VerificationError(f'Promised files are missing: {", ".join(missing)}')
    report.check('Every promised file is installed', True)
    command = tree.path_of(tree.paths.command)
    report.check('The command runs the program',
                 command.resolve() == tree.path_of(tree.paths.program).resolve()
                 and command.stat().st_mode & 0o111 != 0)


def check_no_user_state(report: Report, tree: InstalledTree) -> None:
    """Check that no system-wide FileManager1 service or personal data is installed."""
    share = tree.path_of(tree.paths.share)
    report.check('No system-wide FileManager1 service',
                 not (share / SYSTEM_FILE_MANAGER_SERVICE).exists())
    names = [path.relative_to(tree.root).as_posix() for path in tree.root.rglob('*')]
    report.check('No personal settings, caches or databases',
                 not any(name.startswith(PERSONAL_PREFIXES) or name.endswith(PERSONAL_SUFFIXES)
                         for name in names))


def check_no_python(report: Report, tree: InstalledTree) -> None:
    """Check that no Python module or script is installed: every program is native."""
    python = [path.relative_to(tree.root).as_posix() for path in tree.root.rglob('*')
              if path.is_file() and is_python(path)]
    if python:
        raise VerificationError(f'Python files are installed: {", ".join(sorted(python))}')
    report.check('No Python module or script is installed', True)


def is_python(path: Path) -> bool:
    """Return whether path is a Python module, compiled module or script."""
    if path.suffix in ('.py', '.pyc'):
        return True
    with path.open('rb') as file:
        first_line = file.readline(256)
    return first_line.startswith(b'#!') and b'python' in first_line


def key_file(path: Path) -> configparser.ConfigParser:
    """Read a desktop entry or D-Bus service file, keeping the keys' case.

    Both are key files in the Desktop Entry format: '#' comments, [groups]
    and key=value lines, which configparser reads without interpolation.
    """
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str  # type: ignore[assignment,method-assign]
    with path.open(encoding='utf-8') as file:
        parser.read_file(file)
    return parser


def desktop_entry_keys(tree: InstalledTree) -> dict[str, str]:
    """Return the [Desktop Entry] keys of the installed desktop entry."""
    app_id = tree.channel.app_id
    path = tree.path_of(tree.paths.share / 'applications' / f'{app_id}.desktop')
    return dict(key_file(path)['Desktop Entry'])


def check_desktop_entry(report: Report, tree: InstalledTree) -> None:
    """Check the launcher's command, icon and window class, and the stable contracts."""
    keys = desktop_entry_keys(tree)
    app_id = tree.channel.app_id
    command = tree.channel.package
    report.check('The launcher starts the command with the files it is given',
                 keys.get('Exec') == f'{command} %U' and keys.get('TryExec') == command)
    report.check('The launcher icon and window class are the application ID',
                 keys.get('Icon') == app_id and keys.get('StartupWMClass') == app_id)
    if tree.channel is Channel.STABLE:
        report.check("The launcher keeps the Python app's keys, MIME types and actions",
                     all(keys.get(name) == value for name, value in STABLE_DESKTOP_KEYS.items()))
    else:
        report.check('The preview claims no MIME type', 'MimeType' not in keys)


def check_service_file(report: Report, tree: InstalledTree) -> None:
    """Check that the D-Bus service file starts the command for the app's bus name."""
    app_id = tree.channel.app_id
    path = tree.path_of(tree.paths.share / 'dbus-1/services' / f'{app_id}.service')
    section = key_file(path)['D-BUS Service']
    command = tree.paths.command
    report.check('The D-Bus service starts the app as a GApplication service',
                 section.get('Name') == app_id
                 and section.get('Exec') == f'{command} --gapplication-service')


def check_metainfo(report: Report, tree: InstalledTree) -> None:
    """Check the metainfo's ID, launcher, icon, licences and newest release."""
    app_id = tree.channel.app_id
    path = tree.path_of(tree.paths.share / 'metainfo' / f'{app_id}.metainfo.xml')
    component = ElementTree.parse(path).getroot()
    report.check('AppStream ID, desktop ID and stock icon match',
                 component.findtext('id') == app_id
                 and component.findtext('launchable') == f'{app_id}.desktop'
                 and component.findtext('icon') == app_id)
    report.check('AppStream declares the AGPL project and CC0 metadata licences',
                 component.findtext('project_license') == 'AGPL-3.0-only'
                 and component.findtext('metadata_license') == 'CC0-1.0')
    newest = component.find('releases/release')
    report.check("AppStream's newest release is the package version",
                 newest is not None and newest.get('version') == tree.version)


def check_with_desktop_validators(report: Report, tree: InstalledTree) -> None:
    """Validate the desktop entry and the metainfo with the desktop's own tools."""
    share = tree.path_of(tree.paths.share)
    app_id = tree.channel.app_id
    desktop_entry = share / 'applications' / f'{app_id}.desktop'
    metainfo = share / 'metainfo' / f'{app_id}.metainfo.xml'
    validations = (
        ('desktop-file-validate', [str(desktop_entry)]),
        ('appstreamcli', ['validate', '--no-net', str(metainfo)]),
    )
    for tool, arguments in validations:
        if shutil.which(tool) is None:
            raise VerificationError(f'{tool} is not installed (desktop-file-utils, appstream)')
        subprocess.run([tool, *arguments], check=True, stdout=subprocess.DEVNULL)
    report.check('desktop-file-validate and appstreamcli validate pass', True)


def check_mount_helper(report: Report, tree: InstalledTree, helper: PurePosixPath) -> None:
    """Check that the mount helper commands run the native helper program, which starts."""
    installed = tree.path_of(helper)
    commands = tree.paths.commands
    names = (package_data.MOUNT_HELPER_COMMAND, package_data.LEGACY_MOUNT_HELPER_COMMAND)
    report.check('The mount helper commands run the helper program',
                 all(tree.path_of(commands / name).resolve() == installed.resolve()
                     for name in names)
                 and installed.stat().st_mode & 0o111 != 0)
    with installed.open('rb') as program:
        report.check('The mount helper is a compiled program', program.read(4) == ELF_MAGIC)
    subprocess.run([str(installed), '--help'], check=True, stdout=subprocess.DEVNULL)
    report.check('The mount helper starts', True)


def main(argv: list[str] | None = None) -> int:
    """Verify an unpacked package, print the passed checks and return the exit status."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--app-id', required=True, choices=[channel.value for channel in Channel],
                        help='the application ID the package installs')
    parser.add_argument('--layout', required=True, choices=[layout.value for layout in Layout],
                        help='debian, fhs (RPM and Arch) or flatpak')
    parser.add_argument('--version', required=True, help='the package version')
    parser.add_argument('root', type=Path, help='the folder the package was unpacked into')
    arguments = parser.parse_args(argv)
    tree = InstalledTree(arguments.root, Channel(arguments.app_id), Layout(arguments.layout),
                         arguments.version)
    report = Report()
    try:
        verify_tree(report, tree)
    except (VerificationError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(f'Package verification failed: {error}', file=sys.stderr)
        return 1
    print('\n'.join(report.passed))
    return 0


if __name__ == '__main__':
    sys.exit(main())
