#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Verify a native Debian package without installing it.

Ports v2.0.0:desktop/tools/verify_deb.py. It checks the package's identity and
dependencies, that its payload is safe to unpack (no absolute or traversing
paths, no links out of the package, root-owned, nothing group- or
world-writable), the maintainer scripts, the checksums, the processor the
program is built for and the AppStream catalog; verify_layout.py checks the
installed files. For the stable package it also proves that the in-app
updater of OpenXplorer 1.1.x in v2.0.0:desktop/updater.py downloads, inspects and
installs it. It is not a runtime test of the app.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tarfile
import tempfile
import xml.etree.ElementTree as ElementTree

from build_deb import (DEBIAN_DATA, GTK_PACKAGE, MAINTAINER_SCRIPTS_REFRESHING_CACHES,
                       PackageIdentity, build_architecture, cargo_version, package_identity)
from package_data import Channel, Layout
import updater_compatibility
from verify_layout import InstalledTree, Report, VerificationError, verify_tree

# Commands a maintainer script must never run: installing may not change
# default applications, mounts, folders or user data (UPD-018).
FORBIDDEN_SCRIPT_COMMANDS = ('xdg-mime', 'mount.cifs', 'systemctl', 'rm ', 'settings.json',
                             '/home/', 'gsettings')
# The ELF machine numbers of the architectures the package may be built on.
ELF_MACHINES = {'amd64': 0x3E, 'arm64': 0xB7}
WRITABLE_BY_OTHERS = 0o022


def control_fields(package: Path) -> dict[str, str]:
    """Return the package's control fields, continuation lines joined."""
    text = subprocess.run(['dpkg-deb', '--field', str(package)], capture_output=True, text=True,
                          check=True).stdout
    fields: dict[str, str] = {}
    name = ''
    for line in text.splitlines():
        if line.startswith(' ') and name:
            fields[name] += '\n' + line
        else:
            name, _, value = line.partition(': ')
            fields[name] = value
    return fields


def payload_members(package: Path) -> dict[str, tarfile.TarInfo]:
    """Return the payload's entries by path, without the leading './'."""
    payload = subprocess.run(['dpkg-deb', '--fsys-tarfile', str(package)], capture_output=True,
                             check=True).stdout
    with tarfile.open(fileobj=io.BytesIO(payload), mode='r:') as archive:
        return {member.name.removeprefix('./'): member for member in archive.getmembers()}


def check_identity(report: Report, fields: dict[str, str], channel: Channel,
                   identity: PackageIdentity) -> None:
    """Check the name, version, architecture and relations."""
    report.check("Package name is the channel's", fields.get('Package') == identity.package)
    report.check('Version is the Cargo workspace version',
                 fields.get('Version') == identity.version)
    report.check('Architecture follows the channel',
                 fields.get('Architecture') == identity.architecture)
    report.check('Official project homepage', fields.get('Homepage') == 'https://openxplorer.app')
    if channel is Channel.STABLE:
        report.check('Replaces the pre-0.8 package without deleting user data',
                     'winspace-explorer' in fields.get('Replaces', '')
                     and 'winspace-explorer' in fields.get('Conflicts', ''))


def check_dependencies(report: Report, fields: dict[str, str], channel: Channel) -> None:
    """Check the dependency grouping native/packaging/README.md describes."""
    depends = fields.get('Depends', '')
    recommends = fields.get('Recommends', '')
    report.check('GTK 4.14 or newer is required', f'{GTK_PACKAGE} (>= 4.14)' in depends)
    report.check('GLib comes from the linked libraries', 'libglib2.0-0' in depends)
    report.check('GVfs, a Secret Service, xdg-utils and a terminal are recommended',
                 all(name in recommends for name in
                     ('gvfs-backends', 'gvfs-fuse', 'gnome-keyring', 'xdg-utils', 'terminal')))
    if channel is Channel.STABLE:
        report.check('Updates and the mount helper have their programs recommended',
                     all(name in recommends for name in ('pkexec', 'cifs-utils')))
    report.check('Python is neither required nor recommended',
                 not any(name.strip().startswith('python')
                         for field in (depends, recommends)
                         for alternatives in field.split(',')
                         for name in alternatives.split('|')))


def check_payload_safety(report: Report, members: dict[str, tarfile.TarInfo]) -> None:
    """Check that unpacking cannot escape the package or leave writable files."""
    paths = [PurePosixPath(name) for name in members]
    report.check('No absolute or traversing paths',
                 all(not path.is_absolute() and '..' not in path.parts for path in paths))
    report.check('Every entry is owned by root:root',
                 all(member.uid == 0 and member.gid == 0 for member in members.values()))
    report.check('Nothing is group- or world-writable',
                 all(member.issym() or not member.mode & WRITABLE_BY_OTHERS
                     for member in members.values()))
    report.check('No hard links', not any(member.islnk() for member in members.values()))
    links = {name: member for name, member in members.items() if member.issym()}
    report.check('Every symbolic link is relative and points into the package',
                 all(link_stays_inside(name, member, members) for name, member in links.items()))


def link_stays_inside(name: str, member: tarfile.TarInfo,
                      members: dict[str, tarfile.TarInfo]) -> bool:
    """Return whether a relative link resolves to another entry of the package."""
    if member.linkname.startswith('/'):
        return False
    parts: list[str] = []
    for part in (PurePosixPath(name).parent / member.linkname).parts:
        if part == '..':
            if not parts:
                return False
            parts.pop()
        elif part != '.':
            parts.append(part)
    return '/'.join(parts) in members


def check_maintainer_scripts(report: Report, channel: Channel, control: Path) -> None:
    """Check the scripts: cache refreshes for both channels, the preinst for the stable one."""
    scripts = sorted(path.name for path in control.iterdir()
                     if path.name in ('preinst', 'postinst', 'prerm', 'postrm'))
    expected = sorted([*MAINTAINER_SCRIPTS_REFRESHING_CACHES,
                       *(['preinst'] if channel is Channel.STABLE else [])])
    report.check('Only the expected maintainer scripts are installed', scripts == expected)
    refresh = (DEBIAN_DATA / 'refresh-caches').read_bytes()
    report.check('postinst and postrm only refresh caches',
                 all((control / name).read_bytes() == refresh
                     for name in MAINTAINER_SCRIPTS_REFRESHING_CACHES))
    for name in scripts:
        script = control / name
        subprocess.run(['sh', '-n', str(script)], check=True)
        commands = script_commands(script.read_text(encoding='utf-8'))
        report.check(f'{name} changes no defaults, mounts or user data',
                     not any(forbidden in commands for forbidden in FORBIDDEN_SCRIPT_COMMANDS))


def script_commands(text: str) -> str:
    """Return a shell script without its comment lines, which may name what it avoids."""
    lines = [line for line in text.splitlines() if not line.lstrip().startswith('#')]
    return '\n'.join(lines)


def check_md5sums(report: Report, root: Path) -> None:
    """Check every DEBIAN/md5sums line against the unpacked file."""
    for line in (root / 'DEBIAN/md5sums').read_text(encoding='utf-8').splitlines():
        digest, relative = line.split('  ', 1)
        actual = hashlib.md5((root / relative).read_bytes(), usedforsecurity=False).hexdigest()
        if digest != actual:
            raise VerificationError(f'Checksum mismatch: {relative}')
    report.check('Every md5sums entry matches its file', True)


def elf_machine(program: Path) -> int:
    """Return the e_machine field of a little-endian ELF executable."""
    header = program.read_bytes()[:20]
    if header[:4] != b'\x7fELF':
        raise VerificationError(f'{program.name} is not an ELF executable')
    return int.from_bytes(header[18:20], 'little')


def check_debian_files(report: Report, tree: InstalledTree) -> None:
    """Check the copyright file and the catalog that names the package in GNOME Software."""
    package = tree.channel.package
    copyright_text = (tree.root / 'usr/share/doc' / package / 'copyright').read_text()
    report.check('Machine-readable copyright with the AGPL',
                 'License: AGPL-3.0-only' in copyright_text)
    catalog = ElementTree.parse(tree.root / 'usr/share/swcatalog/xml' / f'{package}.xml')
    component = catalog.getroot().find('component')
    report.check('The AppStream catalog maps the package name to the application',
                 component is not None and component.findtext('pkgname') == package
                 and component.findtext('id') == tree.channel.app_id)


def check_unpacked(report: Report, tree: InstalledTree, architecture: str) -> None:
    """Check the unpacked package: layout, scripts, checksums, program and catalog."""
    verify_tree(report, tree)
    check_maintainer_scripts(report, tree.channel, tree.root / 'DEBIAN')
    check_md5sums(report, tree.root)
    check_debian_files(report, tree)
    if architecture in ELF_MACHINES:
        program = tree.path_of(tree.paths.program)
        report.check("The program is built for the package's processor",
                     elf_machine(program) == ELF_MACHINES[architecture])


def channel_of(fields: dict[str, str]) -> Channel:
    """Return the channel whose package name the control fields carry."""
    for channel in Channel:
        if channel.package == fields.get('Package'):
            return channel
    raise VerificationError(f'Unknown package name {fields.get("Package")!r}')


def verify(package: Path) -> dict[str, object]:
    """Run every check on a package and return the report as JSON-ready data."""
    report = Report()
    fields = control_fields(package)
    channel = channel_of(fields)
    architecture = build_architecture()
    identity = package_identity(channel, cargo_version(), architecture)
    check_identity(report, fields, channel, identity)
    check_dependencies(report, fields, channel)
    check_payload_safety(report, payload_members(package))
    with tempfile.TemporaryDirectory(prefix='openxplorer-verify-') as temporary:
        root = Path(temporary)
        subprocess.run(['dpkg-deb', '--extract', str(package), str(root)], check=True)
        subprocess.run(['dpkg-deb', '--control', str(package), str(root / 'DEBIAN')], check=True)
        tree = InstalledTree(root, channel, Layout.DEBIAN, identity.version)
        check_unpacked(report, tree, architecture)
    if channel is Channel.STABLE:
        updater_compatibility.check_install(package, identity.version)
        report.check('The OpenXplorer 1.1.x updater downloads, inspects and installs it', True)
    return {'package': package.name, 'bytes': package.stat().st_size,
            'sha256': hashlib.sha256(package.read_bytes()).hexdigest(),
            'checks': report.passed, 'nativeRuntimeTested': False}


def main(argv: list[str] | None = None) -> int:
    """Verify one package, print the report and return the exit status."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('package', type=Path, help='the .deb to verify')
    arguments = parser.parse_args(argv)
    try:
        result = verify(arguments.package.resolve())
    except (VerificationError, OSError, KeyError, subprocess.CalledProcessError,
            updater_compatibility.UpdaterRefused) as error:
        print(f'Package verification failed: {error}', file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())
