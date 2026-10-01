#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build and verify the local release artifacts, without installing anything.

The release is the native app (native/): the stable Debian package
openxplorer_<version>_all.deb, which the in-app updater of OpenXplorer 1.1.x
downloads and installs, any RPM, Arch package and Flatpak bundle built for
the release, the corresponding-source archive and their SHA256SUMS, all in
dist/. The Python app of 1.x is retired (tag v1.1.4) and no longer shipped.

The Debian package is built here unless --packages names a folder that
already holds it (the continuous-integration build); --flatpak also builds
the Flatpak bundle here. Every package is the stable channel, application ID
io.winspace.Development; preview packages are never picked up.

The source archive holds every editable input, including the scripts that
regenerate generated files, and leaves out itself, binaries and generated
HTML. Website downloads are removed on purpose: releases are published on
GitHub, not on the website host. Nothing here changes desktop settings.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterable
from fnmatch import fnmatchcase
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tomllib
from typing import NoReturn
import zipfile

ROOT = Path(__file__).resolve().parents[1]
NATIVE = ROOT / 'native'
NATIVE_TOOLS = NATIVE / 'tools'
CARGO_MANIFEST = NATIVE / 'Cargo.toml'
DIST = ROOT / 'dist'
TEST_RESULTS = ROOT / 'test-results'

# The source-archive policy below must agree with .gitignore;
# tests/test_release_source.py checks that it does.

# Directory names that are never release inputs, at any depth: dependencies,
# version-control data, caches, and build and test output.
OMITTED_DIRECTORIES = frozenset({
    'node_modules', '.next', 'out', '.git', '.hg', '.svn', '__pycache__',
    'test-results', 'dist', 'designs', '.pnpm-store', '.wrangler', '.vercel',
    '.venv', 'venv', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.cache',
    '.tox', '.nox', '.hypothesis', '.nyc_output', 'htmlcov', 'coverage',
    'playwright-report', 'blob-report', 'tmp', '.tmp', 'temp', '.temp',
})
# File-name patterns, matched case-insensitively: secrets, local databases and
# logs, private-review identifiers, built packages and editor backups.
OMITTED_NAMES = (
    '.env*', '.dev.vars*', '.private-demo-terms*', 'private-terms.json',
    '.ds_store', '.coverage*', '*.py[cod]', '*.tsbuildinfo', '*.deb', '*.zip',
    '*.log', '*.log.*', '*.sqlite', '*.sqlite-*', '*.sqlite3', '*.sqlite3-*',
    '*.db', '*.db-*', '*.pem', '*.key', '*.p12', '*.pfx', '*.credentials',
    'credentials.json', 'credentials.toml', 'credentials.yaml', 'credentials.yml',
    'client_secret*.json', 'service-account*.json', '*.swp', '*.swo', '*.tmp',
    '*.temp', '*~',
)
# Example files with public placeholders. They match '.env*' or '.dev.vars*'
# but are published; only these exact, case-sensitive names are exempt.
SAFE_EXAMPLE_NAMES = frozenset({'.env.example', '.dev.vars.example'})
# Files the release regenerates from sources that the archive does include.
GENERATED_SOURCE_PATHS = frozenset({
    'apps/web/public/assets/site.js', 'apps/web/public/tour/scenes.js',
})

# The application ID of the released app: the Python app's, which the native
# app takes over (native/packaging/README.md).
STABLE_APP_ID = 'io.winspace.Development'
FLATPAK_MANIFEST = NATIVE / 'packaging/flatpak' / f'{STABLE_APP_ID}.yml'
FLATPAK_BUNDLE = f'{STABLE_APP_ID}.flatpak'
FLATPAK_REMOTE = 'https://dl.flathub.org/repo/flathub.flatpakrepo'
# flatpak-builder's build folder and cache. It must lie outside the repository,
# so neither the source archive nor the public-data audit reads it, and outside
# /tmp, which the org.flatpak.Builder Flatpak does not share with the host.
FLATPAK_WORK = Path(os.environ.get('XDG_CACHE_HOME') or Path.home() / '.cache') / \
    'openxplorer-flatpak-build'
# The file names of the release's packages besides the Debian one, as the
# RPM spec, the PKGBUILD and the Flatpak manifest name them for the version
# in {version}: the Fedora and openSUSE RPMs, which differ in their
# distribution tag (.fc44, .opensuse_tumbleweed), the Arch package and the
# Flatpak bundle. The preview's names (openxplorer-native...,
# ...Development.Native.flatpak) and packages of another version never match.
EXTRA_PACKAGE_PATTERNS = ('openxplorer-{version}-*.rpm', 'openxplorer-{version}-*.pkg.tar.zst',
                          FLATPAK_BUNDLE)
# Suffixes of the artifacts an earlier build left in dist/.
ARTIFACT_SUFFIXES = ('.zip', '.deb', '.rpm', '.zst', '.flatpak')

# Download folders in the website trees, removed by every release.
WEBSITE_DOWNLOADS = (
    ROOT / 'apps/web/public/downloads',
    ROOT / 'designs/downloads',
    ROOT / 'apps/web/out/downloads',
)
# Every archive entry gets the same timestamp and mode, so the same tree always
# produces the same archive.
ARCHIVE_PREFIX = 'openxplorer/'
ARCHIVE_TIMESTAMP = (2026, 9, 6, 0, 0, 0)
ARCHIVE_FILE_MODE = 0o100644  # A regular file, rw-r--r--.


def source_path_excluded(relative: Path, *, directory: bool = False) -> bool:
    """Return whether a repository-relative path stays out of the source archive.

    The policy does not depend on Git being installed or initialised.
    source_files() additionally skips links and special files instead of
    following or reading them. The safe example names are intentional
    exceptions, not a content audit.
    """
    parts = tuple(part.casefold() for part in relative.parts)
    if parts[:2] == ('native', 'target'):  # Rust build output.
        return True
    if any(part in OMITTED_DIRECTORIES for part in parts):
        return True
    # The website's download folder. The other two WEBSITE_DOWNLOADS folders
    # lie inside the omitted designs/ and out/ directories.
    if parts[:4] == ('apps', 'web', 'public', 'downloads'):
        return True
    if '/'.join(parts) in GENERATED_SOURCE_PATHS:
        return True
    if not directory and relative.name in SAFE_EXAMPLE_NAMES:
        return False
    name = relative.name.casefold()
    return any(fnmatchcase(name, pattern) for pattern in OMITTED_NAMES)


def sha256_hex(path: Path) -> str:
    """Return the SHA-256 digest of a file as hexadecimal text."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def raise_walk_error(error: OSError) -> NoReturn:
    """Fail on a directory that cannot be read.

    Skipping it would silently produce incomplete corresponding source.
    """
    raise error


def is_source_directory(path: Path, root: Path) -> bool:
    """Return whether source_files() descends into this directory; links are never followed."""
    excluded = source_path_excluded(path.relative_to(root), directory=True)
    return not path.is_symlink() and not excluded


def is_source_file(path: Path, root: Path) -> bool:
    """Return whether this file is an editable input; links and special files are not."""
    excluded = source_path_excluded(path.relative_to(root))
    return not excluded and not path.is_symlink() and path.is_file()


def source_files(root: Path = ROOT) -> list[tuple[Path, Path]]:
    """Return every editable input under root as (path, relative path), in archive order.

    Excluded directories are pruned before they are read, and an unreadable
    directory raises its error instead of being skipped.
    """
    root = Path(root).resolve()
    files = []
    walk = os.walk(root, topdown=True, onerror=raise_walk_error, followlinks=False)
    for directory, subdirectories, names in walk:
        current = Path(directory)
        # Replacing the list in place is how os.walk() is told what to enter.
        subdirectories[:] = sorted(name for name in subdirectories
                                   if is_source_directory(current / name, root))
        for name in names:
            path = current / name
            if is_source_file(path, root):
                files.append((path, path.relative_to(root)))
    return sorted(files, key=lambda item: item[1].as_posix())


def release_version() -> str:
    """Return the release version: [workspace.package] version in native/Cargo.toml.

    The Debian, RPM and Arch packages and the metainfo carry the same version;
    the tests in native/tools check that they agree.
    """
    manifest = tomllib.loads(CARGO_MANIFEST.read_text(encoding='utf-8'))
    version: str = manifest['workspace']['package']['version']
    return version


def run_python(script: Path, *arguments: str) -> None:
    """Run a repository script with this interpreter and raise if it fails."""
    subprocess.run([sys.executable, str(script), *arguments], check=True)


def remove_website_downloads() -> None:
    """Delete the website download folders; releases are published on GitHub."""
    for directory in WEBSITE_DOWNLOADS:
        if directory.is_dir():
            shutil.rmtree(directory)


def is_previous_artifact(path: Path) -> bool:
    """Return whether path is a package, archive or checksum list of an earlier build."""
    return path.is_file() and (path.suffix in ARTIFACT_SUFFIXES or path.name == 'SHA256SUMS')


def prepare_output_directories() -> None:
    """Create the output directories and empty them of earlier artifacts."""
    DIST.mkdir(parents=True, exist_ok=True)
    for path in DIST.iterdir():
        if is_previous_artifact(path):
            path.unlink()
    TEST_RESULTS.mkdir(exist_ok=True)  # For the package verification report.


def debian_package_name(version: str) -> str:
    """Return the only installer name the 1.1.x updater accepts (v2.0.0:desktop/updater.py)."""
    return f'openxplorer_{version}_all.deb'


def obtain_debian_package(version: str, packages: Path | None) -> Path:
    """Copy the stable .deb from packages, or build it, into dist/ and return its path."""
    name = debian_package_name(version)
    if packages is None:
        run_python(NATIVE_TOOLS / 'build_deb.py', '--app-id', STABLE_APP_ID,
                   '--output-directory', str(DIST))
        return DIST / name
    built = packages / name
    if not built.is_file():
        raise FileNotFoundError(f'{built} is missing; build it with native/packaging/ci/'
                                f'build-package.sh deb {STABLE_APP_ID}')
    shutil.copyfile(built, DIST / name)
    return DIST / name


def verify_debian_package(package: Path) -> None:
    """Verify the package, including the 1.1.x updater's own checks, and save the report."""
    report = TEST_RESULTS / 'package-verification.json'
    result = subprocess.run([sys.executable, str(NATIVE_TOOLS / 'verify_deb.py'), str(package)],
                            stdout=subprocess.PIPE, text=True, check=True)
    report.write_text(result.stdout, encoding='utf-8')


def copy_extra_packages(version: str, packages: Path | None) -> list[Path]:
    """Copy the release's RPMs, Arch packages and Flatpak bundle from packages into dist/."""
    if packages is None:
        return []
    patterns = [pattern.format(version=version) for pattern in EXTRA_PACKAGE_PATTERNS]
    copied = []
    for path in sorted(packages.iterdir()):
        wanted = any(fnmatchcase(path.name, pattern) for pattern in patterns)
        if wanted and path.is_file():
            shutil.copyfile(path, DIST / path.name)
            copied.append(DIST / path.name)
    return copied


def flatpak_builder() -> list[str]:
    """Return the flatpak-builder command: the host's, or the org.flatpak.Builder Flatpak."""
    if shutil.which('flatpak-builder'):
        return ['flatpak-builder']
    return ['flatpak', 'run', 'org.flatpak.Builder']


def build_flatpak_bundle(work: Path) -> Path:
    """Build the stable Flatpak offline in work and export its bundle into dist/.

    The build needs the GNOME 51 SDK and the rust-stable extension installed
    for the user (native/packaging/README.md, "Flatpak"). work keeps
    flatpak-builder's cache between runs (see FLATPAK_WORK).
    """
    work.mkdir(parents=True, exist_ok=True)
    repository = work / 'repo'
    # rofiles-fuse only protects the cache from build commands that modify
    # hard-linked files; the org.flatpak.Builder Flatpak cannot mount it.
    subprocess.run([*flatpak_builder(), '--user', '--force-clean', '--disable-rofiles-fuse',
                    f'--state-dir={work / "state"}', f'--repo={repository}',
                    str(work / 'build'), str(FLATPAK_MANIFEST)], check=True)
    bundle = DIST / FLATPAK_BUNDLE
    subprocess.run(['flatpak', 'build-bundle', f'--runtime-repo={FLATPAK_REMOTE}',
                    str(repository), str(bundle), STABLE_APP_ID], check=True)
    return bundle


def write_source_archive(archive: Path, root: Path = ROOT) -> None:
    """Write every editable input under root to a reproducible ZIP archive.

    An entry written from a ZipInfo ignores the archive's compression method
    and level, so each entry sets ZIP_DEFLATED itself and is compressed at
    zlib's default level.
    """
    with zipfile.ZipFile(archive, 'w') as bundle:
        for path, relative in source_files(root):
            entry = zipfile.ZipInfo(ARCHIVE_PREFIX + relative.as_posix(), ARCHIVE_TIMESTAMP)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = ARCHIVE_FILE_MODE << 16
            bundle.writestr(entry, path.read_bytes())


def write_checksums(artifacts: Iterable[Path], destination: Path) -> None:
    """Write the SHA-256 of each artifact in the format `sha256sum --check` reads."""
    lines = [f'{sha256_hex(path)}  {path.name}\n' for path in artifacts]
    destination.write_text(''.join(lines))


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    """Read the options; a mistyped option stops before anything is built."""
    parser = argparse.ArgumentParser(
        description='Build and verify the native Debian package, gather the other native '
                    'packages, and write the corresponding-source archive and SHA256SUMS in '
                    'dist/.')
    parser.add_argument('--packages', type=Path,
                        help='a folder holding the release packages built elsewhere (CI): '
                             'openxplorer_<version>_all.deb and any RPM, Arch package and '
                             f'{FLATPAK_BUNDLE}; without it the .deb is built here')
    parser.add_argument('--flatpak', action='store_true',
                        help=f'also build {FLATPAK_BUNDLE} here with flatpak-builder')
    parser.add_argument('--flatpak-work', type=Path,
                        default=FLATPAK_WORK,
                        help="flatpak-builder's build folder and cache, outside the "
                             'repository and /tmp (default: %(default)s)')
    return parser.parse_args(argv)


def build_release(arguments: argparse.Namespace) -> None:
    """Build the installers, the corresponding source and their checksums."""
    packages = arguments.packages.resolve() if arguments.packages else None
    version = release_version()
    remove_website_downloads()
    prepare_output_directories()
    package = obtain_debian_package(version, packages)
    verify_debian_package(package)
    artifacts = [package, *copy_extra_packages(version, packages)]
    if arguments.flatpak:
        artifacts = [path for path in artifacts if path.name != FLATPAK_BUNDLE]
        artifacts.append(build_flatpak_bundle(arguments.flatpak_work))
    source = DIST / f'openxplorer-{version}-source.zip'
    write_source_archive(source)
    write_checksums([*artifacts, source], DIST / 'SHA256SUMS')


def describe_failed_command(error: subprocess.CalledProcessError) -> str:
    """Name a failed command and how it ended, quoted as it would be typed in a shell."""
    command = shlex.join(str(part) for part in error.cmd)
    if error.returncode < 0:
        return f'{command} was killed by signal {-error.returncode}'
    return f'{command} exited with status {error.returncode}'


def main(argv: list[str] | None = None) -> int:
    """Build the release and return the exit status: 0 built, 1 failed.

    A failure ends in one line that says what failed, instead of a traceback.
    """
    arguments = parse_arguments(argv)
    try:
        build_release(arguments)
    except subprocess.CalledProcessError as error:
        print(f'Release failed: {describe_failed_command(error)}; its output is above.',
              file=sys.stderr)
        return 1
    except OSError as error:
        print(f'Release failed: {error}. Fix this and rerun tools/release.py.',
              file=sys.stderr)
        return 1
    print('Built the native packages, corresponding source and checksums in dist/. '
          'Website links remain on GitHub.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
