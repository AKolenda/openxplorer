#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Verify a native Debian package without installing it.

Ports desktop/tools/verify_deb.py. It checks the package's identity and
dependencies, that its payload is safe to unpack (no absolute or traversing
paths, no links out of the package, root-owned, nothing group- or
world-writable), that every promised file is there, that installing changes no
user state, the checksums, the desktop entry and the metainfo, and, for the
stable package, that the in-app updater of OpenXplorer 1.1.x in desktop/updater.py
downloads, inspects and installs it. It is not a runtime test of the app.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile

from build_deb import GTK_PACKAGE, PackageIdentity, cargo_version, package_identity
import package_data
from package_data import REPOSITORY, Channel, Layout
import updater_compatibility

# Commands a maintainer script must never run: installing may not change
# default applications, mounts, folders or user data (UPD-018).
FORBIDDEN_SCRIPT_TEXT = ('xdg-mime', 'mount', 'rm -rf', 'settings.json', '/home/')
# A system-wide FileManager1 service would take "Show in folder" requests
# from every user without asking (the opt-in rule in AGENTS.md).
SYSTEM_FILE_MANAGER_SERVICE = 'usr/share/dbus-1/services/org.freedesktop.FileManager1.service'
# The ELF machine numbers of the architectures the package may be built on.
ELF_MACHINES = {'amd64': 0x3E, 'arm64': 0xB7}
WRITABLE_BY_OTHERS = 0o022


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
    report.check('Package name is the channel\'s', fields.get('Package') == identity.package)
    report.check('Version is the Cargo workspace version', fields.get('Version') == identity.version)
    report.check('Architecture follows the channel',
                 fields.get('Architecture') == identity.architecture)
    if channel is Channel.STABLE:
        report.check('Replaces the pre-0.8 package without deleting user data',
                     'winspace-explorer' in fields.get('Replaces', '')
                     and 'winspace-explorer' in fields.get('Conflicts', ''))


def check_dependencies(report: Report, fields: dict[str, str]) -> None:
    """Check the dependency grouping native/packaging/README.md describes."""
    depends = fields.get('Depends', '')
    recommends = fields.get('Recommends', '')
    report.check('GTK 4.14 or newer is required', f'{GTK_PACKAGE} (>= 4.14)' in depends)
    report.check('GLib comes from the linked libraries', 'libglib2.0-0' in depends)
    report.check('GVfs backends and a Secret Service are recommended',
                 all(name in recommends for name in ('gvfs-backends', 'gnome-keyring')))
    report.check('xdg-utils and a terminal are only suggested',
                 all(name in fields.get('Suggests', '') for name in ('xdg-utils', 'terminal')))


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
    report.check('Every symbolic link is relative and points into the package',
                 all(link_stays_inside(name, member, members)
                     for name, member in members.items() if member.issym()))


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


def expected_files(channel: Channel) -> list[str]:
    """Return the payload paths the channel's package must contain."""
    paths = package_data.installed_paths(channel, Layout.DEBIAN)
    app_id = channel.app_id
    files = [paths.program, paths.commands / channel.package,
             paths.share / 'applications' / f'{app_id}.desktop',
             paths.share / 'metainfo' / f'{app_id}.metainfo.xml',
             paths.share / 'icons/hicolor/scalable/apps' / f'{app_id}.svg',
             paths.share / 'swcatalog/xml' / f'{channel.package}.xml',
             paths.licences / 'copyright', paths.licences / 'LICENSE',
             paths.licences / 'rust-crates/INDEX.txt']
    if paths.mount_helper is not None:
        files += [paths.commands / name for name in
                  (package_data.LEGACY_COMMAND, package_data.MOUNT_HELPER_COMMAND,
                   package_data.LEGACY_MOUNT_HELPER_COMMAND)]
        files += [paths.mount_helper / module for module in package_data.MOUNT_HELPER_MODULES]
    return [path.relative_to('/').as_posix() for path in files]


def check_contents(report: Report, channel: Channel, members: dict[str, tarfile.TarInfo]) -> None:
    """Check the promised files are there and no system-wide service is installed."""
    missing = [path for path in expected_files(channel) if path not in members]
    if missing:
        raise VerificationError(f'Promised files are missing: {", ".join(missing)}')
    report.check('Every promised file is installed', True)
    report.check('No system-wide FileManager1 service',
                 SYSTEM_FILE_MANAGER_SERVICE not in members)
    report.check('No personal settings, caches or databases',
                 not any(name.startswith(('home/', 'root/', 'etc/'))
                         or name.endswith(('.sqlite3', 'settings.json')) for name in members))


def check_unpacked(report: Report, channel: Channel, root: Path, architecture: str) -> None:
    """Check the unpacked package's scripts, checksums, program and metadata."""
    check_maintainer_scripts(report, channel, root / 'DEBIAN')
    check_md5sums(report, root)
    paths = package_data.installed_paths(channel, Layout.DEBIAN)
    program = root / paths.program.relative_to('/')
    if architecture in ELF_MACHINES:
        report.check('The program is built for the package\'s processor',
                     elf_machine(program) == ELF_MACHINES[architecture])
    if paths.mount_helper is not None:
        helper = root / paths.mount_helper.relative_to('/')
        report.check('The mount helper is the Python app\'s, byte for byte',
                     all((helper / module).read_bytes()
                         == (REPOSITORY / 'desktop' / module).read_bytes()
                         for module in package_data.MOUNT_HELPER_MODULES))
    check_desktop_metadata(report, channel, root / paths.share.relative_to('/'))


def check_maintainer_scripts(report: Report, channel: Channel, control: Path) -> None:
    """Check that only the stable preinst exists and that it changes no user state."""
    scripts = sorted(path.name for path in control.iterdir()
                     if path.name in ('preinst', 'postinst', 'prerm', 'postrm'))
    expected = ['preinst'] if channel is Channel.STABLE else []
    report.check('Only the stable package has a maintainer script, its preinst',
                 scripts == expected)
    for name in scripts:
        script = control / name
        subprocess.run(['sh', '-n', str(script)], check=True)
        text = script.read_text(encoding='utf-8')
        report.check(f'{name} changes no defaults, mounts or user data',
                     not any(forbidden in text for forbidden in FORBIDDEN_SCRIPT_TEXT))


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


def check_desktop_metadata(report: Report, channel: Channel, share: Path) -> None:
    """Validate the desktop entry and the metainfo with the desktop's own tools."""
    desktop = share / 'applications' / f'{channel.app_id}.desktop'
    metainfo = share / 'metainfo' / f'{channel.app_id}.metainfo.xml'
    for tool, arguments in (('desktop-file-validate', [str(desktop)]),
                            ('appstreamcli', ['validate', '--no-net', str(metainfo)])):
        if shutil.which(tool) is None:
            raise VerificationError(f'{tool} is not installed (desktop-file-utils, appstream)')
        subprocess.run([tool, *arguments], check=True, stdout=subprocess.DEVNULL)
    report.check('desktop-file-validate and appstreamcli validate pass', True)


def verify(package: Path) -> dict[str, object]:
    """Run every check on a package and return the report as JSON-ready data."""
    report = Report()
    fields = control_fields(package)
    channel = next((channel for channel in Channel if channel.package == fields.get('Package')),
                   None)
    if channel is None:
        raise VerificationError(f'Unknown package name {fields.get("Package")!r}')
    architecture = subprocess.run(['dpkg', '--print-architecture'], capture_output=True,
                                  text=True, check=True).stdout.strip()
    identity = package_identity(channel, cargo_version(), architecture)
    check_identity(report, fields, channel, identity)
    check_dependencies(report, fields)
    members = payload_members(package)
    check_payload_safety(report, members)
    check_contents(report, channel, members)
    with tempfile.TemporaryDirectory(prefix='openxplorer-verify-') as temporary:
        root = Path(temporary)
        subprocess.run(['dpkg-deb', '--extract', str(package), str(root)], check=True)
        subprocess.run(['dpkg-deb', '--control', str(package), str(root / 'DEBIAN')], check=True)
        check_unpacked(report, channel, root, architecture)
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
    except (VerificationError, OSError, subprocess.CalledProcessError,
            updater_compatibility.UpdaterRefused) as error:
        print(f'Package verification failed: {error}', file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())
