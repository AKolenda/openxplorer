#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build the native app's Debian package, without installing it.

Ports desktop/tools/build_deb.py for the Rust program. The stable package is
openxplorer_<version>_all.deb, the only installer the in-app updater of
OpenXplorer 1.1.x downloads and accepts, so existing users move to the native
app with the update they are offered. The preview is
openxplorer-native_<version>_<architecture>.deb and installs beside the Python
app. native/packaging/README.md explains the dependencies and the layout.

The build is reproducible: file times come from SOURCE_DATE_EPOCH (or the
last commit), every entry is owned by root, directories are 0755, files 0644
or 0755, and DEBIAN/md5sums lists every file. It needs Cargo, dpkg-deb and
dpkg-shlibdeps (the dpkg-dev package on Debian and Ubuntu).
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import xml.etree.ElementTree as ElementTree

import package_data
from package_data import CARGO_MANIFEST, REPOSITORY, Channel, InstallRequest, Layout

OUTPUT_DIRECTORY = REPOSITORY / 'dist' / 'native'
DEBIAN_DATA = REPOSITORY / 'native' / 'packaging' / 'debian'
PYTHON_APP_CORE = REPOSITORY / 'desktop' / 'core.py'
CARGO_PROGRAM = 'openxplorer-native'

# The version form the 1.1.x updater accepts (version_tuple in
# desktop/updater.py): three numbers without leading zeros.
RELEASE_VERSION = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)')

MAINTAINER = 'OpenXplorer contributors <maintainer@example.invalid>'
HOMEPAGE = 'https://openxplorer.app'

# The dependency grouping native/packaging/README.md describes.
# Depends: dpkg-shlibdeps adds every library the program links, at the version
# its symbols need. GTK is raised to 4.14, the oldest version the app is built
# and tested against (the v4_14 feature in native/Cargo.toml).
GTK_PACKAGE = 'libgtk-4-1'
MINIMUM_GTK = f'{GTK_PACKAGE} (>= 4.14)'
# The folder the launcher icon is installed in.
DEPENDS = ('hicolor-icon-theme',)
# Recommends: what features beyond local browsing need. APT installs these by
# default, and the app explains a missing one when the feature is used.
RECOMMENDS = (
    # GVfs: smb:// and mtp:// browsing, the Recycle Bin and local paths of
    # shares for other applications.
    'gvfs',
    'gvfs-backends',
    'gvfs-fuse',
    # A Secret Service provider, which remembers SMB passwords.
    'gnome-keyring | keepassxc',
    # xdg-mime, which makes OpenXplorer the default file manager on request.
    'xdg-utils',
    # Open in Terminal, and Open in archive manager.
    'gnome-terminal | x-terminal-emulator',
    'file-roller',
)
STABLE_RECOMMENDS = (
    # The administrator prompt of in-app updates.
    'pkexec',
    # The persistent SMB mount helper, openxplorer-mount-share.
    'python3 (>= 3.10)',
    'cifs-utils',
)
# The maintainer scripts that run native/packaging/debian/refresh-caches.
MAINTAINER_SCRIPTS_REFRESHING_CACHES = ('postinst', 'postrm')
# The package name before 0.8.0, as desktop/tools/build_deb.py declares.
STABLE_RELATIONS = {
    'Replaces': 'winspace-explorer (<< 0.8.0)',
    'Conflicts': 'winspace-explorer (<< 0.8.0)',
    'Provides': 'winspace-explorer',
}
DESCRIPTIONS = {
    Channel.STABLE: (
        'Explorer-style local and SMB file manager\n'
        ' A file manager with the look of Windows 11 File Explorer, drawn with\n'
        ' native GTK 4 widgets: tabs, a resizable sidebar, SMB shares and phones\n'
        ' through GVfs, indexed filename search and ZIP browsing.\n'
        ' .\n'
        ' Installing it changes no default applications, mounts or user data.'),
    Channel.PREVIEW: (
        'Preview of the native OpenXplorer file manager\n'
        ' The next OpenXplorer, drawn with native GTK 4 widgets. It runs beside\n'
        ' the openxplorer package under its own application ID and shares its\n'
        ' settings.\n'
        ' .\n'
        ' Installing it changes no default applications, mounts or user data.'),
}


class BuildError(Exception):
    """The package cannot be built; the message says why and what to do."""


@dataclass(frozen=True)
class PackageIdentity:
    """The control fields that name one package file."""

    package: str
    version: str
    architecture: str

    @property
    def file_name(self) -> str:
        """The .deb name: <package>_<version>_<architecture>.deb."""
        return f'{self.package}_{self.version}_{self.architecture}.deb'


def cargo_version() -> str:
    """Return the workspace version in native/Cargo.toml, which the binary reports."""
    manifest = tomllib.loads(CARGO_MANIFEST.read_text(encoding='utf-8'))
    version: str = manifest['workspace']['package']['version']
    return version


def python_app_version() -> str:
    """Return the version of the Python app in desktop/core.py."""
    text = PYTHON_APP_CORE.read_text(encoding='utf-8')
    match = re.search(r"^VERSION = '([^']+)'$", text, re.MULTILINE)
    if match is None:
        raise BuildError(f'{PYTHON_APP_CORE} does not define VERSION.')
    return match.group(1)


def version_numbers(version: str) -> tuple[int, int, int]:
    """Return a release version's numbers, or raise BuildError for another form."""
    match = RELEASE_VERSION.fullmatch(version)
    if match is None:
        raise BuildError(f'{version!r} is not a MAJOR.MINOR.PATCH release version.')
    major, minor, patch = (int(number) for number in match.groups())
    return major, minor, patch


def check_stable_version(version: str, python_version: str) -> None:
    """Refuse a stable package the 1.1.x updater would never offer.

    The updater installs only a newer MAJOR.MINOR.PATCH version, and a lower
    one would be a downgrade for anyone who installs it by hand.
    """
    if version_numbers(version) <= version_numbers(python_version):
        raise BuildError(
            f'The stable package replaces the Python app {python_version}, so its version must '
            f'be newer, not {version}. Raise [workspace.package] version in native/Cargo.toml '
            'first (native/packaging/README.md, "Moving existing users to the native app").')


def package_identity(channel: Channel, version: str, machine: str) -> PackageIdentity:
    """Return the package's name, version and architecture.

    machine is the Debian architecture the program is built for. The stable
    package says "all" because the 1.1.x updater accepts nothing else; its
    preinst refuses any processor but machine.
    """
    architecture = 'all' if channel is Channel.STABLE else machine
    return PackageIdentity(channel.package, version, architecture)


def build_architecture() -> str:
    """Return the Debian architecture of this computer, which the program is built for."""
    result = subprocess.run(['dpkg', '--print-architecture'], capture_output=True, text=True,
                            check=True)
    return result.stdout.strip()


def build_program(channel: Channel) -> Path:
    """Build the release program with the channel's application ID and return its path."""
    command = ['cargo', 'build', '--release', '--locked', '--manifest-path', str(CARGO_MANIFEST),
               '--package', 'ox-app', '--bin', CARGO_PROGRAM,
               '--message-format=json-render-diagnostics']
    environment = dict(os.environ, OX_APP_ID=channel.app_id)
    # Cargo's progress and errors go to the terminal; its JSON messages name
    # the executable it built.
    result = subprocess.run(command, stdout=subprocess.PIPE, text=True, env=environment)
    if result.returncode != 0:
        raise BuildError('cargo build failed; its output is above.')
    for line in result.stdout.splitlines():
        message = json.loads(line)
        is_program = message.get('target', {}).get('name') == CARGO_PROGRAM
        if message.get('reason') == 'compiler-artifact' and is_program:
            return Path(message['executable'])
    raise BuildError(f'cargo build did not report the {CARGO_PROGRAM} executable.')


def shared_library_depends(program: Path) -> list[str]:
    """Return the packages of the libraries the program links, as dpkg-shlibdeps names them."""
    with tempfile.TemporaryDirectory(prefix='openxplorer-shlibdeps-') as temporary:
        control = Path(temporary) / 'debian' / 'control'
        control.parent.mkdir()
        control.write_text('Source: openxplorer\n\nPackage: openxplorer\nArchitecture: any\n',
                           encoding='utf-8')
        result = subprocess.run(['dpkg-shlibdeps', '-O', f'-e{program}'], cwd=temporary,
                                capture_output=True, text=True, check=True)
    substitution = result.stdout.strip().removeprefix('shlibs:Depends=')
    return [dependency.strip() for dependency in substitution.split(',') if dependency.strip()]


def depends_field(library_packages: list[str]) -> str:
    """Return Depends: the linked libraries with GTK raised to 4.14, then DEPENDS."""
    libraries = [package for package in library_packages
                 if package.split(' ', 1)[0] != GTK_PACKAGE]
    return ', '.join([*libraries, MINIMUM_GTK, *DEPENDS])


def control_text(identity: PackageIdentity, channel: Channel, depends: str,
                 installed_kib: int) -> str:
    """Return DEBIAN/control for the package."""
    recommends = RECOMMENDS + (STABLE_RECOMMENDS if channel is Channel.STABLE else ())
    fields = {
        'Package': identity.package,
        'Version': identity.version,
        'Architecture': identity.architecture,
        'Maintainer': MAINTAINER,
        'Installed-Size': str(installed_kib),
        'Depends': depends,
        'Recommends': ', '.join(recommends),
        **(STABLE_RELATIONS if channel is Channel.STABLE else {}),
        'Section': 'utils',
        'Priority': 'optional',
        'Homepage': HOMEPAGE,
        'Description': DESCRIPTIONS[channel],
    }
    return ''.join(f'{name}: {value}\n' for name, value in fields.items())


def install_debian_files(stage: Path, channel: Channel) -> None:
    """Install the Debian copyright file and the one-application AppStream catalog.

    The catalog maps the package name to the application, so GNOME Software
    shows the installed .deb as OpenXplorer rather than as a bare package, as
    the Python package's does.
    """
    documentation = stage / 'usr/share/doc' / channel.package
    shutil.copyfile(DEBIAN_DATA / 'copyright', documentation / 'copyright')
    (documentation / 'copyright').chmod(package_data.DATA_MODE)
    metainfo = stage / 'usr/share/metainfo' / f'{channel.app_id}.metainfo.xml'
    component = ElementTree.parse(metainfo).getroot()
    ElementTree.SubElement(component, 'pkgname').text = channel.package
    catalog = ElementTree.Element('components', {'version': '0.15', 'origin': 'openxplorer.app'})
    catalog.append(component)
    ElementTree.indent(catalog, space='  ')
    target = stage / 'usr/share/swcatalog/xml' / f'{channel.package}.xml'
    target.parent.mkdir(parents=True)
    ElementTree.ElementTree(catalog).write(target, encoding='utf-8', xml_declaration=True)
    target.chmod(package_data.DATA_MODE)


def write_maintainer_scripts(control: Path, channel: Channel, architecture: str) -> None:
    """Write postinst and postrm, which refresh caches, and the stable preinst.

    The preinst refuses a processor the program was not built for; see
    native/packaging/debian/preinst.in.
    """
    for name in MAINTAINER_SCRIPTS_REFRESHING_CACHES:
        script = control / name
        shutil.copyfile(DEBIAN_DATA / 'refresh-caches', script)
        script.chmod(package_data.PROGRAM_MODE)
    if channel is not Channel.STABLE:
        return
    template = (DEBIAN_DATA / 'preinst.in').read_text(encoding='utf-8')
    preinst = control / 'preinst'
    preinst.write_text(template.replace('@ARCHITECTURE@', architecture), encoding='utf-8')
    preinst.chmod(package_data.PROGRAM_MODE)


def payload_files(stage: Path) -> list[Path]:
    """Return the regular files the package installs, sorted, without DEBIAN/."""
    control = stage / 'DEBIAN'
    return sorted(path for path in stage.rglob('*')
                  if path.is_file() and not path.is_symlink() and control not in path.parents)


def installed_size_kib(stage: Path) -> int:
    """Return Installed-Size: the payload's size in KiB, rounded up."""
    total = sum(path.stat().st_size for path in payload_files(stage))
    return -(-total // 1024)


def write_md5sums(stage: Path) -> None:
    """Write DEBIAN/md5sums, which dpkg --verify checks the installed files against."""
    lines = []
    for path in payload_files(stage):
        digest = hashlib.md5(path.read_bytes(), usedforsecurity=False).hexdigest()
        lines.append(f'{digest}  {path.relative_to(stage).as_posix()}\n')
    md5sums = stage / 'DEBIAN' / 'md5sums'
    md5sums.write_text(''.join(lines), encoding='utf-8')
    md5sums.chmod(package_data.DATA_MODE)


def normalise(stage: Path, epoch: int) -> None:
    """Give every directory mode 0755 and every entry the same time.

    Files keep the modes package_data gave them. Directory modes would
    otherwise follow the builder's umask.
    """
    for path in [stage, *stage.rglob('*')]:
        if path.is_dir() and not path.is_symlink():
            path.chmod(0o755)
        os.utime(path, (epoch, epoch), follow_symlinks=False)


def source_date_epoch() -> int:
    """Return SOURCE_DATE_EPOCH, or the time of the last commit when it is unset."""
    value = os.environ.get('SOURCE_DATE_EPOCH')
    if value is None:
        result = subprocess.run(['git', '-C', str(REPOSITORY), 'log', '-1', '--format=%ct'],
                                capture_output=True, text=True)
        value = result.stdout.strip()
    if not value.isdigit():
        raise BuildError('Set SOURCE_DATE_EPOCH to a Unix time, or build from a Git checkout.')
    return int(value)


@dataclass(frozen=True)
class DebianBuild:
    """What to build: the channel, an optional ready program and where to write."""

    channel: Channel
    program: Path | None
    output_directory: Path


def build(request: DebianBuild) -> Path:
    """Build the package and return its path."""
    version = cargo_version()
    if request.channel is Channel.STABLE:
        check_stable_version(version, python_app_version())
    architecture = build_architecture()
    identity = package_identity(request.channel, version, architecture)
    program = request.program or build_program(request.channel)
    epoch = source_date_epoch()
    request.output_directory.mkdir(parents=True, exist_ok=True)
    output = request.output_directory.resolve() / identity.file_name
    with tempfile.TemporaryDirectory(prefix='openxplorer-deb-') as temporary:
        stage = Path(temporary) / 'root'
        install_request = InstallRequest(request.channel, Layout.DEBIAN, program, stage)
        package_data.install(install_request, package_data.linked_crates())
        install_debian_files(stage, request.channel)
        stage_control(stage, request.channel, identity, architecture)
        normalise(stage, epoch)
        environment = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
        subprocess.run(['dpkg-deb', '--root-owner-group', '--uniform-compression', '-Zxz',
                        '--build', str(stage), str(output)], check=True, env=environment)
    return output


def stage_control(stage: Path, channel: Channel, identity: PackageIdentity,
                  architecture: str) -> None:
    """Write DEBIAN/: control, md5sums and the maintainer scripts."""
    control = stage / 'DEBIAN'
    control.mkdir()
    program = package_data.installed_paths(channel, Layout.DEBIAN).program
    libraries = shared_library_depends(stage / program.relative_to('/'))
    text = control_text(identity, channel, depends_field(libraries), installed_size_kib(stage))
    (control / 'control').write_text(text, encoding='utf-8')
    (control / 'control').chmod(package_data.DATA_MODE)
    write_maintainer_scripts(control, channel, architecture)
    write_md5sums(stage)


def missing_tools() -> list[str]:
    """Return the build tools that are not on PATH."""
    tools = ('cargo', 'rustc', 'dpkg', 'dpkg-deb', 'dpkg-shlibdeps')
    return [tool for tool in tools if shutil.which(tool) is None]


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    """Read the command-line options."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--app-id', default=Channel.PREVIEW.value,
                        choices=[channel.value for channel in Channel],
                        help='the application ID to build (default: the preview)')
    parser.add_argument('--program', type=Path,
                        help='package this executable instead of building one; it must have '
                             'been built with OX_APP_ID set to --app-id')
    parser.add_argument('--output-directory', type=Path, default=OUTPUT_DIRECTORY,
                        help='where to write the .deb (default: dist/native)')
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    """Build the package, print its path and return the exit status."""
    arguments = parse_arguments(argv)
    missing = missing_tools()
    if missing:
        print(f'Building the Debian package needs {", ".join(missing)}. On Debian and Ubuntu: '
              'sudo apt install dpkg-dev, and Rust from rustup.rs.', file=sys.stderr)
        return 2
    request = DebianBuild(Channel(arguments.app_id), arguments.program,
                          arguments.output_directory)
    try:
        print(build(request))
    except (BuildError, OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'Building the Debian package failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
