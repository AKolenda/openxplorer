#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Prove that the native Debian packages replace, sit beside and roll back to the Python one.

Each scenario installs real packages with dpkg into a disposable root (dpkg
--root) inside a bubblewrap sandbox that sees this computer read-only, so
neither dpkg nor a maintainer script can change it:

- upgrade: the Python package, then the stable native package, as the 1.1.x
  updater does. dpkg must record the native version, remove every Python
  file the native package does not install, and leave /usr/bin/openxplorer,
  /usr/bin/winspace (the Exec line of the opt-in "Show in folder" files) and
  the mount helper running the native program and helper, with the desktop
  entry's keys unchanged.
- rollback: the Python package installed again over the native one restores
  the Python app completely.
- coexistence: the preview installs beside the Python package without
  touching a file of it.

Run it before publishing a stable release; native/packaging/README.md,
"Moving existing users to the native app", describes the release steps.
"""
from __future__ import annotations

import argparse
import configparser
from dataclasses import dataclass
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from verify_layout import Report, VerificationError, key_file

PACKAGE = 'openxplorer'
PREVIEW_PACKAGE = 'openxplorer-native'
DESKTOP_ENTRY = 'usr/share/applications/io.winspace.Development.desktop'
NATIVE_PROGRAM = 'opt/openxplorer/bin/openxplorer'
NATIVE_MOUNT_HELPER = 'opt/openxplorer/bin/openxplorer-mount-share'
# Files the maintainer scripts generate, which no package lists.
GENERATED_FILES = ('usr/share/applications/mimeinfo.cache',
                   'usr/share/icons/hicolor/icon-theme.cache')
# dpkg's own records.
DPKG_DATABASE = 'var/lib/dpkg'


@dataclass(frozen=True)
class DpkgRoot:
    """A disposable folder dpkg installs into as if it were '/'."""

    path: Path

    def prepare(self) -> None:
        """Create the empty package database dpkg --root expects."""
        database = self.path / DPKG_DATABASE
        for folder in ('info', 'updates'):
            (database / folder).mkdir(parents=True)
        for name in ('status', 'available'):
            (database / name).touch()

    def install(self, package: Path) -> None:
        """Install a package as the updater's apt-get would, sandboxed and without root.

        Dependencies are not installed into the root, so they are not checked;
        maintainer scripts run without chroot and see $DPKG_ROOT.
        """
        dpkg = ['dpkg', f'--root={self.path}', '--force-not-root', '--force-bad-path',
                '--force-depends', '--force-script-chrootless', '--log=/dev/null',
                '--install', str(package)]
        # dpkg warns about every dependency it skips; show its output only
        # when it fails.
        result = subprocess.run([*sandbox_command(self.path, package), *dpkg],
                                capture_output=True, text=True)
        if result.returncode != 0:
            raise VerificationError(f'dpkg could not install {package.name}:\n{result.stderr}')

    def status(self, package: str) -> str:
        """Return dpkg's status and version of package, as "install ok installed 2.0.0"."""
        query = ['dpkg-query', f'--root={self.path}', '-W', '-f=${Status} ${Version}', package]
        return subprocess.run(query, capture_output=True, text=True, check=True).stdout

    def listed_files(self, package: str) -> set[str]:
        """Return the regular files and links dpkg recorded for package, relative to the root."""
        query = ['dpkg-query', f'--root={self.path}', '-L', package]
        output = subprocess.run(query, capture_output=True, text=True, check=True).stdout
        paths = (self.path / line.lstrip('/') for line in output.splitlines() if line.strip())
        return {path.relative_to(self.path).as_posix() for path in paths
                if path.is_symlink() or path.is_file()}

    def installed_files(self) -> set[str]:
        """Return every regular file and link in the root outside dpkg's database."""
        files = set()
        for path in self.path.rglob('*'):
            relative = path.relative_to(self.path).as_posix()
            is_entry = path.is_symlink() or path.is_file()
            if is_entry and not relative.startswith(DPKG_DATABASE):
                files.add(relative)
        return files - set(GENERATED_FILES)


def sandbox_command(writable: Path, package: Path) -> list[str]:
    """Return the bubblewrap prefix: this computer read-only, only writable writable.

    HOME and the temporary folder are empty apart from writable and package,
    and the desktop session is out of reach, so a script that refreshes a
    user cache changes nothing real.
    """
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--bind', str(writable), str(writable),
            '--ro-bind', str(package), str(package),
            '--setenv', 'HOME', '/tmp', '--unsetenv', 'DBUS_SESSION_BUS_ADDRESS',
            '--unsetenv', 'DISPLAY', '--unsetenv', 'WAYLAND_DISPLAY',
            # dpkg gives every file to root, which in the sandbox's own user
            # namespace is this user.
            '--unshare-all', '--uid', '0', '--gid', '0', '--die-with-parent']


@dataclass(frozen=True)
class Packages:
    """The packages the scenarios install."""

    python: Path
    stable: Path
    preview: Path | None


def version_of(package: Path) -> str:
    """Return the Version field of a .deb."""
    result = subprocess.run(['dpkg-deb', '-f', str(package), 'Version'], capture_output=True,
                            text=True, check=True)
    return result.stdout.strip()


def desktop_entry_groups(path: Path) -> dict[str, dict[str, str]]:
    """Return every group of a desktop entry with its keys."""
    parser: configparser.ConfigParser = key_file(path)
    return {name: dict(parser[name]) for name in parser.sections()}


def extracted(package: Path, folder: Path) -> Path:
    """Unpack a package's files into folder and return it."""
    subprocess.run(['dpkg-deb', '--extract', str(package), str(folder)], check=True)
    return folder


def check_upgrade(report: Report, packages: Packages, work: Path) -> None:
    """Install the Python package, then the stable native one, and check the result."""
    root = DpkgRoot(work / 'upgrade')
    root.prepare()
    root.install(packages.python)
    python_entry = desktop_entry_groups(root.path / DESKTOP_ENTRY)
    root.install(packages.stable)
    report.check('dpkg records the native version as installed',
                 root.status(PACKAGE) == f'install ok installed {version_of(packages.stable)}')
    leftovers = root.installed_files() - root.listed_files(PACKAGE)
    report.check('No file of the Python app is left behind', not leftovers)
    program = (root.path / NATIVE_PROGRAM).resolve()
    commands = ('usr/bin/openxplorer', 'usr/bin/winspace')
    report.check('openxplorer and winspace now run the native program',
                 all((root.path / command).resolve() == program for command in commands))
    helper = (root.path / 'usr/bin/openxplorer-mount-share').resolve()
    report.check('The mount helper command runs the native helper',
                 helper == (root.path / NATIVE_MOUNT_HELPER).resolve())
    report.check("The desktop entry keeps the Python app's keys, MIME types and actions",
                 desktop_entry_groups(root.path / DESKTOP_ENTRY) == python_entry)


def check_rollback(report: Report, packages: Packages, work: Path) -> None:
    """Install the Python package over the native one and check it is complete again."""
    root = DpkgRoot(work / 'rollback')
    root.prepare()
    root.install(packages.python)
    root.install(packages.stable)
    root.install(packages.python)
    python_files = extracted(packages.python, work / 'python-files')
    expected = {path.relative_to(python_files).as_posix()
                for path in python_files.rglob('*') if path.is_file() or path.is_symlink()}
    report.check('Rolling back records the Python version as installed',
                 root.status(PACKAGE) == f'install ok installed {version_of(packages.python)}')
    report.check('Rolling back restores exactly the Python files',
                 root.installed_files() == expected)


def check_coexistence(report: Report, packages: Packages, work: Path) -> None:
    """Install the preview beside the Python package and check both stay whole."""
    if packages.preview is None:
        return
    root = DpkgRoot(work / 'coexistence')
    root.prepare()
    root.install(packages.python)
    python_files = root.listed_files(PACKAGE)
    root.install(packages.preview)
    both = root.listed_files(PACKAGE) | root.listed_files(PREVIEW_PACKAGE)
    report.check('The preview installs beside the Python package',
                 root.status(PACKAGE).startswith('install ok installed')
                 and root.status(PREVIEW_PACKAGE).startswith('install ok installed'))
    report.check('The preview shares no file with the Python package',
                 not python_files & root.listed_files(PREVIEW_PACKAGE)
                 and root.installed_files() == both)


def verify(packages: Packages) -> list[str]:
    """Run every scenario and return the names of the passed checks."""
    report = Report()
    with tempfile.TemporaryDirectory(prefix='openxplorer-upgrade-') as temporary:
        work = Path(temporary)
        check_upgrade(report, packages, work)
        check_rollback(report, packages, work)
        check_coexistence(report, packages, work)
    return report.passed


def main(argv: list[str] | None = None) -> int:
    """Run the scenarios, print the passed checks and return the exit status."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--python', required=True, type=Path,
                        help='the Python package, openxplorer_<version>_all.deb')
    parser.add_argument('--stable', required=True, type=Path,
                        help='the stable native package, openxplorer_<version>_all.deb')
    parser.add_argument('--preview', type=Path,
                        help='the preview package, openxplorer-native_<version>_<arch>.deb')
    arguments = parser.parse_args(argv)
    missing = [tool for tool in ('bwrap', 'dpkg', 'dpkg-deb') if shutil.which(tool) is None]
    if missing:
        print(f'Verifying the upgrade needs {", ".join(missing)} (bubblewrap, dpkg).',
              file=sys.stderr)
        return 2
    packages = Packages(arguments.python.resolve(), arguments.stable.resolve(),
                        arguments.preview.resolve() if arguments.preview else None)
    try:
        passed = verify(packages)
    except (VerificationError, OSError, subprocess.CalledProcessError) as error:
        print(f'Upgrade verification failed: {error}', file=sys.stderr)
        return 1
    print('\n'.join(passed))
    return 0


if __name__ == '__main__':
    sys.exit(main())
